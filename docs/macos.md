# QuickADB for macOS

首版源码面向 Apple 芯片 Mac 和 macOS 27。SwiftUI 和 AppKit 提供原生窗口、侧栏、工具栏、菜单栏抽屉与设置。工具栏和抽屉控制使用系统 Liquid Glass，APK 与任务列表沿用系统文字、对比度和主题。应用使用项目现有的黑色图标。菜单栏图标从 App 图标提炼出 Android 头像、APK 折角文件和速度线，并随系统外观自动着色。 [Apple 的 Liquid Glass 指南](https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass)

主窗口和菜单栏抽屉共用 Rust ADB 后端、设备选择与任务队列。支持 USB、局域网无线发现、配对、传统 TCP、单包、批量与拆分 APK。拖入或选取后先检查 APK，点击安装时才按当前选择提交。同一设备串行，不同设备并行。传输显示已确认字节，Android 提交阶段使用不定进度。

菜单栏左键点击图标打开抽屉，右键可打开主窗口、进入设置、检查更新或退出。在「设置 → 常驻」可隐藏 Dock 图标。图钉将抽屉固定为可拖动的浮动窗口。连接、设备和任务详情在主窗口的系统表单中打开。关闭窗口仍可使用菜单栏，退出使用 QuickADB 菜单或 ⌘Q。

支持在线更新的 macOS 测试版会在启动时和常驻期间每天检查 GitHub Releases。设置页的「版本与更新」显示完整版本号并提供「检查更新…」。也可以从菜单栏图标的右键菜单或 QuickADB 应用菜单手动检查。确认后，应用核对安装镜像的 SHA-256、版本和代码签名，等待当前任务结束，再更新原位置的应用并重新打开。需要先把应用拷贝到可写目录。带临时签名的新版本仍可能触发 macOS 的应用或钥匙串授权提示。早期测试版需要先通过 DMG 升级一次。

## 构建

在 Apple 芯片 Mac 上准备带 macOS 27 SDK 的 Xcode 和 Rust 1.95 或更高版本。脚本以 Cargo.lock 构建 Rust 静态库，并将 ADB 和 libusb 链接到 `.app` 中的单个可执行文件。 [Xcode SDK 要求](https://developer.apple.com/xcode/system-requirements/)

```bash
rustup target add aarch64-apple-darwin
bash scripts/Build-Mac.sh
open artifacts/macos/QuickADB.app
```

测试版安装镜像使用 `bash scripts/Package-Mac.sh 1` 构建。脚本会生成带版本号的 `.dmg` 和 SHA-256 校验文件，并以只读方式挂载镜像验证其中的应用、签名和「应用程序」快捷入口。

构建脚本会验证签名及系统动态库依赖。默认使用临时签名做本机测试。使用已安装的签名身份时，通过 `QUICKADB_SIGN_IDENTITY` 指定。macOS 通过签名识别本地网络授权，验证 Finder 启动后的权限和配对时使用同一签名身份。对外发布还需完成签名和公证。 [Apple 本地网络隐私说明](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy) · [Apple 公证说明](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)

设置和日志保存在 `~/Library/Application Support/QuickADB`。macOS 在当前用户的登录钥匙串里保存 ADB 授权私钥。QuickADB 每次启动读取一次，后续连接和配对复用内存中的同一把私钥。首次在 Mac 使用时，在 Android 上授予该 Mac 的 USB 调试权限或完成无线配对。

## 验收

在登录钥匙串可用的 Mac 用户会话中运行协议夹具。夹具使用独立的数据目录和钥匙串条目。

```bash
cargo test --locked --target aarch64-apple-darwin
```

- 用 Finder 启动构建后的 `.app`。检查浅色、深色、减少透明度、菜单栏展开收起、固定后拖动与置顶、设置、重新打开窗口和登录项。
- 用 Finder 拖入含中文和空格路径的 APK，验证准备、移除、拆分选择、目标设备切换、任务进度、错误详情和退出确认。
- 用 Android 真机验证 USB 授权、首次无线配对、允许与拒绝本地网络、重连和断线、真实 APK 覆盖安装及数据保留。先确认调试接口未被另一个 ADB 工具独占。
