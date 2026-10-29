// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only
// 来源保留条款见项目根目录 NOTICE（GPL 第 7(b) 条）。

mod support;

mod application {
    include!("../src/native/ui.rs");

    use eframe::glow::HasContext;
    use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};
    use winit::platform::windows::EventLoopBuilderExtWindows;

    #[derive(Default, Debug)]
    pub struct Observations {
        pub frames: BTreeMap<u8, usize>,
        pub blank_frames: usize,
        pub gl_errors: Vec<u32>,
        pub completed: usize,
        pub transferred: u64,
        pub hidden_observed: bool,
        pub restored_observed: bool,
    }

    struct RefreshTest {
        drawer: Drawer,
        apk: PathBuf,
        submitted: bool,
        started: Instant,
        phase: u8,
        observations: Arc<Mutex<Observations>>,
    }

    impl eframe::App for RefreshTest {
        fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.drawer.logic(ctx, frame);
            let snapshot = self.drawer.backend.snapshot();
            if snapshot
                .devices
                .iter()
                .filter(|d| d.status == DeviceStatus::Online)
                .count()
                == 2
                && snapshot.preparation.is_none()
            {
                self.drawer.backend.select_all(true);
                self.drawer.backend.prepare(vec![self.apk.clone()], false);
            }
            if !self.submitted
                && let Some(preparation) = &snapshot.preparation
                && matches!(preparation.state, PreparationState::Ready(_))
            {
                self.drawer
                    .backend
                    .install_prepared(preparation.revision, false);
                self.submitted = true;
            }
            let phase = (self.started.elapsed().as_secs() / 3).min(7) as u8;
            if phase == self.phase
                && let Some(window) = frame.winit_window()
            {
                let mut observations = self.observations.lock().expect("observations");
                if phase == 4 && window.is_visible() == Some(false) {
                    observations.hidden_observed = true;
                }
                if phase == 5 && window.is_visible() == Some(true) {
                    observations.restored_observed = true;
                }
            }
            if phase != self.phase {
                self.phase = phase;
                match phase {
                    1 => ctx.send_viewport_cmd(ViewportCommand::InnerSize([520., 700.].into())),
                    2 => self.drawer.set_window_mode(ctx, false),
                    3 => self.drawer.set_window_mode(ctx, true),
                    4 => ctx.send_viewport_cmd(ViewportCommand::Visible(false)),
                    5 => ctx.send_viewport_cmd(ViewportCommand::Visible(true)),
                    6 => {
                        self.drawer.preferences.dark = true;
                        self.drawer.apply_style(ctx);
                    }
                    7 => {
                        self.drawer.preferences.dark = false;
                        self.drawer.apply_style(ctx);
                    }
                    _ => {}
                }
            }
            if self.started.elapsed() > Duration::from_secs(24) {
                let mut observations = self.observations.lock().expect("observations");
                observations.completed = snapshot
                    .jobs
                    .iter()
                    .filter(|j| j.stage == JobStage::Succeeded)
                    .count();
                observations.transferred = snapshot.jobs.iter().map(|j| j.transferred).sum();
                self.drawer.exiting = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }

        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            self.drawer.ui(ui, frame);
            let observations = self.observations.clone();
            let phase = self.phase;
            ui.painter().add(egui::PaintCallback {
                rect: ui.ctx().viewport_rect(),
                callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, painter| {
                    let gl = painter.gl();
                    let mut black = 0;
                    for (x, y) in [(0.5, 0.5), (0.5, 0.95), (0.95, 0.05)] {
                        let mut pixel = [0; 4];
                        unsafe {
                            gl.read_pixels(
                                (info.screen_size_px[0] as f32 * x) as i32,
                                (info.screen_size_px[1] as f32 * y) as i32,
                                1,
                                1,
                                eframe::glow::RGBA,
                                eframe::glow::UNSIGNED_BYTE,
                                eframe::glow::PixelPackData::Slice(Some(&mut pixel)),
                            );
                        }
                        black += usize::from(pixel[..3].iter().all(|&c| c <= 16));
                    }
                    let error = unsafe { gl.get_error() };
                    let mut observations = observations.lock().expect("observations");
                    *observations.frames.entry(phase).or_default() += 1;
                    observations.blank_frames += usize::from(black == 3);
                    if error != eframe::glow::NO_ERROR {
                        observations.gl_errors.push(error);
                    }
                })),
            });
            if self.phase == 5 {
                ui.ctx().show_viewport_deferred(
                    egui::ViewportId::from_hash_of("refresh-test-child"),
                    egui::ViewportBuilder::default()
                        .with_title("QuickADB 子窗口绘制验证")
                        .with_inner_size([260., 160.]),
                    |ui, _| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            ui.label("验证切换窗口后仍绘制到正确表面");
                        });
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    },
                );
            }
        }
    }

    pub fn check(
        storage: Arc<Storage>,
        backend: Backend,
        apk: PathBuf,
        observations: Arc<Mutex<Observations>>,
    ) {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("QuickADB 刷新回归（隔离）")
                .with_inner_size([1080., 819.])
                .with_position([200., 120.]),
            renderer: eframe::Renderer::Glow,
            event_loop_builder: Some(Box::new(|builder| {
                builder.with_any_thread(true);
            })),
            ..Default::default()
        };
        eframe::run_native(
            "QuickADB 刷新回归",
            options,
            Box::new(move |cc| {
                configure_fonts(&cc.egui_ctx)?;
                Ok(Box::new(RefreshTest {
                    drawer: Drawer::new(cc, storage, backend)?,
                    apk,
                    submitted: false,
                    started: Instant::now(),
                    phase: 0,
                    observations,
                }))
            }),
        )
        .expect("native refresh regression");
    }
}

