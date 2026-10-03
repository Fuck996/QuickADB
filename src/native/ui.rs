use droidmux::discovery::AdbServiceType;
use eframe::egui::{self, Color32, FontId, RichText, Stroke, Vec2, ViewportCommand, WindowLevel};
use quickadb::{
    engine::Backend,
    model::{Device, DeviceStatus, Endpoint, Job, JobStage, PreparationState, Settings, Snapshot},
    platform,
    storage::Storage,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};
use winit::platform::windows::WindowExtWindows;

const WIDTH: f32 = 440.;
const HEIGHT: f32 = 780.;
const WINDOW_SIZE: [f32; 2] = [1080., 800.];
const GREEN: Color32 = Color32::from_rgb(37, 173, 115);
const PRIMARY_GREEN: Color32 = Color32::from_rgb(20, 133, 84);

pub fn run(storage: Arc<Storage>, backend: Backend) -> eframe::Result {
    let settings = storage.settings();
    let icon = image::load_from_memory(include_bytes!("../../assets/AppIconDisplay.png"))
        .expect("embedded icon invalid")
        .into_rgba8();
    let data = egui::IconData {
        rgba: icon.as_raw().clone(),
        width: icon.width(),
        height: icon.height(),
    };
    let start_in_tray = std::env::args().any(|a| a == "--tray");
    let size = if settings.windowed {
        settings.standard_window_size.unwrap_or(WINDOW_SIZE)
    } else {
        [WIDTH, HEIGHT]
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("QuickADB")
        .with_inner_size(size)
        .with_min_inner_size([WIDTH, if settings.windowed { 480. } else { 64. }])
        .with_decorations(settings.windowed)
        .with_resizable(settings.windowed)
        .with_drag_and_drop(true)
        .with_taskbar(settings.windowed)
        .with_window_level(window_level(&settings))
        .with_visible(!start_in_tray)
        .with_active(!start_in_tray)
        .with_icon(Arc::new(data));
    let area = platform::monitor_work_area();
    let position = if settings.windowed {
        settings.standard_window_position
    } else {
        settings.window_position
    }
    .unwrap_or([
        area.right as f32 - size[0] - 24.,
        area.bottom as f32 - size[1] - 24.,
    ]);
    viewport = viewport.with_position(position);
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "QuickADB",
        options,
        Box::new(move |cc| {
            configure_fonts(&cc.egui_ctx)?;
            let app = Drawer::new(cc, storage, backend)?;
            Ok(Box::new(app))
        }),
    )
}

fn configure_fonts(ctx: &egui::Context) -> anyhow::Result<()> {
    let directory = std::path::PathBuf::from(
        std::env::var_os("WINDIR").ok_or_else(|| anyhow::anyhow!("Windows 字体目录不可用"))?,
    )
    .join("Fonts");
    let bytes = std::fs::read(directory.join("msyh.ttc"))?;
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "微软雅黑".into(),
        Arc::new(egui::FontData::from_owned(bytes)),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "微软雅黑".into());
    ctx.set_fonts(fonts);
    Ok(())
}

#[derive(Clone)]
enum Dialog {
    Connect {
        mode: u8,
        host: String,
        port: String,
        pairing_port: String,
        code: String,
        paired_id: String,
    },
    Settings,
    Devices,
    Notice {
        message: String,
    },
    Detail {
        device: Device,
        alias: String,
    },
    Tcp {
        device: Device,
        host: String,
        port: String,
    },
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
    drag_hovering: bool,
    file_picker: Option<bool>,
    opened: Instant,
    last_position: Option<[f32; 2]>,
    position_changed: Instant,
    position_checked: bool,
    applied_windowed: bool,
    last_size: Option<[f32; 2]>,
}

impl Drawer {
    fn new(
        cc: &eframe::CreationContext<'_>,
        storage: Arc<Storage>,
        backend: Backend,
    ) -> anyhow::Result<Self> {
        let preferences = storage.settings();
        let pixels = image::load_from_memory(include_bytes!("../../assets/AppIconDisplay.png"))?
            .into_rgba8();
        let menu = Menu::new();
        let open_menu = MenuItem::new("打开安装抽屉", true, None);
        let settings_menu = MenuItem::new("设置", true, None);
        let exit_menu = MenuItem::new("退出 QuickADB", true, None);
        menu.append_items(&[
            &open_menu,
            &settings_menu,
            &tray_icon::menu::PredefinedMenuItem::separator(),
            &exit_menu,
        ])?;
        let tray = TrayIconBuilder::new()
            .with_icon(platform::tray_icon()?)
            .with_tooltip("QuickADB · 点击安装 APK")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()?;
        let icon = cc.egui_ctx.load_texture(
            "app-icon",
            egui::ColorImage::from_rgba_unmultiplied(
                [pixels.width() as usize, pixels.height() as usize],
                pixels.as_raw(),
            ),
            egui::TextureOptions::LINEAR,
        );
        let hidden = std::env::args().any(|a| a == "--tray");
        if hidden {
            cc.egui_ctx
                .send_viewport_cmd(ViewportCommand::Visible(false));
        }
        let applied_windowed = preferences.windowed;
        let app = Self {
            backend,
            storage,
            preferences,
            _tray: tray,
            open_menu,
            exit_menu,
            settings_menu,
            dialog: None,
            icon,
            hidden,
            collapsed: false,
            exiting: false,
            was_focused: false,
            drag_hovering: false,
            file_picker: None,
            opened: Instant::now(),
            last_position: None,
            position_changed: Instant::now(),
            position_checked: false,
            applied_windowed,
            last_size: None,
        };
        app.apply_style(&cc.egui_ctx);
        cc.egui_ctx
            .request_repaint_after(Duration::from_millis(100));
        Ok(app)
    }

