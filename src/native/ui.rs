use eframe::egui::{self, Color32, FontId, RichText, Stroke, Vec2, ViewportCommand, WindowLevel};
use quickadb::{engine::Backend, model::{Device, DeviceStatus, Endpoint, Job, JobStage, Settings, Snapshot}, platform, storage::Storage};
use std::{sync::Arc, time::{Duration, Instant}};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent, menu::{Menu, MenuEvent, MenuItem}};

const WIDTH: f32 = 408.;
const HEIGHT: f32 = 608.;
const GREEN: Color32 = Color32::from_rgb(37, 173, 115);
const MUTED: Color32 = Color32::from_rgb(133, 144, 154);

pub fn run(storage: Arc<Storage>, backend: Backend) -> eframe::Result {
    let settings = storage.settings();
    let icon = image::load_from_memory(include_bytes!("../../assets/AppIcon.png")).expect("embedded icon invalid").into_rgba8();
    let data = egui::IconData { rgba: icon.as_raw().clone(), width: icon.width(), height: icon.height() };
    let mut viewport = egui::ViewportBuilder::default().with_title("QuickADB").with_inner_size([WIDTH, HEIGHT]).with_decorations(false).with_resizable(false).with_taskbar(false).with_icon(Arc::new(data));
    if settings.topmost { viewport = viewport.with_window_level(WindowLevel::AlwaysOnTop); }
    let area = platform::monitor_work_area();
    let position = settings.window_position.unwrap_or([area.right as f32 - WIDTH - 24., area.bottom as f32 - HEIGHT - 24.]);
    viewport = viewport.with_position(position);
    let options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Glow, ..Default::default() };
    eframe::run_native("QuickADB", options, Box::new(move |cc| {
        configure_fonts(&cc.egui_ctx)?;
        let app = Drawer::new(cc, storage, backend)?;
        Ok(Box::new(app))
    }))
}

fn configure_fonts(ctx: &egui::Context) -> anyhow::Result<()> {
    let directory = std::path::PathBuf::from(std::env::var_os("WINDIR").ok_or_else(|| anyhow::anyhow!("Windows 字体目录不可用"))?).join("Fonts");
    let bytes = std::fs::read(directory.join("msyh.ttc"))?;
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("微软雅黑".into(), Arc::new(egui::FontData::from_owned(bytes)));
    fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "微软雅黑".into());
    ctx.set_fonts(fonts);
    Ok(())
}

#[derive(Clone)]
enum Dialog {
    Connect { mode: u8, host: String, port: String, pairing_port: String, code: String, paired_id: String },
    Settings,
    Devices,
    Detail { device: Device, alias: String },
    Tcp { device: Device, host: String, port: String },
    Exit,
}

struct Drawer {
    backend: Backend,
    storage: Arc<Storage>,
    preferences: Settings,
    _tray: TrayIcon,
    open_menu: MenuItem,
    exit_menu: MenuItem,
    settings_menu: MenuItem,
    dialog: Option<Dialog>,
    icon: egui::TextureHandle,
    hidden: bool,
    collapsed: bool,
    exiting: bool,
    was_focused: bool,
    opened: Instant,
    last_position: Option<[f32; 2]>,
    position_changed: Instant,
}

impl Drawer {
    fn new(cc: &eframe::CreationContext<'_>, storage: Arc<Storage>, backend: Backend) -> anyhow::Result<Self> {
        let preferences = storage.settings();
        let pixels = image::load_from_memory(include_bytes!("../../assets/AppIcon.png"))?.into_rgba8();
        let tray_pixels = image::imageops::resize(&pixels, 32, 32, image::imageops::FilterType::Lanczos3);
        let menu = Menu::new();
        let open_menu = MenuItem::new("打开安装抽屉", true, None);
        let settings_menu = MenuItem::new("设置", true, None);
        let exit_menu = MenuItem::new("退出 QuickADB", true, None);
        menu.append_items(&[&open_menu, &settings_menu, &tray_icon::menu::PredefinedMenuItem::separator(), &exit_menu])?;
        let tray = TrayIconBuilder::new().with_icon(Icon::from_rgba(tray_pixels.as_raw().clone(), 32, 32)?).with_tooltip("QuickADB · 点击安装 APK").with_menu(Box::new(menu)).with_menu_on_left_click(false).build()?;
        let icon = cc.egui_ctx.load_texture("app-icon", egui::ColorImage::from_rgba_unmultiplied([pixels.width() as usize, pixels.height() as usize], pixels.as_raw()), egui::TextureOptions::LINEAR);
        let hidden = std::env::args().any(|a| a == "--tray");
        if hidden { cc.egui_ctx.send_viewport_cmd(ViewportCommand::Visible(false)); }
        let app = Self { backend, storage, preferences, _tray: tray, open_menu, exit_menu, settings_menu, dialog: None, icon, hidden, collapsed: false, exiting: false, was_focused: false, opened: Instant::now(), last_position: None, position_changed: Instant::now() };
        app.apply_style(&cc.egui_ctx);
        cc.egui_ctx.request_repaint_after(Duration::from_millis(100));
        Ok(app)
    }

