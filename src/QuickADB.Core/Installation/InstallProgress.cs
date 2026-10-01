namespace QuickADB.Core.Installation;

public enum InstallStage { Preparing, Transferring, Installing, Succeeded, Failed, Cancelled, Unknown }

public sealed record InstallProgress(InstallStage Stage, long BytesSent = 0, long TotalBytes = 0, double BytesPerSecond = 0, string Detail = "")
{
    public double Percentage => TotalBytes == 0 ? 0 : Math.Clamp(BytesSent * 100d / TotalBytes, 0, 100);
}

public sealed record InstallOptions(bool AllowTestPackages = true, bool GrantPermissions = false);
