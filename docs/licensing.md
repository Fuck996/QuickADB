# 许可与来源

QuickADB 的原创部分采用 **GNU GPL 第 3 版（GPL-3.0-only）**。正式条款为根目录 [LICENSE](../LICENSE) 的未修改英文原文，以及 [NOTICE](../NOTICE) 中依据 GPL 第 7(b) 条规定的合理来源保留条款。本页是中文说明，不代替正式条款。

## 使用与分发

- 可以个人使用、企业内部使用、修改和商用，也可以收费提供副本或服务。
- 对外分发 QuickADB 或受 GPL 覆盖的修改、整合版本时，必须按 GPLv3 授权整个受覆盖作品，并向接收者提供完整对应源码及必要构建说明；不能只发闭源 EXE，也不能通过额外协议剥夺接收者的 GPL 权利。
- 保留项目名称、版权声明、原项目地址、许可全文及第三方声明。来源可放在随附的 NOTICE 或许可文档中，不要求强制广告或指定界面位置。
- 分发修改版时，明确标注修改内容及日期，不冒充未修改的官方版本。
- 未对外分发的内部修改不要求公开。GPL 也不要求无关、独立的软件使用同一许可；用 QuickADB 安装或测试的 APK 不会仅因此受到 GPL 约束。
- 软件按现状提供，保修与责任限制以 GPL 第 15、16 条及适用法律为准。

这里的“不能闭源商用”指**不能将受 GPL 覆盖的衍生产品闭源分发**。不是禁止商业使用，也不是要求所有企业内部代码公开。禁止任何商用的附加条款不符合标准开源定义，因此本项目没有加入此类条款。

## 原创范围

QuickADB 的应用界面、设备管理、安装队列、配置持久化和托盘交互为本项目编写；ADB 协议、无线配对、USB、图形界面等依赖第三方库。项目不是所有组件都从零实现。第三方代码的作者、许可和来源不归 QuickADB 所有，详见 [第三方来源](../THIRD_PARTY.md)。

项目图标由项目工作流提供和处理：应用图标使用既有 `assets/AppIcon.png`，托盘图标通过 imagegen 生成。该记录说明素材来源，不构成对生成素材独占版权或所有权的法律保证。项目许可只授权贡献者有权授权的部分。

## 对应源码

每次二进制发布应同时提供完整对应源码，包含应用源码、锁定依赖、内置资源、第三方许可和构建脚本。Windows 系统库、Rust 编译器、Visual Studio C++ Build Tools 与 Windows SDK 等通用构建工具另行安装。

当前 0.9.6 的 EXE 不变；许可与来源说明于 2026-10-04 补齐。对应源码包记录原 EXE 的提交及 SHA-256，应用代码与该版本一致，仅增加许可声明和发布辅助文件。根目录许可也适用于维护者在当前 Release 中发布的 0.9.6 原创部分；第三方部分仍遵守各自许可。

源码包解压后，在 Windows x64、PowerShell 7、Rust 1.95+ 和 C++ Build Tools 环境运行：

```powershell
./scripts/Build-Source.ps1 -Release
```

此脚本使用源码包内的 `vendor/registry` 离线构建，并静态链接 C 运行库与 libusb。要修改并重新链接某个 Cargo 依赖，按 [Cargo 的建议](https://doc.rust-lang.org/cargo/commands/cargo-vendor.html)，把该依赖复制到一个可编辑目录，再在 Cargo.toml 的 `[patch.crates-io]` 中指定该目录；保留其许可和修改说明。不要直接更改带校验清单的只读依赖目录后跳过校验。

维护者使用 `scripts/Publish-Source.ps1` 从已提交的版本生成源码包。发布 EXE 时，同时在下载处提供源码包和法律声明；不能只链接到可能变化的依赖主页。

## 官方依据

- [GPLv3 原文](https://www.gnu.org/licenses/gpl-3.0.html)：第 4、5、6 条规定分发与源码义务，第 7(b) 条允许合理的来源保留要求。
- [GNU 常见问题：是否必须公开源码](https://www.gnu.org/licenses/gpl-faq.en.html#GPLRequireSourcePostedPublic)：未分发的私人或内部修改无需公开。
- [开源定义](https://opensource.org/osd)：允许销售和商业领域使用。
- [LGPL 2.1 原文](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html)：静态链接 libusb 时，随分发提供库源码及可重新链接的应用源码等材料。