    fn apply_style(&self, ctx: &egui::Context) {
        let mut style = egui::Style::default();
        style.visuals = if self.preferences.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        style.visuals.selection.bg_fill = GREEN.gamma_multiply(0.28);
        style.visuals.selection.stroke = Stroke::new(1., GREEN);
        style.visuals.panel_fill = if self.preferences.dark { Color32::from_rgb(24, 29, 34) } else { Color32::from_rgb(247, 249, 250) };
        style.visuals.window_fill = if self.preferences.dark { Color32::from_rgb(32, 38, 44) } else { Color32::WHITE };
        style.visuals.widgets.inactive.corner_radius = 7.into();
        style.visuals.widgets.hovered.corner_radius = 7.into();
        style.visuals.widgets.active.corner_radius = 7.into();
        style.spacing.item_spacing = Vec2::new(8., 7.);
        style.spacing.button_padding = Vec2::new(10., 7.);
        style.text_styles.insert(egui::TextStyle::Body, FontId::proportional(13.));
        style.text_styles.insert(egui::TextStyle::Button, FontId::proportional(13.));
        style.text_styles.insert(egui::TextStyle::Small, FontId::proportional(11.));
        style.text_styles.insert(egui::TextStyle::Heading, FontId::proportional(17.));
        ctx.set_global_style(style);
    }

    fn persist(&self) {
        if let Err(error) = self.storage.update_settings(|settings| {
            settings.startup = self.preferences.startup;
            settings.dark = self.preferences.dark;
            settings.pinned = self.preferences.pinned;
            settings.topmost = self.preferences.topmost;
            settings.test_packages = self.preferences.test_packages;
            settings.window_position = self.preferences.window_position;
        }) { self.backend.notify(&format!("设置保存失败：{error:#}")); }
    }

