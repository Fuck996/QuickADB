# QuickADB

常驻 Windows 托盘的 APK 安装抽屉。使用原生 Rust 应用与静态 ADB 协议库，运行时不释放 adb.exe 或应用 DLL，不要求安装 .NET，也不依赖外部 ADB server。

## 使用

1. USB：手机开启开发者选项和 USB 调试，连接后在手机上允许本机调试。Windows 需有适用于该手机 ADB 接口的驱动。
2. 无线：电脑与手机处于同一局域网，点击“连接设备”。Android 11+ 先在手机上打开“使用配对码配对设备”，再输入手机 IP、六位配对码、配对端口和连接端口；两个端口分别取自配对弹窗与无线调试主页。电脑不能远程触发此配对码弹窗。发现列表显示型号或真实服务名称，点击相应服务可填写对应端口。
3. 点击设备行选择安装目标，再点击取消选择，可选择多台。设备上线不会自动选中。
4. 将一个或多个独立 APK 拖入窗口，或点击“选择 APK”。任务绑定提交时选中的设备，之后改变选择不会转移已有任务。
5. 一个应用的 base/split APK 使用“安装拆分 APK”，一起选入该应用的全部包。普通批量安装不会把任意多个 APK 当作拆分包。

同一设备按顺序安装，不同设备可以同时安装。传输显示实际确认字节、百分比和速度；Android 执行安装时显示动态状态。最后结果丢失会显示“结果未知”，应先在手机上确认，再决定重试。默认覆盖更新，保留应用数据，不自动卸载或降级。

托盘单击打开抽屉，点击外部收起；固定后可以拖动标题栏放到桌面任意位置，置顶为独立设置。关闭窗口会隐藏到托盘，退出使用托盘菜单。开机启动需在设置中启用，启动后静默进入托盘。`--tray` 可用于直接启动到托盘。

设置、日志和授权数据保存在 `%LOCALAPPDATA%\QuickADB`。授权私钥使用当前 Windows 账号的 DPAPI 保护，不能直接作为其他账号的授权备份。移动 EXE 后，已启用的开机启动位置会在下一次正常运行时更新。

USB 接口可能被 Android Studio、adb 或其他调试工具占用，应用会显示连接错误，不自动关闭其他程序。驱动与并发访问限制见 [libusb Windows 文档](https://github.com/libusb/libusb/wiki/Windows)。本应用不提供文件传输、投屏或跨互联网远程服务。

## 构建与验证

Windows x64，PowerShell 7，Rust 1.95+，Visual Studio 2022 C++ Build Tools 与 Windows SDK。

```powershell
./scripts/Build.ps1
./scripts/Test.ps1
./scripts/Build.ps1 -Release
```

构建脚本使用项目内忽略的 Cargo 缓存、rsproxy 镜像和静态 C 运行库。版本由 Cargo.toml 与 Cargo.lock 固定。原生 SDK 的来源与修改见 [vendor/README.md](vendor/README.md)。

完成测试、窗口验证和依赖检查后，用 `scripts/Publish.ps1` 生成带版本号的本地产物，并将同版本 EXE 复制到 `U:\开发工作`，校验 SHA-256。输出目录、缓存、日志和授权文件不提交 Git。

当前版本的验证记录见 [docs/verification.md](docs/verification.md)。协议夹具通过不代表真实手机通过；USB 授权、手机无线配对、签名 APK 的真实安装和机型驱动兼容性仍需真机验收。

## Windows 发布者提示

当前测试版 EXE 未进行 Authenticode 代码签名。下载后出现“Windows 已保护你的电脑 / 无法识别的应用”可能是 SmartScreen 对未知发布者和文件信誉的提示，不能据此判断缺少运行依赖。更改版本号、图标或应用清单不能建立发布者信誉。

正式发行需要可信代码签名证书或经过身份验证的签名服务；签名也不保证新文件立即免除 SmartScreen 提示。微软说明自签名证书不能解决这种提示，见 [SmartScreen 应用信誉说明](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)。发布脚本核对 EXE 内部版本，输出实际签名状态与副本 SHA-256，不把未签名文件标记为已认证发布者。