#[test]
#[ignore = "需要 Windows 桌面与 OpenGL；仅使用本机协议夹具"]
fn install_refresh_preserves_content_across_window_lifecycle() {
    use quickadb::{engine::Backend, model::Endpoint, storage::Storage};
    use std::{
        io::Write,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use support::{DeviceOptions, DeviceServer, TempDirectory};

    let directory = TempDirectory::new();
    let apk = directory.0.join("刷新回归.apk");
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&apk).expect("APK"));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive
        .start_file("AndroidManifest.xml", options)
        .expect("manifest entry");
    archive
        .write_all(&support::binary_manifest("test.refresh", "", 1))
        .expect("manifest");
    archive
        .start_file("payload", options)
        .expect("payload entry");
    archive
        .write_all(&vec![7; 8 * 1024 * 1024])
        .expect("payload");
    archive.finish().expect("APK finish");
    let size = std::fs::metadata(&apk).expect("APK metadata").len();
    let servers: Vec<_> = ["REFRESH-A", "REFRESH-B"]
        .into_iter()
        .map(|serial| {
            DeviceServer::start(DeviceOptions {
                serial,
                features: "shell_v2,cmd,abb_exec",
                max_payload: 64 * 1024,
                delay_ack: Duration::from_millis(35),
                idle_timeout: Duration::from_secs(35),
                ..Default::default()
            })
        })
        .collect();
    let storage = Storage::open(directory.0.join("data")).expect("isolated storage");
    storage
        .update_settings(|s| {
            s.windowed = true;
            s.pinned = true;
            s.dark = false;
            s.endpoints = servers
                .iter()
                .map(|server| Endpoint {
                    host: "127.0.0.1".into(),
                    port: server.port,
                    paired_id: None,
                })
                .collect();
        })
        .expect("isolated settings");
    let backend = Backend::new(storage.clone()).expect("backend");
    let observations = Arc::new(Mutex::new(application::Observations::default()));
    application::check(storage, backend, apk, observations.clone());
    let observations = observations.lock().expect("observations");
    println!("{observations:?}");
    assert_eq!(observations.completed, 2);
    assert_eq!(observations.transferred, size * 2);
    assert_eq!(observations.blank_frames, 0);
    assert!(observations.gl_errors.is_empty());
    assert!(observations.hidden_observed);
    assert!(observations.restored_observed);
    for phase in [0, 1, 2, 3, 5, 6, 7] {
        assert!(
            observations
                .frames
                .get(&phase)
                .is_some_and(|&frames| frames >= 10),
            "unobserved phase {phase}"
        );
    }
    for server in &servers {
        assert_eq!(
            server.records.lock().expect("device records").uploads.len(),
            1
        );
    }
}