    fn show(&mut self, ctx: &egui::Context, anchor: Option<[f32; 2]>) {
        if let Some([x, y]) = anchor {
            if !self.preferences.pinned {
                let dpi = ctx.input(|i| i.viewport().native_pixels_per_point.unwrap_or(1.));
                let area = platform::monitor_work_area();
                let position = egui::pos2((x / dpi - WIDTH + 22.).clamp(area.left as f32 / dpi, area.right as f32 / dpi - WIDTH), (y / dpi - HEIGHT - 12.).clamp(area.top as f32 / dpi, area.bottom as f32 / dpi - HEIGHT));
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position));
            }
        }
        self.hidden = false;
        self.opened = Instant::now();
        self.was_focused = false;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn hide(&mut self, ctx: &egui::Context) {
        self.hidden = true;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    fn request_exit(&mut self, ctx: &egui::Context) {
        if self.backend.snapshot().jobs.iter().any(|j| j.stage.active()) { self.show(ctx, None); self.dialog = Some(Dialog::Exit); }
        else { self.exiting = true; ctx.send_viewport_cmd(ViewportCommand::Close); }
    }

    fn pick_apks(&self, split: bool) {
        if let Some(paths) = rfd::FileDialog::new().set_title(if split { "选择一组拆分 APK（含基础包）" } else { "选择一个或多个 APK" }).add_filter("Android 安装包", &["apk"]).pick_files() {
            self.backend.submit(paths, split, self.preferences.test_packages);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let response = ui.horizontal(|ui| {
            ui.add(egui::Image::new(&self.icon).fit_to_exact_size(Vec2::splat(36.)));
            ui.vertical(|ui| {
                ui.label(RichText::new("QuickADB").size(17.).strong());
                ui.label(RichText::new("随手安装，随时测试").size(11.).color(MUTED));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("×").frame(false)).on_hover_text("收回托盘，安装继续").clicked() { self.hide(ui.ctx()); }
                if ui.add(egui::Button::new("−").frame(false)).on_hover_text("收成窄栏").clicked() { self.collapsed = true; ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(WIDTH, 64.))); }
                if ui.selectable_label(self.preferences.pinned, "固定").on_hover_text("固定后点击其他窗口时保持展开").clicked() { self.preferences.pinned = !self.preferences.pinned; self.persist(); }
            });
        });
        if ui.interact(response.response.rect, ui.id().with("window-drag"), egui::Sense::drag()).drag_started() { ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag); }
    }

    fn device_rows(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let selected = snapshot.devices.iter().filter(|d| d.selected).count();
        ui.horizontal(|ui| {
            ui.label(RichText::new("安装设备").strong());
            if selected > 0 { ui.label(RichText::new(format!("已选 {selected} 台")).small().color(GREEN)); }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("连接设备").frame(false)).clicked() { self.dialog = Some(connect_dialog()); }
                if ui.add(egui::Button::new(if selected == 0 { "全选" } else { "取消全选" }).frame(false)).clicked() { self.backend.select_all(selected == 0); }
            });
        });
        if snapshot.devices.is_empty() {
            card(ui, self.preferences.dark, |ui| {
                ui.add_space(8.);
                ui.label(RichText::new("等待连接第一台设备").strong());
                ui.label(RichText::new("USB 连接后开启调试并在手机上授权，\n也可使用同一局域网的无线调试。").small().color(MUTED));
                ui.add_space(8.);
            });
        } else {
            egui::ScrollArea::vertical().id_salt("devices").max_height(156.).show(ui, |ui| {
                for device in &snapshot.devices {
                    ui.horizontal(|ui| {
                        let online = device.status == DeviceStatus::Online;
                        let text = format!("{}  {}\n    {} · {}{}", if device.selected { "✓" } else { "○" }, device.name, device.transport, device.status.label(), if device.android.is_empty() { String::new() } else { format!(" · Android {}", device.android) });
                        let button = egui::Button::new(RichText::new(text).size(12.)).min_size(Vec2::new(ui.available_width() - 32., 48.)).fill(if device.selected { GREEN.gamma_multiply(if self.preferences.dark { 0.22 } else { 0.12 }) } else { ui.visuals().window_fill }).stroke(Stroke::new(1., if device.selected { GREEN } else { ui.visuals().widgets.noninteractive.bg_stroke.color }));
                        if ui.add_enabled(online, button).on_hover_text(if online { "点击选择，再次点击取消；可同时选择多台" } else { &device.detail }).clicked() { self.backend.toggle(&device.id); }
                        if ui.add(egui::Button::new("⋯").frame(false)).on_hover_text("设备详情与连接操作").clicked() { self.dialog = Some(Dialog::Detail { device: device.clone(), alias: self.storage.settings().aliases.get(&device.serial).cloned().unwrap_or_default() }); }
                    });
                }
            });
        }
    }

    fn drop_zone(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
        let selected = snapshot.devices.iter().filter(|d| d.selected && d.status == DeviceStatus::Online).count();
        let border = if hovering { GREEN } else { ui.visuals().widgets.noninteractive.bg_stroke.color };
        egui::Frame::new().fill(if hovering { GREEN.gamma_multiply(0.10) } else { ui.visuals().window_fill }).stroke(Stroke::new(1.3, border)).corner_radius(12).inner_margin(18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(38., 40.), egui::Sense::hover());
                let paper = rect.shrink(5.);
                ui.painter().rect(paper, 4., GREEN.gamma_multiply(0.09), Stroke::new(1.5, GREEN), egui::StrokeKind::Inside);
                ui.painter().text(paper.center(), egui::Align2::CENTER_CENTER, "APK", FontId::proportional(10.), GREEN);
                ui.label(RichText::new(if hovering { "松开即可加入安装队列" } else { "把 APK 拖到这里" }).size(16.).strong());
                ui.label(RichText::new(if selected > 0 { format!("将安装到已选的 {selected} 台设备 · 支持批量拖入") } else { "先点击选择设备 · 可单选或多选".into() }).small().color(MUTED));
                ui.add_space(3.);
                ui.horizontal(|ui| {
                    ui.add_space(61.);
                    if ui.button("选择 APK").clicked() { self.pick_apks(false); self.opened = Instant::now(); }
                    if ui.add(egui::Button::new("安装拆分 APK").frame(false)).clicked() { self.pick_apks(true); self.opened = Instant::now(); }
                });
            });
        });
    }

    fn job_row(&mut self, ui: &mut egui::Ui, job: &Job) {
        card(ui, self.preferences.dark, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&job.title).strong().size(12.));
                    ui.label(RichText::new(&job.device_name).small().color(MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if matches!(job.stage, JobStage::Queued | JobStage::Preparing | JobStage::Transferring) {
                        if ui.small_button("取消").clicked() { self.backend.cancel(job.id); }
                    } else if matches!(job.stage, JobStage::Failed | JobStage::Canceled | JobStage::Unknown) && ui.small_button("重试").clicked() { self.backend.retry_job(job.id); }
                });
            });
            if job.stage == JobStage::Transferring {
                let fraction = if job.total == 0 { 0. } else { job.transferred as f32 / job.total as f32 };
                ui.add(egui::ProgressBar::new(fraction).fill(GREEN).desired_height(5.));
                ui.label(RichText::new(format!("传输 {:.0}%  ·  {} / {}  ·  {}/s", fraction * 100., bytes(job.transferred as f64), bytes(job.total as f64), bytes(job.bytes_per_second))).small().color(MUTED));
            } else {
                ui.horizontal(|ui| {
                    if matches!(job.stage, JobStage::Preparing | JobStage::Installing) { ui.add(egui::Spinner::new().size(12.)); }
                    let color = match job.stage { JobStage::Succeeded => GREEN, JobStage::Failed | JobStage::Unknown => Color32::from_rgb(214, 105, 82), _ => MUTED };
                    ui.label(RichText::new(job.stage.label()).small().color(color));
                });
            }
            if !job.detail.is_empty() {
                egui::CollapsingHeader::new("查看结果详情").id_salt(job.id).show(ui, |ui| {
                    ui.label(error_help(&job.detail));
                    ui.label(RichText::new(&job.detail).small());
                    if ui.small_button("复制原始结果").clicked() { ui.ctx().copy_text(job.detail.clone()); }
                });
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context, snapshot: &Snapshot) {
        let Some(mut dialog) = self.dialog.take() else { return; };
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("drawer-dialog")).show(ctx, |ui| {
            ui.set_width(326.);
            match &mut dialog {
                Dialog::Connect { mode, host, port, pairing_port, code, paired_id } => {
                    modal_heading(ui, "连接设备", &mut close);
                    ui.horizontal(|ui| { ui.selectable_value(mode, 0, "新设备配对"); ui.selectable_value(mode, 1, "已配对"); ui.selectable_value(mode, 2, "传统 TCP"); });
                    ui.add_space(6.);
                    if *mode == 0 {
                        ui.label(RichText::new("Android 11+：打开“无线调试 → 使用配对码配对”。配对端口和连接端口不同。").small().color(MUTED));
                    } else if *mode == 1 {
                        let saved = self.storage.settings();
                        egui::ComboBox::from_id_salt("paired-device").selected_text(if paired_id.is_empty() { "选择已配对设备" } else { paired_id.as_str() }).show_ui(ui, |ui| {
                            for endpoint in saved.endpoints.iter().filter(|e| e.paired_id.is_some()) {
                                if ui.selectable_label(endpoint.paired_id.as_ref() == Some(paired_id), format!("{} · {}:{}", endpoint.paired_id.as_deref().unwrap_or(""), endpoint.host, endpoint.port)).clicked() {
                                    *host = endpoint.host.clone(); *port = endpoint.port.to_string(); *paired_id = endpoint.paired_id.clone().unwrap_or_default();
                                }
                            }
                        });
                        ui.label(RichText::new("填入无线调试主页显示的当前连接端口。").small().color(MUTED));
                    } else { ui.label(RichText::new("适用于已启用 TCP 调试的设备，或在设备详情中通过 USB 转无线。").small().color(MUTED)); }
                    field(ui, "手机 IP 地址", host, false);
                    if *mode == 0 { field(ui, "配对端口", pairing_port, false); field(ui, "六位配对码", code, true); }
                    field(ui, "连接端口", port, false);
                    if !snapshot.discovered.is_empty() {
                        egui::CollapsingHeader::new("局域网发现的地址").show(ui, |ui| {
                            for endpoint in &snapshot.discovered {
                                if ui.small_button(format!("{}:{}", endpoint.host, endpoint.port)).clicked() { *host = endpoint.host.clone(); *port = endpoint.port.to_string(); }
                            }
                        });
                    }
                    ui.add_space(8.);
                    let valid = !host.trim().is_empty() && port.parse::<u16>().is_ok_and(|p| p > 0)
                        && (*mode != 0 || (pairing_port.parse::<u16>().is_ok_and(|p| p > 0) && code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())))
                        && (*mode != 1 || !paired_id.is_empty());
                    if snapshot.pairing { ui.horizontal(|ui| { ui.spinner(); ui.label("正在验证配对码…"); }); }
                    if ui.add_enabled(valid && !snapshot.pairing, primary_button(if *mode == 0 { "配对并连接" } else { "连接设备" })).clicked() {
                        let connection_port = port.parse().expect("validated port");
                        if *mode == 0 { self.backend.pair(host.trim().into(), pairing_port.parse().expect("validated pairing port"), code.clone(), connection_port); }
                        else { self.backend.connect(Endpoint { host: host.trim().into(), port: connection_port, paired_id: if *mode == 1 { Some(paired_id.clone()) } else { None } }); }
                        code.clear(); close = true;
                    }
                }
                Dialog::Settings => {
                    modal_heading(ui, "设置", &mut close);
                    ui.label(RichText::new("常驻行为").strong());
                    let before = self.preferences.startup;
                    if ui.checkbox(&mut self.preferences.startup, "开机启动，静默进入托盘").changed() {
                        if let Err(error) = platform::set_startup(self.preferences.startup) { self.preferences.startup = before; self.backend.notify(&format!("开机启动设置失败：{error:#}")); }
                        else { self.persist(); }
                    }
                    if ui.checkbox(&mut self.preferences.pinned, "固定抽屉，点击外部时不收起").changed() { self.persist(); }
                    if ui.checkbox(&mut self.preferences.topmost, "窗口保持置顶").changed() {
                        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(if self.preferences.topmost { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal })); self.persist();
                    }
                    ui.add_space(6.);
                    ui.label(RichText::new("外观与安装").strong());
                    if ui.checkbox(&mut self.preferences.dark, "深色外观").changed() { self.apply_style(ctx); self.persist(); }
                    if ui.checkbox(&mut self.preferences.test_packages, "允许标记为 testOnly 的测试 APK").changed() { self.persist(); }
                    ui.label(RichText::new("默认覆盖更新并保留应用数据。").small().color(MUTED));
                    ui.add_space(8.);
                    ui.label(RichText::new("数据目录").strong());
                    ui.label(RichText::new(self.storage.directory.display().to_string()).small());
                    ui.label(RichText::new("授权密钥由当前 Windows 账号保护。").small().color(MUTED));
                    if ui.small_button("复制数据目录").clicked() { ctx.copy_text(self.storage.directory.display().to_string()); }
                }
                Dialog::Devices => {
                    modal_heading(ui, "设备管理", &mut close);
                    egui::ScrollArea::vertical().max_height(310.).show(ui, |ui| {
                        for device in &snapshot.devices {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| { ui.label(RichText::new(&device.name).strong()); ui.label(RichText::new(format!("{} · {}", device.transport, device.status.label())).small().color(MUTED)); });
                                if device.status != DeviceStatus::Online && device.status != DeviceStatus::Connecting && ui.small_button("重新连接").clicked() { self.backend.retry_connection(device.id.clone()); }
                                if device.endpoint.is_some() && device.status == DeviceStatus::Online && ui.small_button("断开").clicked() { self.backend.disconnect(device.id.clone()); }
                            });
                            ui.separator();
                        }
                        ui.label(RichText::new("无线连接记录").strong());
                        for endpoint in self.storage.settings().endpoints {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}:{}", endpoint.host, endpoint.port));
                                if ui.small_button("连接").clicked() { self.backend.connect(endpoint.clone()); }
                                if ui.small_button("移除记录").clicked() {
                                    if let Err(error) = self.storage.update_settings(|settings| settings.endpoints.retain(|e| e.key() != endpoint.key())) { self.backend.notify(&format!("移除记录失败：{error:#}")); }
                                }
                            });
                        }
                    });
                }
                Dialog::Detail { device, alias } => {
                    modal_heading(ui, "设备详情", &mut close);
                    ui.label(RichText::new(&device.name).size(16.).strong());
                    ui.label(format!("{} · {}", device.transport, device.status.label()));
                    ui.label(format!("序列号：{}", device.serial));
                    ui.label(format!("Android：{}", device.android));
                    if !device.detail.is_empty() { ui.label(&device.detail); }
                    field(ui, "设备备注", alias, false);
                    if ui.add_enabled(!device.serial.is_empty(), egui::Button::new("保存备注")).clicked() {
                        self.backend.set_alias(device, alias);
                        close = true;
                    }
                    if device.status != DeviceStatus::Online && ui.button("重新连接").clicked() { self.backend.retry_connection(device.id.clone()); close = true; }
                    if device.status == DeviceStatus::Online {
                        if device.endpoint.is_some() && ui.button("断开无线连接").clicked() { self.backend.disconnect(device.id.clone()); close = true; }
                        if device.endpoint.is_none() && ui.button("通过 USB 转无线").clicked() { self.dialog = Some(Dialog::Tcp { device: device.clone(), host: String::new(), port: "5555".into() }); close = true; }
                    }
                    if ui.small_button("复制设备标识").clicked() { ctx.copy_text(device.id.clone()); }
                }
                Dialog::Tcp { device, host, port } => {
                    modal_heading(ui, "USB 转无线", &mut close);
                    ui.label(format!("将 {} 的调试服务切换到 TCP。", device.name));
                    ui.label(RichText::new("手机与电脑需处于可信的同一局域网。该操作会重启手机调试服务，USB 连接可能中断。").small().color(MUTED));
                    field(ui, "手机当前局域网 IP", host, false);
                    field(ui, "TCP 调试端口", port, false);
                    if ui.add_enabled(!host.trim().is_empty() && port.parse::<u16>().is_ok_and(|p| p > 0), primary_button("启用并连接")).clicked() { self.backend.enable_tcp(device.id.clone(), host.trim().into(), port.parse().expect("validated port")); close = true; }
                }
                Dialog::Exit => {
                    modal_heading(ui, "退出 QuickADB", &mut close);
                    ui.label("仍有安装任务。退出将中断传输，已提交给 Android 的安装可能继续，结果需要在手机上确认。");
                    ui.add_space(8.);
                    ui.horizontal(|ui| {
                        if ui.button("继续等待").clicked() { close = true; }
                        if ui.button("退出应用").clicked() { self.exiting = true; ctx.send_viewport_cmd(ViewportCommand::Close); close = true; }
                    });
                }
            }
        });
        if !close && !response.should_close() { self.dialog = Some(dialog); }
    }
}

