namespace QuickADB.Core.Installation;

public static class InstallErrors
{
    public static string Explain(string error) => error switch
    {
        var value when value.Contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE", StringComparison.Ordinal) => "签名与已安装应用不一致。需要你决定是否卸载旧应用；卸载会清除数据。",
        var value when value.Contains("INSTALL_FAILED_VERSION_DOWNGRADE", StringComparison.Ordinal) => "APK 版本低于设备上的版本，未执行降级安装。",
        var value when value.Contains("INSTALL_FAILED_INSUFFICIENT_STORAGE", StringComparison.Ordinal) => "设备存储空间不足，请清理后重试。",
        var value when value.Contains("INSTALL_FAILED_NO_MATCHING_ABIS", StringComparison.Ordinal) => "APK 的处理器架构与设备不兼容。",
        var value when value.Contains("INSTALL_FAILED_OLDER_SDK", StringComparison.Ordinal) => "APK 要求更高的 Android 版本。",
        var value when value.Contains("INSTALL_FAILED_USER_RESTRICTED", StringComparison.Ordinal) => "设备限制了安装，请检查手机上的确认提示和 USB 安装设置。",
        var value when value.Contains("unauthorized", StringComparison.OrdinalIgnoreCase) => "请解锁手机，允许这台电脑进行 USB 调试。",
        var value when value.Contains("offline", StringComparison.OrdinalIgnoreCase) || value.Contains("not found", StringComparison.OrdinalIgnoreCase) => "目标设备已离线或断开，请恢复连接后重试。",
        _ => error
    };
}