    fn apply_style(&self, ctx: &egui::Context) {
        let mut style = egui::Style {
            visuals: if self.preferences.dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            },
            ..Default::default()
        };
        style.visuals.widgets.noninteractive.fg_stroke.color = if self.preferences.dark {
            Color32::from_rgb(223, 231, 238)
        } else {
            Color32::from_rgb(41, 50, 59)
        };
        style.visuals.weak_text_color = Some(if self.preferences.dark {
            Color32::from_rgb(163, 175, 187)
        } else {
            Color32::from_rgb(99, 111, 123)
        });
        style.visuals.selection.bg_fill = GREEN.gamma_multiply(0.28);
        style.visuals.selection.stroke = Stroke::new(
            1.,
            if self.preferences.dark {
                GREEN
            } else {
                PRIMARY_GREEN
            },
        );
        style.visuals.panel_fill = if self.preferences.dark {
            Color32::from_rgb(24, 29, 34)
        } else {
            Color32::from_rgb(247, 249, 250)
        };
        style.visuals.window_fill = if self.preferences.dark {
            Color32::from_rgb(32, 38, 44)
        } else {
            Color32::WHITE
        };
        style.visuals.widgets.inactive.corner_radius = 7.into();
        style.visuals.widgets.hovered.corner_radius = 7.into();
        style.visuals.widgets.active.corner_radius = 7.into();
        style.spacing.item_spacing = Vec2::new(8., 8.);
        style.spacing.button_padding = Vec2::new(12., 8.);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(15.));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(15.));
        style
            .text_styles
            .insert(egui::TextStyle::Small, FontId::proportional(13.));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, FontId::proportional(19.));
        let theme = if self.preferences.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        };
        ctx.set_style_of(theme, style);
        ctx.set_theme(theme);
    }

    fn persist(&self) {
        if let Err(error) = self.storage.update_settings(|settings| {
            settings.startup = self.preferences.startup;
            settings.dark = self.preferences.dark;
            settings.pinned = self.preferences.pinned;
            settings.topmost = self.preferences.topmost;
            settings.test_packages = self.preferences.test_packages;
            settings.window_position = self.preferences.window_position;
            settings.windowed = self.preferences.windowed;
            settings.standard_window_size = self.preferences.standard_window_size;
            settings.standard_window_position = self.preferences.standard_window_position;
        }) {
            self.backend.notify(&format!("设置保存失败：{error:#}"));
        }
    }

    fn show(&mut self, ctx: &egui::Context, anchor: Option<[f32; 2]>) {
        if let Some([x, y]) = anchor
            && !self.preferences.pinned
            && !self.preferences.windowed
        {
            let dpi = ctx.input(|i| i.viewport().native_pixels_per_point.unwrap_or(1.));
            let area = platform::monitor_work_area();
            let height = ctx.content_rect().height();
            let left = area.left as f32 / dpi;
            let top = area.top as f32 / dpi;
            let position = egui::pos2(
                (x / dpi - WIDTH + 22.).clamp(left, (area.right as f32 / dpi - WIDTH).max(left)),
                (y / dpi - height - 12.).clamp(top, (area.bottom as f32 / dpi - height).max(top)),
            );
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position));
        }
        self.hidden = false;
        self.opened = Instant::now();
        self.was_focused = false;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(window_level(
            &self.preferences,
        )));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn hide(&mut self, ctx: &egui::Context) {
        self.hidden = true;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    fn set_window_mode(&mut self, ctx: &egui::Context, windowed: bool) {
        self.preferences.windowed = windowed;
        self.collapsed = false;
        self.position_checked = false;
        self.last_position = None;
        self.last_size = None;
        ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
        ctx.send_viewport_cmd(ViewportCommand::Decorations(windowed));
        ctx.send_viewport_cmd(ViewportCommand::Resizable(windowed));
        ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(Vec2::new(
            WIDTH,
            if windowed { 480. } else { 64. },
        )));
        let size = if windowed {
            self.preferences.standard_window_size.unwrap_or(WINDOW_SIZE)
        } else {
            [WIDTH, HEIGHT]
        };
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(size.into()));
        let position = if windowed {
            self.preferences.standard_window_position
        } else {
            self.preferences.window_position
        };
        if let Some(position) = position {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position.into()));
        }
        self.persist();
    }

    fn request_exit(&mut self, ctx: &egui::Context) {
        if self
            .backend
            .snapshot()
            .jobs
            .iter()
            .any(|j| j.stage.active())
        {
            self.show(ctx, None);
            self.dialog = Some(Dialog::Exit);
        } else {
            self.exiting = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn pick_apks(&self, split: bool) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title(if split {
                "选择一组拆分 APK（含基础包）"
            } else {
                "选择一个或多个 APK"
            })
            .add_filter("Android 安装包", &["apk"])
            .pick_files()
        {
            self.backend.prepare(paths, split);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let response = ui.horizontal(|ui| {
            ui.add(egui::Image::new(&self.icon).fit_to_exact_size(Vec2::splat(36.)));
            ui.vertical(|ui| {
                ui.label(RichText::new("QuickADB").size(20.).strong());
                ui.label(
                    RichText::new("随手安装，随时测试")
                        .size(13.)
                        .color(secondary_text(ui)),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.preferences.windowed
                    && ui
                        .add(egui::Button::new("×").frame(false))
                        .on_hover_text("收回托盘，安装继续")
                        .clicked()
                {
                    self.hide(ui.ctx());
                }
                if !self.preferences.windowed
                    && ui
                        .add(egui::Button::new("−").frame(false))
                        .on_hover_text("收成窄栏")
                        .clicked()
                {
                    self.collapsed = true;
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(WIDTH, 64.)));
                }
                if window_mode_button(ui, self.preferences.windowed).clicked() {
                    self.set_window_mode(ui.ctx(), !self.preferences.windowed);
                }
                if pin_button(ui, self.preferences.pinned, self.preferences.windowed).clicked() {
                    self.toggle_pin(ui.ctx());
                }
            });
        });
        let title_rect = egui::Rect::from_min_size(
            response.response.rect.min,
            Vec2::new(180., response.response.rect.height()),
        );
        if ui
            .interact(title_rect, ui.id().with("window-drag"), egui::Sense::drag())
            .drag_started()
        {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }

    fn toggle_pin(&mut self, ctx: &egui::Context) {
        self.preferences.pinned = !self.preferences.pinned;
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(window_level(
            &self.preferences,
        )));
        self.persist();
    }

    fn device_rows(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot, sidebar: bool) {
        let selected = snapshot.devices.iter().filter(|d| d.selected).count();
        ui.horizontal(|ui| {
            ui.label(RichText::new(if sidebar { "设备" } else { "安装设备" }).strong());
            if selected > 0 && !sidebar {
                ui.label(
                    RichText::new(format!("已选 {selected} 台"))
                        .small()
                        .color(positive_text(ui)),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !sidebar && ui.add(egui::Button::new("连接设备").frame(false)).clicked() {
                    self.dialog = Some(connect_dialog());
                }
                if ui
                    .add(
                        egui::Button::new(if selected == 0 {
                            "全选"
                        } else {
                            "取消全选"
                        })
                        .frame(false),
                    )
                    .clicked()
                {
                    self.backend.select_all(selected == 0);
                }
            });
        });
        if sidebar {
            ui.label(
                RichText::new(format!(
                    "{} 台设备 · 已选 {selected} 台",
                    snapshot.devices.len()
                ))
                .small()
                .color(secondary_text(ui)),
            );
            if ui
                .add_sized([ui.available_width(), 36.], egui::Button::new("连接设备"))
                .clicked()
            {
                self.dialog = Some(connect_dialog());
            }
            ui.add_space(8.);
        }
        if snapshot.devices.is_empty() {
            card(ui, self.preferences.dark, |ui| {
                ui.add_space(8.);
                ui.label(RichText::new("等待连接第一台设备").strong());
                ui.label(
                    RichText::new(
                        "USB 连接后开启调试并在手机上授权，\n也可使用同一局域网的无线调试。",
                    )
                    .small()
                    .color(secondary_text(ui)),
                );
                ui.add_space(8.);
            });
        } else {
            egui::ScrollArea::vertical()
                .id_salt("devices")
                .max_height(if sidebar {
                    ui.available_height().max(0.)
                } else {
                    224.
                })
                .show(ui, |ui| {
                    for device in &snapshot.devices {
                        ui.horizontal(|ui| {
                            let online = device.status == DeviceStatus::Online;
                            let row_width = ui.available_width() - if sidebar { 0. } else { 44. };
                            let status = format!(
                                "{} · {}{}",
                                device.transport,
                                device.status.label(),
                                if device.android.is_empty() {
                                    String::new()
                                } else {
                                    format!(" · Android {}", device.android)
                                }
                            );
                            let frame = egui::Frame::new()
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .corner_radius(6)
                                .fill(if device.selected {
                                    GREEN.gamma_multiply(if self.preferences.dark {
                                        0.22
                                    } else {
                                        0.12
                                    })
                                } else {
                                    ui.visuals().window_fill
                                })
                                .stroke(Stroke::new(
                                    1.,
                                    if device.selected {
                                        GREEN
                                    } else {
                                        ui.visuals().widgets.noninteractive.bg_stroke.color
                                    },
                                ));
                            let selection = ui
                                .add_enabled_ui(device.selectable() || sidebar, |ui| {
                                    let content_width =
                                        (row_width - frame.total_margin().sum().x).max(1.);
                                    frame
                                        .show(ui, |ui| {
                                            ui.set_width(content_width);
                                            ui.set_min_height(42.);
                                            ui.vertical(|ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new(if device.selected {
                                                            "●"
                                                        } else {
                                                            "○"
                                                        })
                                                        .color(if device.selected {
                                                            GREEN
                                                        } else {
                                                            secondary_text(ui)
                                                        }),
                                                    );
                                                    ui.vertical(|ui| {
                                                        if sidebar {
                                                            ui.set_width((row_width - 40.).max(1.));
                                                        }
                                                        let name = egui::Label::new(
                                                            RichText::new(&device.name)
                                                                .size(14.)
                                                                .strong(),
                                                        );
                                                        ui.add(if sidebar {
                                                            name.wrap()
                                                        } else {
                                                            name.truncate()
                                                        })
                                                        .on_hover_text(&device.name);
                                                        if !sidebar {
                                                            ui.add(
                                                                egui::Label::new(
                                                                    RichText::new(&status)
                                                                        .small()
                                                                        .color(secondary_text(ui)),
                                                                )
                                                                .truncate(),
                                                            );
                                                        }
                                                    });
                                                });
                                                if sidebar {
                                                    ui.horizontal(|ui| {
                                                        ui.with_layout(
                                                            egui::Layout::right_to_left(
                                                                egui::Align::Center,
                                                            ),
                                                            |ui| {
                                                                if ui
                                                                    .add(
                                                                        egui::Button::new("详情")
                                                                            .small()
                                                                            .frame(false),
                                                                    )
                                                                    .clicked()
                                                                {
                                                                    self.device_detail(device);
                                                                }
                                                                ui.add(
                                                                    egui::Label::new(
                                                                        RichText::new(&status)
                                                                            .small()
                                                                            .color(secondary_text(
                                                                                ui,
                                                                            )),
                                                                    )
                                                                    .truncate(),
                                                                );
                                                            },
                                                        );
                                                    });
                                                }
                                            });
                                        })
                                        .response
                                        .interact(if device.selectable() {
                                            egui::Sense::click()
                                        } else {
                                            egui::Sense::hover()
                                        })
                                })
                                .inner;
                            selection.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::Checkbox,
                                    device.selectable(),
                                    device.selected,
                                    format!("选择设备：{}", device.name),
                                )
                            });
                            if selection
                                .on_hover_text(if online {
                                    "点击选择，再次点击取消；可同时选择多台"
                                } else {
                                    "点击选择；安装时会尝试重新连接此无线设备"
                                })
                                .clicked()
                            {
                                self.backend.toggle(&device.id);
                            }
                            if !sidebar
                                && ui
                                    .add(egui::Button::new("详情").small().frame(false))
                                    .on_hover_text("设备详情与连接操作")
                                    .clicked()
                            {
                                self.device_detail(device);
                            }
                        });
                    }
                });
        }
    }

    fn device_detail(&mut self, device: &Device) {
        self.dialog = Some(Dialog::Detail {
            device: device.clone(),
            alias: self
                .storage
                .settings()
                .aliases
                .get(&device.serial)
                .cloned()
                .unwrap_or_default(),
        });
    }

    fn drop_zone(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let hovering = self.drag_hovering;
        let selected = snapshot
            .devices
            .iter()
            .filter(|d| d.selected && d.selectable())
            .count();
        let border = if hovering {
            GREEN
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke.color
        };
        egui::Frame::new()
            .fill(if hovering {
                GREEN.gamma_multiply(0.10)
            } else {
                ui.visuals().window_fill
            })
            .stroke(Stroke::new(1.3, border))
            .corner_radius(12)
            .inner_margin(16)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if let Some(preparation) = &snapshot.preparation {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("待安装 · {} 个 APK", preparation.paths.len()))
                                .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("清空").frame(false)).clicked() {
                                self.backend.clear_preparation();
                            }
                        });
                    });
                    ui.add_space(4.);
                    egui::ScrollArea::vertical()
                        .id_salt("prepared-apks")
                        .max_height(if self.preferences.windowed { 120. } else { 88. })
                        .min_scrolled_height(0.)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for (index, path) in preparation.paths.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    let width = (ui.available_width() - 38.).max(1.);
                                    ui.add_sized(
                                        [width, 24.],
                                        egui::Label::new(
                                            RichText::new(
                                                path.file_name()
                                                    .unwrap_or(path.as_os_str())
                                                    .to_string_lossy(),
                                            )
                                            .size(14.),
                                        )
                                        .truncate()
                                        .halign(egui::Align::Min),
                                    )
                                    .on_hover_text(path.display().to_string());
                                    if ui
                                        .add_sized([28., 24.], egui::Button::new("×").frame(false))
                                        .on_hover_text("移除此 APK")
                                        .clicked()
                                    {
                                        let mut paths = preparation.paths.clone();
                                        paths.remove(index);
                                        self.backend.prepare(paths, preparation.split);
                                    }
                                });
                            }
                            if let PreparationState::Failed(error) = &preparation.state {
                                ui.label(
                                    RichText::new(format!("准备失败：{error}"))
                                        .small()
                                        .color(ui.visuals().error_fg_color),
                                );
                            }
                        });
                    ui.add_space(4.);
                    match &preparation.state {
                        PreparationState::Checking => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("正在检查安装包…");
                            });
                        }
                        PreparationState::Ready(apks) => {
                            let total: u64 = apks.iter().map(|a| a.size).sum();
                            ui.label(
                                RichText::new(format!(
                                    "已准备 · {} · 点击安装后开始",
                                    bytes(total as f64)
                                ))
                                .small()
                                .color(secondary_text(ui)),
                            );
                        }
                        PreparationState::Failed(_) => {}
                    }
                    if preparation.paths.len() > 1 {
                        let mut split = preparation.split;
                        if ui
                            .checkbox(&mut split, "作为一个应用的拆分 APK 安装")
                            .on_hover_text(
                                "仅适用于同一应用、同一版本的基础包和拆分包；普通批量 APK 无需勾选",
                            )
                            .changed()
                        {
                            self.backend.prepare(preparation.paths.clone(), split);
                        }
                    }
                    if hovering {
                        ui.label(
                            RichText::new("松开以更换待安装文件")
                                .small()
                                .color(positive_text(ui)),
                        );
                    }
                } else {
                    if self.preferences.windowed {
                        ui.add_space(8.);
                        ui.horizontal(|ui| {
                            apk_symbol(ui);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(if hovering {
                                        "松开以准备安装包"
                                    } else {
                                        "拖入 APK，或选择安装包"
                                    })
                                    .size(18.)
                                    .strong(),
                                );
                                ui.label(
                                    RichText::new("支持批量及拆分 APK，准备完成后点击开始安装")
                                        .small()
                                        .color(secondary_text(ui)),
                                );
                            });
                        });
                        ui.add_space(8.);
                    } else {
                        ui.vertical_centered(|ui| {
                            apk_symbol(ui);
                            ui.label(
                                RichText::new(if hovering {
                                    "松开以准备安装包"
                                } else {
                                    "把 APK 拖到这里"
                                })
                                .size(18.)
                                .strong(),
                            );
                            ui.label(
                                RichText::new("支持批量拖入 · 准备后点击安装")
                                    .small()
                                    .color(secondary_text(ui)),
                            );
                        });
                    }
                }
                ui.add_space(8.);
                if self.preferences.windowed && ui.available_width() >= 520. {
                    ui.horizontal(|ui| {
                        self.apk_picker_button(ui, snapshot, false, 120.);
                        self.apk_picker_button(ui, snapshot, true, 144.);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.install_prepared_button(ui, snapshot, selected, 180.);
                        });
                    });
                    return;
                }
                ui.columns(2, |columns| {
                    let first_width = columns[0].available_width();
                    self.apk_picker_button(&mut columns[0], snapshot, false, first_width);
                    let second_width = columns[1].available_width();
                    self.apk_picker_button(&mut columns[1], snapshot, true, second_width);
                });
                if snapshot.preparation.is_none() && !self.preferences.windowed {
                    return;
                }
                ui.add_space(8.);
                self.install_prepared_button(ui, snapshot, selected, ui.available_width());
            });
    }

    fn apk_picker_button(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        split: bool,
        width: f32,
    ) {
        let label = if split {
            "选择拆分 APK"
        } else if snapshot.preparation.is_some() {
            "更换 APK"
        } else {
            "选择 APK"
        };
        if ui
            .add_sized([width, 36.], egui::Button::new(label))
            .clicked()
        {
            self.file_picker = Some(split);
            ui.ctx().request_repaint();
        }
    }

    fn install_prepared_button(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        selected: usize,
        width: f32,
    ) {
        let ready = snapshot
            .preparation
            .as_ref()
            .is_some_and(|p| matches!(p.state, PreparationState::Ready(_)));
        let label = if selected > 0 && snapshot.preparation.is_some() {
            format!("安装到 {selected} 台设备")
        } else if self.preferences.windowed {
            "开始安装".into()
        } else {
            "安装 · 请先选择设备".into()
        };
        let clicked = ui
            .add_enabled_ui(ready && selected > 0, |ui| {
                ui.add_sized([width, 40.], primary_button(&label))
            })
            .inner
            .on_hover_text(if selected == 0 {
                "先在设备列表选择安装目标"
            } else if !ready {
                "先添加并检查安装包"
            } else {
                "安装到选中的设备；离线无线设备会先尝试重连"
            })
            .clicked();
        if clicked && let Some(preparation) = &snapshot.preparation {
            self.backend
                .install_prepared(preparation.revision, self.preferences.test_packages);
            self.opened = Instant::now();
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        if self.preferences.windowed {
            ui.separator();
            ui.horizontal(|ui| {
                device_summary(ui, snapshot);
                ui.label(
                    RichText::new(format!(
                        "已选 {} 台",
                        snapshot.devices.iter().filter(|d| d.selected).count()
                    ))
                    .small()
                    .color(secondary_text(ui)),
                );
                self.notice_preview(ui, snapshot);
            });
            return;
        }
        self.notice_preview(ui, snapshot);
        ui.separator();
        ui.horizontal(|ui| {
            device_summary(ui, snapshot);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("设置").frame(false)).clicked() {
                    self.dialog = Some(Dialog::Settings);
                }
                if ui.add(egui::Button::new("设备管理").frame(false)).clicked() {
                    self.dialog = Some(Dialog::Devices);
                }
            });
        });
    }

    fn notice_preview(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        if !snapshot.notice.is_empty() {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("关闭提示").clicked() {
                        self.backend.clear_notice();
                    }
                    let width = ui.available_width();
                    ui.allocate_ui_with_layout(
                        Vec2::new(width, 24.),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(
                                            snapshot.notice.lines().next().unwrap_or_default(),
                                        )
                                        .small()
                                        .color(ui.visuals().warn_fg_color),
                                    )
                                    .truncate()
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_text("点击查看完整提示")
                                .clicked()
                            {
                                self.dialog = Some(Dialog::Notice {
                                    message: snapshot.notice.clone(),
                                });
                            }
                        },
                    );
                });
            });
        }
    }

    fn job_row(&mut self, ui: &mut egui::Ui, job: &Job, table: bool) {
        card(ui, self.preferences.dark, |ui| {
            if table {
                let widths = task_column_widths(ui.available_width(), ui.spacing().item_spacing.x);
                ui.horizontal(|ui| {
                    task_cell(ui, widths[0], |ui| {
                        ui.add(egui::Label::new(RichText::new(&job.title).strong()).truncate())
                            .on_hover_text(&job.title);
                    });
                    task_cell(ui, widths[1], |ui| {
                        ui.add(egui::Label::new(&job.device_name).truncate())
                            .on_hover_text(&job.device_name);
                    });
                    task_cell(ui, widths[2], |ui| self.job_status(ui, job));
                    task_cell(ui, widths[3], |ui| self.job_action(ui, job));
                });
            } else {
                ui.horizontal(|ui| {
                    let label_width = (ui.available_width() - 52.).max(1.);
                    ui.allocate_ui_with_layout(
                        Vec2::new(label_width, 32.),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&job.title).strong().size(14.))
                                    .truncate(),
                            )
                            .on_hover_text(&job.title);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&job.device_name)
                                        .small()
                                        .color(secondary_text(ui)),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&job.device_name);
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.job_action(ui, job);
                    });
                });
            }
            if job.stage == JobStage::Transferring {
                let fraction = if job.total == 0 {
                    0.
                } else {
                    job.transferred as f32 / job.total as f32
                };
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .fill(GREEN)
                        .desired_height(5.),
                );
                ui.label(
                    RichText::new(format!(
                        "传输 {:.0}%  ·  {} / {}  ·  {}/s",
                        fraction * 100.,
                        bytes(job.transferred as f64),
                        bytes(job.total as f64),
                        bytes(job.bytes_per_second)
                    ))
                    .small()
                    .color(secondary_text(ui)),
                );
            } else if !table {
                self.job_status(ui, job);
            }
            if !job.detail.is_empty() {
                egui::CollapsingHeader::new("查看结果详情")
                    .id_salt(job.id)
                    .show(ui, |ui| {
                        ui.label(error_help(&job.detail));
                        ui.label(RichText::new(&job.detail).small());
                        if ui.small_button("复制原始结果").clicked() {
                            ui.ctx().copy_text(job.detail.clone());
                        }
                    });
            }
        });
    }

    fn job_action(&mut self, ui: &mut egui::Ui, job: &Job) {
        if matches!(
            job.stage,
            JobStage::Queued | JobStage::Connecting | JobStage::Preparing | JobStage::Transferring
        ) {
            if ui.small_button("取消").clicked() {
                self.backend.cancel(job.id);
            }
        } else if matches!(
            job.stage,
            JobStage::Failed | JobStage::Canceled | JobStage::Unknown
        ) && ui.small_button("重试").clicked()
        {
            self.backend.retry_job(job.id);
        }
    }

    fn job_status(&self, ui: &mut egui::Ui, job: &Job) {
        ui.horizontal(|ui| {
            if matches!(
                job.stage,
                JobStage::Connecting | JobStage::Preparing | JobStage::Installing
            ) {
                ui.add(egui::Spinner::new().size(12.));
            }
            let color = match job.stage {
                JobStage::Succeeded => positive_text(ui),
                JobStage::Failed | JobStage::Unknown => ui.visuals().error_fg_color,
                _ => secondary_text(ui),
            };
            ui.add(
                egui::Label::new(RichText::new(job.stage.label()).small().color(color)).truncate(),
            )
            .on_hover_text(job.stage.label());
        });
    }

    fn connection_form(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        dialog: &mut Dialog,
        close: &mut bool,
    ) {
        let Dialog::Connect {
            mode,
            host,
            port,
            pairing_port,
            code,
            paired_id,
        } = dialog
        else {
            return;
        };
        modal_heading(ui, "连接设备", close);
        ui.label(
            RichText::new("无线连接需在同一局域网，USB 设备会自动识别。")
                .size(13.)
                .color(secondary_text(ui)),
        );
        ui.add_space(10.);
        let tab_width = (ui.available_width() - 16.) / 3.;
        ui.horizontal(|ui| {
            for (value, label) in [(0, "无线配对"), (1, "已配对设备"), (2, "TCP 连接")] {
                let active = *mode == value;
                if ui
                    .add_sized(
                        [tab_width, 36.],
                        egui::Button::new(RichText::new(label).color(if active {
                            Color32::WHITE
                        } else {
                            ui.visuals().text_color()
                        }))
                        .fill(if active {
                            PRIMARY_GREEN
                        } else {
                            ui.visuals().widgets.inactive.bg_fill
                        })
                        .stroke(Stroke::NONE)
                        .corner_radius(8),
                    )
                    .clicked()
                {
                    *mode = value;
                    code.clear();
                    port.clear();
                }
            }
        });
        ui.add_space(8.);
        egui::Frame::new()
            .fill(if self.preferences.dark {
                Color32::from_rgb(29, 48, 42)
            } else {
                Color32::from_rgb(238, 248, 243)
            })
            .corner_radius(10)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if *mode == 0 {
                    ui.label(RichText::new("先在手机上打开配对码弹窗").strong());
                    ui.label(RichText::new("开发者选项 → 无线调试 → 使用配对码配对设备").size(13.));
                    ui.label(
                        RichText::new("适用于 Android 11 及以上；请保持配对弹窗打开。")
                            .small()
                            .color(secondary_text(ui)),
                    );
                    ui.label(
                        RichText::new("此弹窗由手机操作打开，电脑不能远程触发。")
                            .small()
                            .color(secondary_text(ui)),
                    );
                } else if *mode == 1 {
                    ui.label(RichText::new("连接已授权的手机").strong());
                    ui.label(
                        RichText::new(
                            "选择配对记录后自动发现当前地址与端口，手机需保持无线调试开启。",
                        )
                        .size(13.)
                        .color(secondary_text(ui)),
                    );
                } else {
                    ui.label(RichText::new("连接已启用 TCP 调试的设备").strong());
                    ui.label(
                        RichText::new("首次使用可先接 USB，在设备详情中选择“通过 USB 转无线”。")
                            .size(13.)
                            .color(secondary_text(ui)),
                    );
                }
            });
        ui.add_space(8.);
        ui.label(
            RichText::new(format!("局域网发现 · {} 个服务", snapshot.discovered.len())).strong(),
        );
        if snapshot.discovered.is_empty() {
            ui.label(
                RichText::new("暂未发现设备，请确认手机已开启无线调试并与电脑在同一局域网。")
                    .small()
                    .color(secondary_text(ui)),
            );
        } else {
            ui.label(
                RichText::new("自动更新 · 点击设备选择连接方式")
                    .small()
                    .color(secondary_text(ui)),
            );
            let paired = self.storage.paired_devices();
            egui::ScrollArea::vertical()
                .id_salt("discovered-devices")
                .max_height(144.)
                .min_scrolled_height(0.)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for device in &snapshot.discovered {
                        let endpoint = &device.endpoint;
                        let known_name = snapshot.devices.iter().find_map(|known| {
                            known.endpoint.as_ref().filter(|address| address.host == endpoint.host && address.port == endpoint.port).map(|_| known.name.as_str())
                        });
                        let name = known_name.unwrap_or(&device.name);
                        let authorized_id = device.paired_id().filter(|id| {
                            paired.iter().any(|record| record.device_id == *id)
                        });
                        let kind = match device.service_type {
                            AdbServiceType::Pairing => "可配对 · 填入配对端口",
                            AdbServiceType::TlsConnect if authorized_id.is_some() => "已配对 · 自动获取连接端口",
                            AdbServiceType::TlsConnect => "无配对记录 · 在手机打开配对码弹窗",
                            AdbServiceType::Legacy => "传统 TCP · 填入连接端口",
                        };
                        let response = egui::Frame::new()
                            .fill(ui.visuals().widgets.inactive.bg_fill)
                            .corner_radius(8)
                            .inner_margin(10)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(egui::Label::new(RichText::new(name).strong()).truncate());
                                ui.label(RichText::new(format!("{}:{} · {kind}", endpoint.host, endpoint.port)).small().color(secondary_text(ui)));
                            }).response.interact(egui::Sense::click());
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("选择{name} · {kind}")));
                        let response = response.on_hover_text(if device.model_advertised || known_name.is_some() {
                            device.fullname.as_str()
                        } else {
                            "设备没有广播型号，当前显示其真实服务名称。连接授权后可读取型号并设置备注。"
                        });
                        if response.clicked() {
                            *host = endpoint.host.clone();
                            pairing_port.clear();
                            port.clear();
                            code.clear();
                            paired_id.clear();
                            match device.service_type {
                                AdbServiceType::Pairing => {
                                    *mode = 0;
                                    *pairing_port = endpoint.port.to_string();
                                }
                                AdbServiceType::TlsConnect => {
                                    if let Some(id) = authorized_id {
                                        *mode = 1;
                                        *paired_id = id.into();
                                    } else {
                                        *mode = 0;
                                    }
                                }
                                AdbServiceType::Legacy => {
                                    *mode = 2;
                                    *port = endpoint.port.to_string();
                                }
                            }
                        }
                    }
                });
        }
        ui.add_space(8.);
        if *mode == 1 {
            let paired = self.storage.paired_devices();
            if paired.is_empty() {
                ui.label("还没有配对记录，请先完成无线配对。");
                if ui.button("去无线配对").clicked() {
                    *mode = 0;
                }
            } else {
                ui.label(RichText::new("选择设备记录").strong());
                let selected = paired.iter().find(|e| e.device_id == *paired_id);
                let record_label = |id: &str, host: &str| {
                    let name = snapshot
                        .devices
                        .iter()
                        .find(|d| d.id == format!("tls:{id}"))
                        .map(|d| d.name.as_str())
                        .or_else(|| {
                            snapshot
                                .discovered
                                .iter()
                                .find(|d| d.paired_id() == Some(id))
                                .map(|d| d.name.as_str())
                        })
                        .unwrap_or(id);
                    format!("{name} · {host}")
                };
                let label = selected
                    .map(|e| record_label(&e.device_id, &e.host))
                    .unwrap_or_else(|| "请选择已配对的设备".into());
                egui::ComboBox::from_id_salt("paired-device")
                    .width(ui.available_width())
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for record in &paired {
                            if ui
                                .selectable_label(
                                    record.device_id == *paired_id,
                                    record_label(&record.device_id, &record.host),
                                )
                                .clicked()
                            {
                                *host = record.host.clone();
                                port.clear();
                                *paired_id = record.device_id.clone();
                            }
                        }
                    });
                ui.add_space(6.);
            }
        }
        if *mode != 1 {
            field(ui, "手机 IP 地址", host, false, "例如 192.168.1.8");
        }
        if *mode == 0 {
            ui.columns(2, |columns| {
                field(
                    &mut columns[0],
                    "配对端口",
                    pairing_port,
                    false,
                    "配对弹窗中的端口",
                );
                field(&mut columns[1], "六位配对码", code, true, "输入 6 位数字");
            });
        }
        if *mode == 2 {
            field(ui, "连接端口", port, false, "通常为 5555，以设备设置为准");
        } else {
            ui.label(
                RichText::new(if port.trim().is_empty() {
                    "连接端口自动发现，无需填写。"
                } else {
                    "已指定手动端口，清空后恢复自动发现。"
                })
                .size(13.)
                .color(secondary_text(ui)),
            );
            egui::CollapsingHeader::new("手动填写连接端口（可选）")
                .id_salt("manual-wireless-port")
                .show(ui, |ui| {
                    if *mode == 1 {
                        field(ui, "手机 IP 地址", host, false, "无线调试主页中的 IP 地址");
                    }
                    field(ui, "连接端口", port, false, "留空自动发现");
                    ui.label(
                        RichText::new(
                            "仅在自动发现不可用时填写无线调试主页的端口，与配对端口不同。",
                        )
                        .size(13.)
                        .color(secondary_text(ui)),
                    );
                });
        }
        ui.add_space(10.);
        if !port.trim().is_empty() && !port.trim().parse::<u16>().is_ok_and(|p| p > 0) {
            ui.colored_label(ui.visuals().error_fg_color, "连接端口需为 1–65535 的整数。");
        }
        let valid = !host.trim().is_empty()
            && ((*mode != 2 && port.trim().is_empty())
                || port.trim().parse::<u16>().is_ok_and(|p| p > 0))
            && (*mode != 0
                || (pairing_port.parse::<u16>().is_ok_and(|p| p > 0)
                    && code.len() == 6
                    && code.bytes().all(|b| b.is_ascii_digit())))
            && (*mode != 1 || !paired_id.is_empty());
        if snapshot.pairing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("正在验证配对码…");
            });
        }
        let text = if *mode == 0 {
            "配对并连接"
        } else {
            "连接设备"
        };
        if ui
            .add_enabled_ui(valid && !snapshot.pairing, |ui| {
                ui.add_sized([ui.available_width(), 42.], primary_button(text))
            })
            .inner
            .clicked()
        {
            let connection_port = if port.trim().is_empty() {
                None
            } else {
                Some(port.trim().parse().expect("validated port"))
            };
            if *mode == 0 {
                self.backend.pair(
                    host.trim().into(),
                    pairing_port.parse().expect("validated pairing port"),
                    code.clone(),
                    connection_port,
                );
            } else if *mode == 1 && connection_port.is_none() {
                self.backend.connect_paired(paired_id.clone());
            } else {
                self.backend.connect(Endpoint {
                    host: host.trim().into(),
                    port: connection_port.expect("validated manual port"),
                    paired_id: if *mode == 1 {
                        Some(paired_id.clone())
                    } else {
                        None
                    },
                });
            }
            code.clear();
            *close = true;
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context, snapshot: &Snapshot) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut close = false;
        let dialog_height = (ctx.content_rect().height() - 48.).max(120.);
        let response = egui::Modal::new(egui::Id::new("drawer-dialog")).frame(egui::Frame::popup(&ctx.global_style()).corner_radius(14).inner_margin(16)).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.).min(if self.preferences.windowed { 520. } else { 368. }).max(1.));
            egui::ScrollArea::vertical()
                .id_salt("dialog-content")
                .max_height(dialog_height)
                .min_scrolled_height(dialog_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
            match &mut dialog {
                Dialog::Connect { .. } => self.connection_form(ui, snapshot, &mut dialog, &mut close),
                Dialog::Settings => {
                    modal_heading(ui, "设置", &mut close);
                    let mut windowed = self.preferences.windowed;
                    if ui.checkbox(&mut windowed, "标准窗口模式（支持调整大小与 Windows 贴靠）").changed() {
                        self.set_window_mode(ctx, windowed);
                    }
                    ui.label(RichText::new("常驻行为").strong());
                    let before = self.preferences.startup;
                    if ui.checkbox(&mut self.preferences.startup, "开机启动，静默进入托盘").changed() {
                        if let Err(error) = platform::set_startup(self.preferences.startup) { self.preferences.startup = before; self.backend.notify(&format!("开机启动设置失败：{error:#}")); }
                        else { self.persist(); }
                    }
                    if ui.checkbox(&mut self.preferences.pinned, "固定抽屉，置顶且点击外部时不收起").changed() {
                        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(window_level(&self.preferences))); self.persist();
                    }
                    let mut topmost = self.preferences.pinned || self.preferences.topmost;
                    if ui.add_enabled(!self.preferences.pinned, egui::Checkbox::new(&mut topmost, "窗口保持置顶"))
                        .on_hover_text("固定抽屉时自动保持置顶；取消固定后可单独设置置顶。")
                        .changed() {
                        self.preferences.topmost = topmost;
                        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(window_level(&self.preferences))); self.persist();
                    }
                    ui.add_space(6.);
                    ui.label(RichText::new("外观与安装").strong());
                    if ui.checkbox(&mut self.preferences.dark, "深色外观").changed() { self.apply_style(ctx); self.persist(); }
                    if ui.checkbox(&mut self.preferences.test_packages, "允许标记为 testOnly 的测试 APK").changed() { self.persist(); }
                    ui.label(RichText::new("默认覆盖更新并保留应用数据。").small().color(secondary_text(ui)));
                    ui.add_space(8.);
                    ui.label(RichText::new("数据目录").strong());
                    ui.label(RichText::new(self.storage.directory.display().to_string()).small());
                    ui.label(RichText::new("授权密钥由当前 Windows 账号保护。").small().color(secondary_text(ui)));
                    if ui.small_button("复制数据目录").clicked() { ctx.copy_text(self.storage.directory.display().to_string()); }
                    ui.separator();
                    ui.label(RichText::new(concat!("QuickADB ", env!("CARGO_PKG_VERSION"))).small().color(secondary_text(ui)));
                    if ui.small_button("复制开源组件许可").clicked() {
                        ctx.copy_text(include_str!("../../assets/ThirdPartyNotices.txt").into());
                    }
                }
                Dialog::Devices => {
                    modal_heading(ui, "设备管理", &mut close);
                    egui::ScrollArea::vertical().max_height(310.).show(ui, |ui| {
                        for device in &snapshot.devices {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| { ui.label(RichText::new(&device.name).strong()); ui.label(RichText::new(format!("{} · {}", device.transport, device.status.label())).small().color(secondary_text(ui))); });
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
                                if ui.small_button("移除记录").clicked()
                                    && let Err(error) = self.storage.update_settings(|settings| settings.endpoints.retain(|e| e.key() != endpoint.key())) { self.backend.notify(&format!("移除记录失败：{error:#}")); }
                            });
                        }
                    });
                }
                Dialog::Notice { message } => {
                    modal_heading(ui, "提示详情", &mut close);
                    egui::ScrollArea::vertical().max_height(360.).show(ui, |ui| {
                        ui.add(egui::Label::new(message.as_str()).wrap());
                    });
                    if ui.button("复制完整提示").clicked() { ctx.copy_text(message.clone()); }
                }
                Dialog::Detail { device, alias } => {
                    modal_heading(ui, "设备详情", &mut close);
                    ui.label(RichText::new(&device.name).size(16.).strong());
                    ui.label(format!("{} · {}", device.transport, device.status.label()));
                    ui.label(format!("序列号：{}", device.serial));
                    ui.label(format!("Android：{}", device.android));
                    if !device.detail.is_empty() { ui.label(&device.detail); }
                    field(ui, "设备备注", alias, false, "为设备起一个容易识别的名字");
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
                    ui.label(RichText::new("手机与电脑需处于可信的同一局域网。该操作会重启手机调试服务，USB 连接可能中断。").small().color(secondary_text(ui)));
                    field(ui, "手机当前局域网 IP", host, false, "例如 192.168.1.8");
                    field(ui, "TCP 调试端口", port, false, "5555");
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
        });
        if !close && !response.should_close() {
            self.dialog = Some(dialog);
        }
    }
}