impl eframe::App for Drawer {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, position, .. } = event { self.show(ctx, Some([position.x as f32, position.y as f32])); }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *self.open_menu.id() { self.show(ctx, None); }
            else if event.id == *self.exit_menu.id() { self.request_exit(ctx); }
            else if event.id == *self.settings_menu.id() { self.show(ctx, None); self.dialog = Some(Dialog::Settings); }
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.exiting { ctx.send_viewport_cmd(ViewportCommand::CancelClose); self.hide(ctx); }
        if !self.hidden {
            let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
            if focused { self.was_focused = true; }
            if self.was_focused && !focused && !self.preferences.pinned && self.dialog.is_none() && self.opened.elapsed() > Duration::from_millis(600) { self.hide(ctx); }
            if self.preferences.pinned {
                if let Some(position) = ctx.input(|i| i.viewport().outer_rect.map(|r| [r.min.x, r.min.y])) {
                    if self.last_position != Some(position) { self.last_position = Some(position); self.position_changed = Instant::now(); }
                    else if self.position_changed.elapsed() > Duration::from_millis(700) && self.preferences.window_position != Some(position) { self.preferences.window_position = Some(position); self.persist(); }
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(if self.hidden { 250 } else { 100 }));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.collapsed {
            egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(12).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Image::new(&self.icon).fit_to_exact_size(Vec2::splat(32.)));
                    let snapshot = self.backend.snapshot();
                    let count = snapshot.jobs.iter().filter(|j| j.stage.active()).count();
                    if ui.add(egui::Button::new(if count > 0 { format!("QuickADB · {count} 项安装中") } else { "QuickADB · 展开安装抽屉".into() }).frame(false)).clicked() { self.collapsed = false; ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(WIDTH, HEIGHT))); }
                    if ui.small_button("×").clicked() { self.hide(&ctx); }
                });
            });
            return;
        }
        let snapshot = self.backend.snapshot();
        let files = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect::<Vec<_>>());
        if !files.is_empty() { self.backend.submit(files, false, self.preferences.test_packages); }
        egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(16).show(ui, |ui| {
            self.header(ui);
            ui.add_space(7.);
            ui.separator();
            ui.add_space(5.);
            self.device_rows(ui, &snapshot);
            ui.add_space(6.);
            self.drop_zone(ui, &snapshot);
            ui.add_space(6.);
            ui.horizontal(|ui| {
                ui.label(RichText::new("安装任务").strong());
                let count = snapshot.jobs.iter().filter(|j| j.stage.active()).count();
                if count > 0 { ui.label(RichText::new(format!("{count} 项进行中")).small().color(GREEN)); }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { if ui.add(egui::Button::new("清除已结束").frame(false)).clicked() { self.backend.clear_finished(); } });
            });
            let remaining = (ui.available_height() - if snapshot.notice.is_empty() { 37. } else { 90. }).max(60.);
            egui::ScrollArea::vertical().id_salt("jobs").max_height(remaining).auto_shrink([false, false]).show(ui, |ui| {
                if snapshot.jobs.is_empty() { ui.add_space(18.); ui.vertical_centered(|ui| { ui.label(RichText::new("安装结果会显示在这里").small().color(MUTED)); }); }
                for job in &snapshot.jobs { self.job_row(ui, job); }
            });
            if !snapshot.notice.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&snapshot.notice).small().color(Color32::from_rgb(207, 134, 64)));
                    if ui.small_button("×").clicked() { self.backend.clear_notice(); }
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                let online = snapshot.devices.iter().filter(|d| d.status == DeviceStatus::Online).count();
                ui.label(RichText::new(format!("● {online} 台在线")).small().color(if online > 0 { GREEN } else { MUTED }));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("设置").frame(false)).clicked() { self.dialog = Some(Dialog::Settings); }
                    if ui.add(egui::Button::new("设备管理").frame(false)).clicked() { self.dialog = Some(Dialog::Devices); }
                });
            });
        });
        self.dialogs(&ctx, &snapshot);
    }
}

