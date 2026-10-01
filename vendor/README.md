# 原生 ADB SDK

来源：https://github.com/poapoauu/DroidMux

固定提交：24b17e5fe83bc68f509215ab2cfeaa661005e852

源码下载自上游该提交的 codeload ZIP，保留上游许可证。当前启用 TCP、USB、Android 11 无线配对、mDNS 和 Shell；不启用投屏、文件管理或文件传输界面。

USB 后端通过静态 libusb 与 Windows 系统接口访问设备，无须释放 adb.exe 或应用 DLL。无线配对使用上游 SPAKE2/TLS 实现。该 SDK 尚处于 0.1 阶段，支持范围必须通过源码、协议测试和真机验收确认，不能把 README 声明作为完成证据。

安装需复用同一 SDK 连接生命周期，并采用官方包管理器流式安装服务。USB 界面占用必须明确展示，不自动终止其他调试工具。

本地修改：droidmux-client 增加 open_abb_exec_args，按 AOSP client/adb_install.cpp 使用 NUL 分隔的参数向量，并复用原 open_service 的会话与流管理。原上游 open_abb_exec 字符串方法拒绝 NUL，无法表达官方 APK 安装服务；不通过失败后改走另一条链路掩盖这一差异。