impl eframe::App for Drawer {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if let Some(split) = self.file_picker.take() {
            self.pick_apks(split);
            self.opened = Instant::now();
            self.was_focused = false;
        }
        self.drag_hovering = !input.hovered_files.is_empty();
        if self.drag_hovering {
            self.opened = Instant::now();
        }
        let files = std::mem::take(&mut input.dropped_files);
        if !files.is_empty() {
            self.backend.prepare(
                files.iter().map(|f| f.path().to_path_buf()).collect(),
                false,
            );
            if self.collapsed {
                self.collapsed = false;
                self.position_checked = false;
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(WIDTH, HEIGHT)));
            }
            self.show(ctx, None);
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.applied_windowed != self.preferences.windowed {
            if let Some(window) = frame.winit_window() {
                window.set_skip_taskbar(!self.preferences.windowed);
            }
            self.applied_windowed = self.preferences.windowed;
        }
        if self.hidden && platform::window_visible() {
            self.show(ctx, None);
        }
        if !self.position_checked
            && let Some((outer, inner, dpi)) = ctx.input(|i| {
                let viewport = i.viewport();
                Some((
                    viewport.outer_rect?,
                    viewport.inner_rect?,
                    viewport.native_pixels_per_point?,
                ))
            })
        {
            match platform::fit_window_to_work_area(outer.min.into(), outer.size().into(), dpi) {
                Ok((position, size)) => {
                    if Vec2::from(size) != outer.size() {
                        let chrome = outer.size() - inner.size();
                        ctx.send_viewport_cmd(ViewportCommand::InnerSize(
                            Vec2::from(size) - chrome,
                        ));
                    }
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position.into()));
                }
                Err(error) => self.backend.notify(&format!("恢复窗口位置失败：{error:#}")),
            }
            self.position_checked = true;
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                self.show(ctx, Some([position.x as f32, position.y as f32]));
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *self.open_menu.id() {
                self.show(ctx, None);
            } else if event.id == *self.exit_menu.id() {
                self.request_exit(ctx);
            } else if event.id == *self.settings_menu.id() {
                self.show(ctx, None);
                self.dialog = Some(Dialog::Settings);
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.exiting {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.hide(ctx);
        }
        if !self.hidden {
            let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
            if focused {
                self.was_focused = true;
            }
            if self.was_focused
                && !focused
                && !self.preferences.pinned
                && !self.preferences.windowed
                && self.dialog.is_none()
                && !self.drag_hovering
                && !platform::pointer_button_down()
                && self.opened.elapsed() > Duration::from_millis(600)
            {
                self.hide(ctx);
            }
            if (self.preferences.pinned || self.preferences.windowed)
                && let Some((position, size)) = ctx.input(|i| {
                    let viewport = i.viewport();
                    if viewport.maximized == Some(true) || viewport.minimized == Some(true) {
                        return None;
                    }
                    Some((
                        viewport.outer_rect?.min.into(),
                        viewport.inner_rect?.size().into(),
                    ))
                })
            {
                if self.last_position != Some(position) || self.last_size != Some(size) {
                    self.last_position = Some(position);
                    self.last_size = Some(size);
                    self.position_changed = Instant::now();
                } else if self.position_changed.elapsed() > Duration::from_millis(700) {
                    if self.preferences.windowed {
                        if self.preferences.standard_window_position != Some(position)
                            || self.preferences.standard_window_size != Some(size)
                        {
                            self.preferences.standard_window_position = Some(position);
                            self.preferences.standard_window_size = Some(size);
                            self.persist();
                        }
                    } else if self.preferences.window_position != Some(position) {
                        self.preferences.window_position = Some(position);
                        self.persist();
                    }
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(if self.hidden { 250 } else { 100 }));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.collapsed {
            egui::Frame::new()
                .fill(ui.visuals().panel_fill)
                .inner_margin(12)
                .show(ui, |ui| {
                    let response = ui.horizontal(|ui| {
                        ui.add(egui::Image::new(&self.icon).fit_to_exact_size(Vec2::splat(32.)));
                        let snapshot = self.backend.snapshot();
                        let count = snapshot.jobs.iter().filter(|j| j.stage.active()).count();
                        if ui
                            .add(
                                egui::Button::new(if count > 0 {
                                    format!("QuickADB · {count} 项安装中")
                                } else {
                                    "QuickADB · 展开安装抽屉".into()
                                })
                                .frame(false),
                            )
                            .clicked()
                        {
                            self.collapsed = false;
                            self.position_checked = false;
                            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(
                                WIDTH, HEIGHT,
                            )));
                        }
                        if ui.small_button("×").clicked() {
                            self.hide(&ctx);
                        }
                    });
                    if ui
                        .interact(
                            response.response.rect,
                            ui.id().with("collapsed-drag"),
                            egui::Sense::drag(),
                        )
                        .drag_started()
                    {
                        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                    }
                });
            return;
        }
        let snapshot = self.backend.snapshot();
        if self.preferences.windowed {
            self.window_ui(ui, &snapshot);
            self.dialogs(&ctx, &snapshot);
            return;
        }
        egui::Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(16)
            .show(ui, |ui| {
                egui::Panel::bottom("drawer-footer")
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| self.footer(ui, &snapshot));
                self.header(ui);
                ui.add_space(7.);
                ui.separator();
                ui.add_space(5.);
                let task_height = if snapshot.jobs.is_empty() { 56. } else { 132. };
                let height = (ui.available_height() - task_height).max(0.);
                self.preparation_ui(ui, &snapshot, height);
                ui.add_space(6.);
                self.installation_tasks(ui, &snapshot);
            });
        self.dialogs(&ctx, &snapshot);
    }
}

