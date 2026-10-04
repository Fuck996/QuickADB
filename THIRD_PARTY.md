# 第三方来源与许可

QuickADB 的原创部分采用根目录 [LICENSE](LICENSE) 与 [NOTICE](NOTICE) 规定的许可。以下第三方材料保留原作者版权及各自许可，根目录 GPL 声明不替代其原始授权。

| 组件 / 素材 | 来源 | 许可与用途 |
| --- | --- | --- |
| DroidMux | [poapoauu/DroidMux](https://github.com/poapoauu/DroidMux)，固定提交 `24b17e5fe83bc68f509215ab2cfeaa661005e852` | MIT OR Apache-2.0；ADB、USB、无线配对、发现和 shell。原始许可随 `vendor/droidmux` 保留，本地修改见 [vendor/README.md](vendor/README.md)。 |
| libusb 1.0.27 / libusb1-sys 0.7.0 / rusb 0.9.4 | [libusb](https://github.com/libusb/libusb)、[libusb1-sys](https://github.com/a1ien/libusb1-sys)、[rusb](https://github.com/a1ien/rusb) | libusb 为 LGPL-2.1-or-later，Rust 封装为 MIT。静态链接，发布源码包包括库源码与重新构建所需材料。 |
| egui / eframe 0.36.2 | [emilk/egui](https://github.com/emilk/egui) | MIT OR Apache-2.0；原生界面。 |
| AccessKit | [AccessKit/accesskit](https://github.com/AccessKit/accesskit) | MIT OR Apache-2.0；部分适配器为 Apache-2.0；无障碍支持。 |
| winit / tray-icon / rfd | [winit](https://github.com/rust-windowing/winit)、[tray-icon](https://github.com/tauri-apps/tray-icon)、[rfd](https://github.com/PolyMeilex/rfd) | 保留各版本的 MIT / Apache-2.0 等许可；窗口、托盘及文件选择器。 |
| 默认字体 | [epaint_default_fonts](https://github.com/emilk/egui/tree/main/crates/epaint_default_fonts) | 除代码许可外还包含 OFL-1.1、Ubuntu-font-1.0；字体许可全文在第三方声明与源码包中。 |
| 其他 Rust 运行及构建依赖 | [Cargo.lock](Cargo.lock) 与 [完整第三方声明](assets/ThirdPartyNotices.txt) | 依赖名、版本、来源及许可全文逐项保留；不以本表代替完整清单。 |

`assets/ThirdPartyNotices.txt` 由 `scripts/Prepare-Notices.ps1` 按锁定的 Windows x64 依赖生成，当前清单还包括构建与测试使用的依赖。旧清单中缺少的许可文本从对应 crate 发布记录中的官方 Git 提交补齐；[registry-notices.json](assets/licenses/registry-notices.json) 记录提交、原文地址、文件位置与 SHA-256。

0.9.6 原 EXE 内嵌的是发布时的旧清单；本次补齐的完整清单随 Release 单独提供，原 EXE 的文件内容和校验值不变。后续构建会内嵌补齐后的版本。

未启用 DroidMux 的投屏或控制功能，也未把其可选 scrcpy server 作为 QuickADB 的运行资源。SDK 仓库中保留的其他上游材料仍按它们自己的说明处理。

应用图标沿用项目已有 `assets/AppIcon.png`，托盘图标使用项目内 imagegen 生成素材；两者在项目内裁剪、导出，不把第三方库标识用作应用图标。生成来源不等于独占版权证明。
