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

const WIDTH: f32 = 440.;
const HEIGHT: f32 = 660.;
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
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("QuickADB")
        .with_inner_size([WIDTH, HEIGHT])
        .with_decorations(false)
        .with_resizable(false)
        .with_drag_and_drop(true)
        .with_taskbar(false)
        .with_visible(!start_in_tray)
        .with_active(!start_in_tray)
        .with_icon(Arc::new(data));
    if settings.topmost {
        viewport = viewport.with_window_level(WindowLevel::AlwaysOnTop);
    }
    let area = platform::monitor_work_area();
    let position = settings.window_position.unwrap_or([
        area.right as f32 - WIDTH - 24.,
        area.bottom as f32 - HEIGHT - 24.,
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
        }) {
            self.backend.notify(&format!("设置保存失败：{error:#}"));
        }
    }

    fn show(&mut self, ctx: &egui::Context, anchor: Option<[f32; 2]>) {
        if let Some([x, y]) = anchor
            && !self.preferences.pinned
        {
            let dpi = ctx.input(|i| i.viewport().native_pixels_per_point.unwrap_or(1.));
            let area = platform::monitor_work_area();
            let height = if self.collapsed { 64. } else { HEIGHT };
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
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn hide(&mut self, ctx: &egui::Context) {
        self.hidden = true;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
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
                if ui
                    .add(egui::Button::new("×").frame(false))
                    .on_hover_text("收回托盘，安装继续")
                    .clicked()
                {
                    self.hide(ui.ctx());
                }
                if ui
                    .add(egui::Button::new("−").frame(false))
                    .on_hover_text("收成窄栏")
                    .clicked()
                {
                    self.collapsed = true;
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(WIDTH, 64.)));
                }
                if pin_button(ui, self.preferences.pinned).clicked() {
                    self.preferences.pinned = !self.preferences.pinned;
                    self.persist();
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

    fn device_rows(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let selected = snapshot.devices.iter().filter(|d| d.selected).count();
        ui.horizontal(|ui| {
            ui.label(RichText::new("安装设备").strong());
            if selected > 0 {
                ui.label(
                    RichText::new(format!("已选 {selected} 台"))
                        .small()
                        .color(positive_text(ui)),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("连接设备").frame(false)).clicked() {
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
                .max_height(176.)
                .show(ui, |ui| {
                    for device in &snapshot.devices {
                        ui.horizontal(|ui| {
                            let online = device.status == DeviceStatus::Online;
                            let row_width = ui.available_width() - 44.;
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
                            if ui
                                .add_enabled_ui(online, |ui| {
                                    frame
                                        .show(ui, |ui| {
                                            ui.set_width(row_width - 20.);
                                            ui.set_min_height(42.);
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
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&device.name).size(14.),
                                                        )
                                                        .truncate(),
                                                    )
                                                    .on_hover_text(&device.name);
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(status)
                                                                .small()
                                                                .color(secondary_text(ui)),
                                                        )
                                                        .truncate(),
                                                    );
                                                });
                                            });
                                        })
                                        .response
                                        .interact(egui::Sense::click())
                                })
                                .inner
                                .on_hover_text(if online {
                                    "点击选择，再次点击取消；可同时选择多台"
                                } else {
                                    &device.detail
                                })
                                .clicked()
                            {
                                self.backend.toggle(&device.id);
                            }
                            if ui
                                .add(egui::Button::new("详情").small().frame(false))
                                .on_hover_text("设备详情与连接操作")
                                .clicked()
                            {
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
                        });
                    }
                });
        }
    }

    fn drop_zone(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let hovering = self.drag_hovering;
        let selected = snapshot
            .devices
            .iter()
            .filter(|d| d.selected && d.status == DeviceStatus::Online)
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
                        .max_height(88.)
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
                                        .truncate(),
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
                    ui.vertical_centered(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(38., 40.), egui::Sense::hover());
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
                ui.add_space(8.);
                ui.columns(2, |columns| {
                    let first_width = columns[0].available_width();
                    if columns[0]
                        .add_sized(
                            [first_width, 34.],
                            egui::Button::new(if snapshot.preparation.is_some() {
                                "更换 APK"
                            } else {
                                "选择 APK"
                            }),
                        )
                        .clicked()
                    {
                        self.file_picker = Some(false);
                        columns[0].ctx().request_repaint();
                    }
                    let second_width = columns[1].available_width();
                    if columns[1]
                        .add_sized([second_width, 34.], egui::Button::new("选择拆分 APK"))
                        .clicked()
                    {
                        self.file_picker = Some(true);
                        columns[1].ctx().request_repaint();
                    }
                });
                if snapshot.preparation.is_none() {
                    return;
                }
                ui.add_space(8.);
                let ready = snapshot
                    .preparation
                    .as_ref()
                    .is_some_and(|p| matches!(p.state, PreparationState::Ready(_)));
                let label = if selected > 0 {
                    format!("安装到 {selected} 台设备")
                } else {
                    "安装 · 请先选择设备".into()
                };
                let clicked = ui
                    .add_enabled_ui(ready && selected > 0, |ui| {
                        ui.add_sized([ui.available_width(), 40.], primary_button(&label))
                    })
                    .inner
                    .clicked();
                if clicked && let Some(preparation) = &snapshot.preparation {
                    self.backend
                        .install_prepared(preparation.revision, self.preferences.test_packages);
                    self.opened = Instant::now();
                }
            });
    }

    fn footer(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        if !snapshot.notice.is_empty() {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 32.).max(1.);
                egui::ScrollArea::vertical()
                    .id_salt("notice")
                    .max_height(52.)
                    .min_scrolled_height(0.)
                    .max_width(width)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&snapshot.notice)
                                    .small()
                                    .color(ui.visuals().warn_fg_color),
                            )
                            .wrap(),
                        );
                    });
                if ui.small_button("×").on_hover_text("关闭提示").clicked() {
                    self.backend.clear_notice();
                }
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
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

    fn job_row(&mut self, ui: &mut egui::Ui, job: &Job) {
        card(ui, self.preferences.dark, |ui| {
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
                    if matches!(
                        job.stage,
                        JobStage::Queued | JobStage::Preparing | JobStage::Transferring
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
                });
            });
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
            } else {
                ui.horizontal(|ui| {
                    if matches!(job.stage, JobStage::Preparing | JobStage::Installing) {
                        ui.add(egui::Spinner::new().size(12.));
                    }
                    let color = match job.stage {
                        JobStage::Succeeded => positive_text(ui),
                        JobStage::Failed | JobStage::Unknown => {
                            if self.preferences.dark {
                                Color32::from_rgb(214, 105, 82)
                            } else {
                                Color32::from_rgb(181, 76, 54)
                            }
                        }
                        _ => secondary_text(ui),
                    };
                    ui.label(RichText::new(job.stage.label()).small().color(color));
                });
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
        let discovered: Vec<_> = snapshot
            .discovered
            .iter()
            .filter(|device| match *mode {
                0 => device.service_type == AdbServiceType::Pairing,
                1 => device.paired_id().is_some_and(|id| {
                    self.storage
                        .paired_devices()
                        .iter()
                        .any(|d| d.device_id == id)
                }),
                _ => device.service_type == AdbServiceType::Legacy,
            })
            .collect();
        if !discovered.is_empty() {
            egui::CollapsingHeader::new("局域网发现的设备").show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(128.).show(ui, |ui| {
                    for device in discovered {
                        let endpoint = &device.endpoint;
                        let known_name = snapshot.devices.iter().find_map(|known| {
                            known.endpoint.as_ref().filter(|address| address.host == endpoint.host && address.port == endpoint.port).map(|_| known.name.as_str())
                        });
                        let name = known_name.unwrap_or(&device.name);
                        let kind = match device.service_type {
                            AdbServiceType::Pairing => "配对服务 · 填入配对端口",
                            AdbServiceType::TlsConnect => "无线连接 · 自动获取端口",
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
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("填入{name} · {kind}")));
                        let response = response.on_hover_text(if device.model_advertised || known_name.is_some() {
                            device.fullname.as_str()
                        } else {
                            "设备没有广播型号，当前显示其真实服务名称。连接授权后可读取型号并设置备注。"
                        });
                        if response.clicked() {
                            if *host != endpoint.host {
                                pairing_port.clear();
                                port.clear();
                                code.clear();
                            }
                            *host = endpoint.host.clone();
                            if device.service_type == AdbServiceType::Pairing {
                                *pairing_port = endpoint.port.to_string();
                            } else if let Some(id) = device.paired_id() {
                                *paired_id = id.into();
                                port.clear();
                            } else {
                                *port = endpoint.port.to_string();
                            }
                        }
                    }
                });
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
        let dialog_height = (ctx.content_rect().height() - 64.).max(120.);
        let response = egui::Modal::new(egui::Id::new("drawer-dialog")).frame(egui::Frame::popup(&ctx.global_style()).corner_radius(14).inner_margin(16)).show(ctx, |ui| {
            ui.set_width(368.);
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
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(WIDTH, HEIGHT)));
            }
            self.show(ctx, None);
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if self.hidden && platform::window_visible() {
            self.show(ctx, None);
        }
        if !self.position_checked
            && let Some(position) =
                ctx.input(|i| i.viewport().outer_rect.map(|r| [r.min.x, r.min.y]))
        {
            let dpi = ctx.input(|i| i.viewport().native_pixels_per_point.unwrap_or(1.));
            match platform::clamp_position(position, [WIDTH, HEIGHT], dpi) {
                Ok(position) => {
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position.into()))
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
                && self.dialog.is_none()
                && !self.drag_hovering
                && !platform::pointer_button_down()
                && self.opened.elapsed() > Duration::from_millis(600)
            {
                self.hide(ctx);
            }
            if self.preferences.pinned
                && let Some(position) =
                    ctx.input(|i| i.viewport().outer_rect.map(|r| [r.min.x, r.min.y]))
            {
                if self.last_position != Some(position) {
                    self.last_position = Some(position);
                    self.position_changed = Instant::now();
                } else if self.position_changed.elapsed() > Duration::from_millis(700)
                    && self.preferences.window_position != Some(position)
                {
                    self.preferences.window_position = Some(position);
                    self.persist();
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
                egui::ScrollArea::vertical()
                    .id_salt("drawer-preparation")
                    .max_height((ui.available_height() - task_height).max(0.))
                    .min_scrolled_height(0.)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        self.device_rows(ui, &snapshot);
                        ui.add_space(6.);
                        self.drop_zone(ui, &snapshot);
                    });
                ui.add_space(6.);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("安装任务").strong());
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
                let remaining = ui.available_height().max(0.);
                egui::ScrollArea::vertical()
                    .id_salt("jobs")
                    .max_height(remaining)
                    .min_scrolled_height(0.)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if snapshot.jobs.is_empty() {
                            let text_height = ui.text_style_height(&egui::TextStyle::Small);
                            ui.add_space(((remaining - text_height) / 2.).max(0.));
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("安装结果会显示在这里")
                                        .small()
                                        .color(secondary_text(ui)),
                                );
                            });
                        }
                        for job in &snapshot.jobs {
                            self.job_row(ui, job);
                        }
                    });
            });
        self.dialogs(&ctx, &snapshot);
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
fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE))
        .fill(PRIMARY_GREEN)
        .min_size(Vec2::new(128., 40.))
}
fn pin_button(ui: &mut egui::Ui, pinned: bool) -> egui::Response {
    let label = if pinned {
        "取消固定"
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
    response.on_hover_text(if pinned {
        "取消固定，点击外部时自动收起"
    } else {
        "固定抽屉，点击外部时保持展开"
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