impl Drawer {
    fn window_ui(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        egui::Frame::new().fill(ui.visuals().panel_fill).show(ui, |ui| {
            egui::Panel::bottom("window-status").frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 8))).show(ui, |ui| self.footer(ui, snapshot));
            egui::Panel::top("window-toolbar").frame(egui::Frame::new().fill(ui.visuals().window_fill).inner_margin(egui::Margin::symmetric(20, 8))).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Image::new(&self.icon).fit_to_exact_size(Vec2::splat(28.)));
                    ui.label(RichText::new("安装工作台").size(18.).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if window_mode_button(ui, true).clicked() { self.set_window_mode(ui.ctx(), false); }
                        if pin_button(ui, self.preferences.pinned, true).clicked() { self.toggle_pin(ui.ctx()); }
                        ui.separator();
                        if ui.button("设置").clicked() { self.dialog = Some(Dialog::Settings); }
                        if ui.ctx().content_rect().width() < 760. && ui.button("设备管理").clicked() { self.dialog = Some(Dialog::Devices); }
                    });
                });
            });
            let sidebar = ui.available_width() >= 760.;
            if sidebar {
                egui::Panel::left("window-devices").default_size(272.).min_size(228.).max_size(360.).resizable(true)
                    .frame(egui::Frame::new().fill(ui.visuals().window_fill).inner_margin(16)).show(ui, |ui| {
                        egui::Panel::bottom("device-sidebar-footer").frame(egui::Frame::NONE).show(ui, |ui| {
                            ui.separator();
                            ui.label(RichText::new("单击选择，可同时选择多台\n离线无线设备在安装前自动重连").small().color(secondary_text(ui)));
                            if ui.add_sized([ui.available_width(), 36.], egui::Button::new("设备管理")).clicked() { self.dialog = Some(Dialog::Devices); }
                        });
                        self.device_rows(ui, snapshot, true);
                    });
            }
            egui::Frame::new().inner_margin(20).show(ui, |ui| {
                let reserved = (ui.available_height() * 0.4).max(120.);
                egui::ScrollArea::vertical().id_salt("window-preparation").max_height((ui.available_height() - reserved).max(0.)).min_scrolled_height(0.).auto_shrink([false, true]).show(ui, |ui| {
                    if !sidebar {
                        let selected = snapshot.devices.iter().filter(|d| d.selected).count();
                        egui::CollapsingHeader::new(format!("选择设备 · 已选 {selected} 台")).id_salt("window-device-selector").show(ui, |ui| self.device_rows(ui, snapshot, false));
                        ui.add_space(8.);
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("安装包").size(18.).strong());
                        let selected = snapshot.devices.iter().filter(|d| d.selected).count();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(if selected == 0 { "请先选择目标设备".into() } else { format!("安装目标：{selected} 台设备") }).small().color(secondary_text(ui)));
                        });
                    });
                    ui.add_space(8.);
                    self.drop_zone(ui, snapshot);
                });
                ui.add_space(20.);
                self.installation_tasks(ui, snapshot);
            });
        });
    }

    fn preparation_ui(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot, height: f32) {
        egui::ScrollArea::vertical()
            .id_salt("drawer-preparation")
            .max_height(height)
            .min_scrolled_height(0.)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                self.device_rows(ui, snapshot, false);
                ui.add_space(6.);
                self.drop_zone(ui, snapshot);
            });
    }

    fn installation_tasks(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let table = self.preferences.windowed && ui.available_width() >= 540.;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("安装任务")
                    .size(if self.preferences.windowed { 18. } else { 15. })
                    .strong(),
            );
            let count = snapshot.jobs.iter().filter(|j| j.stage.active()).count();
            if count > 0 {
                ui.label(
                    RichText::new(format!("{count} 项进行中"))
                        .small()
                        .color(positive_text(ui)),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("清除已结束").frame(false))
                    .clicked()
                {
                    self.backend.clear_finished();
                }
            });
        });
        if table {
            egui::Frame::new()
                .fill(ui.visuals().window_fill)
                .inner_margin(10)
                .corner_radius(6)
                .show(ui, |ui| {
                    let widths =
                        task_column_widths(ui.available_width(), ui.spacing().item_spacing.x);
                    ui.horizontal(|ui| {
                        for (width, label) in
                            widths
                                .into_iter()
                                .zip(["安装包", "目标设备", "状态", "操作"])
                        {
                            task_cell(ui, width, |ui| {
                                ui.label(RichText::new(label).small().color(secondary_text(ui)));
                            });
                        }
                    });
                });
        }
        let remaining = ui.available_height().max(0.);
        egui::ScrollArea::vertical()
            .id_salt("jobs")
            .max_height(remaining)
            .min_scrolled_height(0.)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if snapshot.jobs.is_empty() {
                    let text_height = ui.text_style_height(&egui::TextStyle::Small);
                    ui.add_space(
                        ((remaining
                            - if self.preferences.windowed {
                                72.
                            } else {
                                text_height
                            })
                            / 2.)
                            .max(0.),
                    );
                    ui.vertical_centered(|ui| {
                        if self.preferences.windowed {
                            ui.label(RichText::new("暂无安装任务").size(16.).strong());
                        }
                        ui.label(
                            RichText::new(if self.preferences.windowed {
                                "添加 APK 并点击开始安装，进度和结果会显示在这里"
                            } else {
                                "安装结果会显示在这里"
                            })
                            .small()
                            .color(secondary_text(ui)),
                        );
                    });
                }
                for job in &snapshot.jobs {
                    self.job_row(ui, job, table);
                }
            });
    }
}

