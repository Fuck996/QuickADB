# 原生 ADB SDK

## 窗口渲染依赖

`eframe/` 来自已锁定的 eframe 0.36.2 发布源码，上游提交 `49682f8baa058bf49e011035cfbd6e825f88a5ef`，保留 MIT / Apache-2.0 双许可。仅回移官方 [PR #8461](https://github.com/emilk/egui/pull/8461) 最终合并补丁（`983155456c964fcc92c951ed6b1c721cc24a205c`），没有升级其余 egui 组件或替换 Glow 后端。

原 Windows 路径每帧在清屏前、绘制前都解绑并重新绑定同一 GL 上下文。[WGL 文档](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-wglmakecurrent)说明切换会刷新之前上下文的命令，因而清屏与内容绘制之间也多了一次切换。最终上游补丁记录实际绑定的 viewport，同时验证上下文仍为 current；目标未变时保留绑定，真正切换窗口时仍正常切换。窗口销毁、重建、挂起时清除记录，避免仅按 context 判断而画到错误的 surface（上游 #4289）。

这是对已确认的渲染生命周期问题的修正；上游问题本身证明的是重复切换及其开销，不等于已证明用户现场黑闪的唯一原因。该限制和现场验收状态见 `docs/verification.md`。

## ADB 协议依赖

来源：https://github.com/poapoauu/DroidMux

固定提交：24b17e5fe83bc68f509215ab2cfeaa661005e852

源码下载自上游该提交的 codeload ZIP，保留上游许可证。当前启用 TCP、USB、Android 11 无线配对、mDNS 和 Shell；不启用投屏、文件管理或文件传输界面。

USB 后端通过静态 libusb 与 Windows 系统接口访问设备，无须释放 adb.exe 或应用 DLL。无线配对使用上游 SPAKE2/TLS 实现。该 SDK 尚处于 0.1 阶段，支持范围必须通过源码、协议测试和真机验收确认，不能把 README 声明作为完成证据。

安装需复用同一 SDK 连接生命周期，并采用官方包管理器流式安装服务。USB 界面占用必须明确展示，不自动终止其他调试工具。

本地修改：droidmux-client 增加 open_abb_exec_args，按 AOSP client/adb_install.cpp 使用 NUL 分隔的参数向量，并复用原 open_service 的会话与流管理。原上游 open_abb_exec 字符串方法拒绝 NUL，无法表达官方 APK 安装服务；不通过失败后改走另一条链路掩盖这一差异。

droidmux-client 的普通接收队列按字节限制容量，允许 Android 连续发送较小的 WRTE（真机命令的 stdout 和退出状态会连续到达），不再收到第二包就关闭流。普通模式最多缓冲两倍协商单包大小，读取后释放容量；delayed ACK 的协商窗口与确认语义保持不变。依据：[AOSP socket 接收队列与背压](https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/main/sockets.cpp)。

droidmux-pairing 在每个 TLS ADB 包写入后执行 flush，并计入同一写超时，保证等待设备确认前排空加密缓冲。依据：[tokio-rustls 0.26.4 的 flush 要求](https://docs.rs/tokio-rustls/0.26.4/tokio_rustls/)。

droidmux-discovery 保留 AOSP 广播的 `name` TXT 型号和真实服务实例名，区分 `_adb-tls-pairing`、`_adb-tls-connect` 与 `_adb`。同一浏览链路返回解析更新与撤销事件，避免把配对端口当连接端口或保留已经撤销的配对服务。`browse` 返回类型已更新为 `AdbMdnsEvent`，应用和 `discover_once` 使用同一事件语义。依据：[AOSP 广播源码](https://android.googlesource.com/platform/packages/modules/adb/+/HEAD/daemon/mdns.cpp)、[ADB Wi-Fi 架构](https://android.googlesource.com/platform/packages/modules/adb/+/HEAD/docs/dev/adb_wifi.md)。
