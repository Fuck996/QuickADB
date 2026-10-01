# 便携打包选型评估

这里的零释放是指不释放应用运行所需的 EXE、DLL；设备授权密钥、设置、日志和缓存数据仍然需要正常持久化。它不是零写盘。

## 方案 1：自包含 WPF 单文件与内嵌 ADB

用户仅携带一个 EXE。WPF 的原生运行时依赖与 ADB 文件释放到当前用户缓存目录，不要求安装 .NET 或 Android SDK。安装本身通过 ADB server 协议发送，直接计量传输字节。

实现难度中等。工程可复用官方 ADB 二进制，重点在设备状态、安装队列、窗口生命周期、进度与失败处理。

## 方案 2：原生应用与静态 ADB 能力

需要选择可静态发布的原生界面方案，并把官方 ADB host/server 所依赖的 Windows USB 通信、RSA 授权、TLS 配对、设备发现等整合进应用。官方 Windows 构建依赖 AdbWinApi 等辅助模块，不是改一个发布开关就能得到可嵌入的独立库。

还需要定义与 Android Studio 等工具的共享 server 行为、退出时的连接生命周期、版本更新和依赖许可说明。USB 驱动仍由系统或设备厂商提供。

开发难度高，粗略工程工作量预计为方案 1 的 2～4 倍，此范围未经过集成原型验证，不能视为承诺排期。现代抽屉界面、USB 与无线连接能力必须完整保留，不能以仅无线或依赖外部 ADB server 冒充完成。

WPF 不支持通过 Native AOT 简单达成此目标，需要重新选择界面与运行时路线。

## 资料

- ADB 官方构建定义：https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/main/Android.bp
- ADB 服务协议：https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/main/docs/dev/services.md
- ADB 官方流式安装：https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/main/client/adb_install.cpp
- .NET 单文件发布：https://learn.microsoft.com/en-us/dotnet/core/deploying/single-file/overview
- WPF 裁剪限制：https://learn.microsoft.com/en-us/dotnet/core/deploying/trimming/incompatibilities#wpf
- Native AOT：https://learn.microsoft.com/en-us/dotnet/core/deploying/native-aot/

用户已确认方案 2：原生应用，不释放 EXE/DLL 运行依赖，设置和设备授权数据正常保存。原 WPF 原型保存在本地 checkpoint，后续实现需验证静态依赖、USB、无线配对与安装链路。