fn connect_dialog() -> Dialog {
    Dialog::Connect {
        mode: 0,
        host: String::new(),
        port: String::new(),
        pairing_port: String::new(),
        code: String::new(),
        paired_id: String::new(),
    }
}
fn device_summary(ui: &mut egui::Ui, snapshot: &Snapshot) {
    let online = snapshot
        .devices
        .iter()
        .filter(|d| d.status == DeviceStatus::Online)
        .count();
    ui.label(
        RichText::new(format!("● {online} 台在线"))
            .small()
            .color(if online > 0 {
                positive_text(ui)
            } else {
                secondary_text(ui)
            }),
    );
}

fn apk_symbol(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(38., 40.), egui::Sense::hover());
    let paper = rect.shrink(5.);
    ui.painter().rect(
        paper,
        4.,
        GREEN.gamma_multiply(0.09),
        Stroke::new(1.5, GREEN),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        paper.center(),
        egui::Align2::CENTER_CENTER,
        "APK",
        FontId::proportional(10.),
        GREEN,
    );
}

fn task_column_widths(width: f32, spacing: f32) -> [f32; 4] {
    let usable = (width - spacing * 3.).max(0.);
    [usable * 0.4, usable * 0.24, usable * 0.24, usable * 0.12]
}

fn task_cell(ui: &mut egui::Ui, width: f32, content: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, 28.),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_width(width);
            ui.set_min_height(28.);
            content(ui);
        },
    );
}

fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE))
        .fill(PRIMARY_GREEN)
        .min_size(Vec2::new(128., 40.))
}
fn window_level(settings: &Settings) -> WindowLevel {
    if settings.pinned || settings.topmost {
        WindowLevel::AlwaysOnTop
    } else {
        WindowLevel::Normal
    }
}
fn window_mode_button(ui: &mut egui::Ui, windowed: bool) -> egui::Response {
    let label = if windowed {
        "切换抽屉模式"
    } else {
        "切换窗口模式"
    };
    let response = ui.add_sized([32., 32.], egui::Button::new("").selected(windowed));
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), windowed, label)
    });
    let color = ui.visuals().text_color();
    let origin = response.rect.center() - Vec2::splat(9.);
    let rect = egui::Rect::from_min_size(origin, Vec2::new(18., 16.));
    ui.painter()
        .rect_stroke(rect, 2., Stroke::new(1.5, color), egui::StrokeKind::Inside);
    ui.painter().line_segment(
        [origin + Vec2::new(1., 5.), origin + Vec2::new(17., 5.)],
        Stroke::new(1.5, color),
    );
    if windowed {
        ui.painter().line_segment(
            [origin + Vec2::new(12., 6.), origin + Vec2::new(12., 15.)],
            Stroke::new(1.5, color),
        );
    }
    response.on_hover_text(label)
}

