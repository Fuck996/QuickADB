using System.IO.Compression;

namespace QuickADB.Core.Installation;

public sealed record ApkPackage(string Path, string PackageName, string VersionCode, string SplitName, long Size)
{
    public string FileName => System.IO.Path.GetFileName(Path);

    public static ApkPackage Read(string path)
    {
        var fullPath = System.IO.Path.GetFullPath(path);
        if (!string.Equals(System.IO.Path.GetExtension(fullPath), ".apk", StringComparison.OrdinalIgnoreCase))
            throw new InvalidDataException("请选择 APK 文件。其他文件不会安装。");
        using var file = File.OpenRead(fullPath);
        using var archive = new ZipArchive(file, ZipArchiveMode.Read);
        var manifest = archive.GetEntry("AndroidManifest.xml") ?? throw new InvalidDataException("APK 缺少 AndroidManifest.xml。");
        if (manifest.Length > 4 * 1024 * 1024) throw new InvalidDataException("APK 清单文件过大。");
        using var input = manifest.Open();
        using var data = new MemoryStream();
        input.CopyTo(data);
        var attributes = AndroidManifest.ReadAttributes(data.ToArray());
        if (!attributes.TryGetValue("package", out var packageName) || string.IsNullOrWhiteSpace(packageName))
            throw new InvalidDataException("APK 清单缺少包名。");
        return new ApkPackage(fullPath, packageName, attributes.GetValueOrDefault("versionCode", ""), attributes.GetValueOrDefault("split", ""), file.Length);
    }

    public static void ValidateSplitGroup(IReadOnlyList<ApkPackage> packages)
    {
        if (packages.Count < 2 || packages.Count(x => x.SplitName.Length == 0) != 1)
            throw new InvalidDataException("拆分安装需要一个基础 APK 和至少一个分包 APK。");
        if (packages.Select(x => (x.PackageName, x.VersionCode)).Distinct().Count() != 1)
            throw new InvalidDataException("拆分 APK 必须属于同一应用和同一版本。");
        if (packages.Select(x => x.SplitName).Distinct(StringComparer.Ordinal).Count() != packages.Count)
            throw new InvalidDataException("拆分 APK 中有重复的分包。");
    }
}
