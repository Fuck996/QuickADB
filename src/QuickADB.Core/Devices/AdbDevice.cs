namespace QuickADB.Core.Devices;

public sealed record AdbDevice(string Serial, string State, string Model, string Product, bool IsUsb)
{
    public bool IsOnline => State == "device";
    public string DisplayName => string.IsNullOrWhiteSpace(Model) ? Serial : Model.Replace('_', ' ');
    public string ConnectionKind => Serial.StartsWith("emulator-", StringComparison.Ordinal) ? "模拟器" : IsUsb ? "USB" : "无线";
    public string StatusText => State switch
    {
        "device" => "已连接",
        "unauthorized" => "等待手机授权",
        "offline" => "离线",
        "disconnected" => "已断开",
        "recovery" => "恢复模式",
        "sideload" => "侧载模式",
        "bootloader" => "引导模式",
        _ => State
    };

    public static IReadOnlyList<AdbDevice> ParseList(string response)
    {
        var devices = new List<AdbDevice>();
        foreach (var line in response.Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            var fields = line.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
            if (fields.Length < 2) throw new InvalidDataException("ADB 设备列表格式无效。");
            string Attribute(string name) => fields.Skip(2).FirstOrDefault(x => x.StartsWith(name + ":", StringComparison.Ordinal))?[(name.Length + 1)..] ?? "";
            devices.Add(new AdbDevice(fields[0], fields[1], Attribute("model"), Attribute("product"), fields.Skip(2).Any(x => x.StartsWith("usb:", StringComparison.Ordinal))));
        }
        return devices;
    }
}