fn pin_button(ui: &mut egui::Ui, pinned: bool, windowed: bool) -> egui::Response {
    let label = if pinned {
        "取消固定"
    } else if windowed {
        "固定窗口"
    } else {
        "固定抽屉"
    };
    let response = ui.add_sized([32., 32.], egui::Button::new("").selected(pinned));
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), pinned, label)
    });
    let color = if pinned {
        positive_text(ui)
    } else {
        ui.visuals().text_color()
    };
    let origin = response.rect.center() - Vec2::splat(10.);
    let shape = [
        (6., 3.),
        (14., 3.),
        (13., 10.),
        (16., 13.),
        (4., 13.),
        (7., 10.),
    ]
    .into_iter()
    .map(|(x, y)| origin + Vec2::new(x, y))
    .collect();
    ui.painter().add(egui::epaint::PathShape {
        points: shape,
        closed: true,
        fill: if pinned { color } else { Color32::TRANSPARENT },
        stroke: Stroke::new(1.5, color).into(),
    });
    ui.painter().line_segment(
        [origin + Vec2::new(10., 13.), origin + Vec2::new(10., 19.)],
        Stroke::new(1.5, color),
    );
    response.on_hover_text(if pinned && windowed {
        "取消置顶，窗口保持打开"
    } else if pinned {
        "取消固定，点击外部时自动收起"
    } else {
        "固定并置顶，点击外部时保持展开"
    })
}
fn modal_heading(ui: &mut egui::Ui, title: &str, close: &mut bool) {
    ui.horizontal(|ui| {
        ui.heading(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("×").clicked() {
                *close = true;
            }
        });
    });
    ui.add_space(8.);
}
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, password: bool, hint: &str) {
    let label = ui.label(RichText::new(label).strong());
    ui.add_sized(
        [ui.available_width(), 38.],
        egui::TextEdit::singleline(value)
            .password(password)
            .hint_text(hint)
            .margin(Vec2::new(10., 9.))
            .background_color(if ui.visuals().dark_mode {
                Color32::from_rgb(24, 29, 34)
            } else {
                Color32::from_rgb(247, 249, 250)
            })
            .desired_width(f32::INFINITY),
    )
    .labelled_by(label.id);
}
fn secondary_text(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(163, 175, 187)
    } else {
        Color32::from_rgb(99, 111, 123)
    }
}
fn positive_text(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        GREEN
    } else {
        PRIMARY_GREEN
    }
}
fn card(ui: &mut egui::Ui, dark: bool, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(if dark {
            Color32::from_rgb(32, 38, 44)
        } else {
            Color32::WHITE
        })
        .corner_radius(9)
        .stroke(Stroke::new(
            1.,
            ui.visuals()
                .widgets
                .noninteractive
                .bg_stroke
                .color
                .gamma_multiply(0.6),
        ))
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}
fn bytes(value: f64) -> String {
    if value >= 1024. * 1024. {
        format!("{:.1} MB", value / (1024. * 1024.))
    } else {
        format!("{:.0} KB", value / 1024.)
    }
}
fn error_help(error: &str) -> &'static str {
    if error.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") {
        "签名与已安装应用不同，需要确认应用来源；不会自动卸载旧应用。"
    } else if error.contains("INSTALL_FAILED_VERSION_DOWNGRADE") {
        "APK 版本低于设备上已有版本。"
    } else if error.contains("INSTALL_FAILED_INSUFFICIENT_STORAGE") {
        "手机存储空间不足。"
    } else if error.contains("INSTALL_FAILED_TEST_ONLY") {
        "此 APK 为测试包，可在设置中允许 testOnly APK 后重试。"
    } else if error.contains("INSTALL_FAILED_NO_MATCHING_ABIS") {
        "APK 不支持这台设备的 CPU 架构。"
    } else if error.contains("INSTALL_FAILED_OLDER_SDK") {
        "手机 Android 版本低于 APK 要求。"
    } else {
        "以下为设备返回的原始结果："
    }
}