fn connect_dialog() -> Dialog { Dialog::Connect { mode: 0, host: String::new(), port: String::new(), pairing_port: String::new(), code: String::new(), paired_id: String::new() } }
fn primary_button(text: &str) -> egui::Button<'_> { egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(GREEN).min_size(Vec2::new(128., 34.)) }
fn modal_heading(ui: &mut egui::Ui, title: &str, close: &mut bool) { ui.horizontal(|ui| { ui.heading(title); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { if ui.small_button("×").clicked() { *close = true; } }); }); ui.add_space(8.); }
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, password: bool) { ui.label(RichText::new(label).small().color(MUTED)); ui.add(egui::TextEdit::singleline(value).password(password).desired_width(f32::INFINITY)); }
fn card(ui: &mut egui::Ui, dark: bool, content: impl FnOnce(&mut egui::Ui)) { egui::Frame::new().fill(if dark { Color32::from_rgb(32, 38, 44) } else { Color32::WHITE }).corner_radius(9).stroke(Stroke::new(1., ui.visuals().widgets.noninteractive.bg_stroke.color.gamma_multiply(0.6))).inner_margin(10).show(ui, |ui| { ui.set_width(ui.available_width()); content(ui); }); }
fn bytes(value: f64) -> String { if value >= 1024. * 1024. { format!("{:.1} MB", value / (1024. * 1024.)) } else { format!("{:.0} KB", value / 1024.) } }
fn error_help(error: &str) -> &'static str {
    if error.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") { "签名与已安装应用不同，需要确认应用来源；不会自动卸载旧应用。" }
    else if error.contains("INSTALL_FAILED_VERSION_DOWNGRADE") { "APK 版本低于设备上已有版本。" }
    else if error.contains("INSTALL_FAILED_INSUFFICIENT_STORAGE") { "手机存储空间不足。" }
    else if error.contains("INSTALL_FAILED_TEST_ONLY") { "此 APK 为测试包，可在设置中允许 testOnly APK 后重试。" }
    else if error.contains("INSTALL_FAILED_NO_MATCHING_ABIS") { "APK 不支持这台设备的 CPU 架构。" }
    else if error.contains("INSTALL_FAILED_OLDER_SDK") { "手机 Android 版本低于 APK 要求。" }
    else { "以下为设备返回的原始结果：" }
}
