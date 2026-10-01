using System.Buffers.Binary;
using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;
using QuickADB.Core.Adb;

namespace QuickADB.Core.Installation;

public sealed partial class ApkInstaller(AdbClient adb)
{
    public async Task InstallAsync(string serial, IReadOnlyList<ApkPackage> packages, InstallOptions options, IProgress<InstallProgress> progress, CancellationToken cancellationToken)
    {
        if (packages.Count == 0) throw new ArgumentException("没有 APK。", nameof(packages));
        if (packages.Count > 1) ApkPackage.ValidateSplitGroup(packages);
        else if (packages[0].SplitName.Length > 0) throw new InvalidDataException("这是拆分 APK，请通过“安装拆分 APK”与基础 APK 一起安装。");

        progress.Report(new InstallProgress(InstallStage.Preparing));
        var features = await adb.GetFeaturesAsync(serial, cancellationToken);
        var total = packages.Sum(x => x.Size);
        var clock = Stopwatch.StartNew();
        var flags = new List<string> { "-r" };
        if (options.AllowTestPackages) flags.Add("-t");
        if (options.GrantPermissions) flags.Add("-g");

        string Service(params string[] arguments) => features.Contains("abb_exec")
            ? "abb_exec:" + string.Join('\0', new[] { "package" }.Concat(arguments)) + '\0'
            : "exec:cmd package " + string.Join(' ', arguments);

        if (!features.Contains("cmd"))
        {
            if (packages.Count > 1) throw new AdbException("此设备不支持拆分 APK 安装。");
            await InstallLegacyAsync(serial, packages[0], flags, progress, clock, cancellationToken);
            return;
        }

        if (packages.Count == 1)
        {
            var arguments = new[] { "install" }.Concat(flags).Concat(new[] { "-S", total.ToString(CultureInfo.InvariantCulture) }).ToArray();
            await using var connection = await adb.OpenDeviceServiceAsync(serial, Service(arguments), cancellationToken);
            await SendApkAsync(connection, packages[0], 0, total, clock, progress, cancellationToken);
            progress.Report(new InstallProgress(InstallStage.Installing, total, total));
            EnsureSuccess(await connection.ReadOutputAsync(cancellationToken));
        }
        else
        {
            await using var create = await adb.OpenDeviceServiceAsync(serial, Service(new[] { "install-create" }.Concat(flags).Concat(new[] { "-S", total.ToString(CultureInfo.InvariantCulture) }).ToArray()), cancellationToken);
            var createResult = await create.ReadOutputAsync(cancellationToken);
            EnsureSuccess(createResult);
            var match = SessionPattern().Match(createResult);
            if (!match.Success) throw new AdbException("未取得安装会话 ID：" + createResult);
            var session = match.Groups[1].Value;
            try
            {
                long sent = 0;
                for (var index = 0; index < packages.Count; index++)
                {
                    var package = packages[index];
                    await using var write = await adb.OpenDeviceServiceAsync(serial, Service("install-write", "-S", package.Size.ToString(CultureInfo.InvariantCulture), session, $"part{index}.apk", "-"), cancellationToken);
                    await SendApkAsync(write, package, sent, total, clock, progress, cancellationToken);
                    EnsureSuccess(await write.ReadOutputAsync(cancellationToken));
                    sent += package.Size;
                }
                progress.Report(new InstallProgress(InstallStage.Installing, total, total));
                await using var commit = await adb.OpenDeviceServiceAsync(serial, Service("install-commit", session), cancellationToken);
                EnsureSuccess(await commit.ReadOutputAsync(cancellationToken));
            }
            catch (Exception installationError)
            {
                try
                {
                    using var cleanupTimeout = new CancellationTokenSource(TimeSpan.FromSeconds(5));
                    await using var abandon = await adb.OpenDeviceServiceAsync(serial, Service("install-abandon", session), cleanupTimeout.Token);
                    EnsureSuccess(await abandon.ReadOutputAsync(cleanupTimeout.Token));
                }
                catch (Exception cleanupError)
                {
                    throw new AggregateException($"安装会话 {session} 失败，且设备上的会话未能清理。", installationError, cleanupError);
                }
                throw;
            }
        }
        progress.Report(new InstallProgress(InstallStage.Succeeded, total, total));
    }

    private static async Task SendApkAsync(AdbConnection connection, ApkPackage package, long previouslySent, long total, Stopwatch clock, IProgress<InstallProgress> progress, CancellationToken cancellationToken)
    {
        await using var file = new FileStream(package.Path, FileMode.Open, FileAccess.Read, FileShare.Read, 128 * 1024, FileOptions.Asynchronous | FileOptions.SequentialScan);
        if (file.Length != package.Size) throw new IOException("APK 在加入队列后发生变化，请重新添加。");
        var buffer = new byte[128 * 1024];
        long sent = 0;
        long lastReport = 0;
        int count;
        while ((count = await file.ReadAsync(buffer, cancellationToken)) > 0)
        {
            await connection.Stream.WriteAsync(buffer.AsMemory(0, count), cancellationToken);
            sent += count;
            if (clock.ElapsedMilliseconds - lastReport >= 80 || sent == package.Size)
            {
                lastReport = clock.ElapsedMilliseconds;
                var bytes = previouslySent + sent;
                progress.Report(new InstallProgress(InstallStage.Transferring, bytes, total, bytes / Math.Max(clock.Elapsed.TotalSeconds, 0.001)));
            }
        }
    }

    private async Task InstallLegacyAsync(string serial, ApkPackage package, IReadOnlyList<string> flags, IProgress<InstallProgress> progress, Stopwatch clock, CancellationToken cancellationToken)
    {
        var remotePath = "/data/local/tmp/quickadb-" + Guid.NewGuid().ToString("N") + ".apk";
        Exception? installationError = null;
        string cleanupError = "";
        try
        {
        await using (var sync = await adb.OpenDeviceServiceAsync(serial, "sync:", cancellationToken))
        {
            await WriteSyncMessageAsync(sync, "SEND", Encoding.UTF8.GetBytes(remotePath + ",33188"), cancellationToken);
            await using var file = new FileStream(package.Path, FileMode.Open, FileAccess.Read, FileShare.Read, 64 * 1024, FileOptions.Asynchronous);
            if (file.Length != package.Size) throw new IOException("APK 在加入队列后发生变化，请重新添加。");
            var buffer = new byte[64 * 1024];
            long sent = 0;
            int count;
            while ((count = await file.ReadAsync(buffer, cancellationToken)) > 0)
            {
                await WriteSyncMessageAsync(sync, "DATA", buffer.AsMemory(0, count), cancellationToken);
                sent += count;
                progress.Report(new InstallProgress(InstallStage.Transferring, sent, package.Size, sent / Math.Max(clock.Elapsed.TotalSeconds, 0.001)));
            }
            var done = new byte[8];
            Encoding.ASCII.GetBytes("DONE").CopyTo(done, 0);
            BinaryPrimitives.WriteUInt32LittleEndian(done.AsSpan(4), checked((uint)DateTimeOffset.UtcNow.ToUnixTimeSeconds()));
            await sync.Stream.WriteAsync(done, cancellationToken);
            var response = new byte[8];
            await sync.Stream.ReadExactlyAsync(response, cancellationToken);
            if (Encoding.ASCII.GetString(response, 0, 4) == "FAIL")
            {
                var length = BinaryPrimitives.ReadUInt32LittleEndian(response.AsSpan(4));
                if (length > 65536) throw new AdbException("ADB 文件同步错误消息无效。");
                var error = new byte[length];
                await sync.Stream.ReadExactlyAsync(error, cancellationToken);
                throw new AdbException(Encoding.UTF8.GetString(error));
            }
            if (Encoding.ASCII.GetString(response, 0, 4) != "OKAY") throw new AdbException("ADB 文件同步没有确认成功。");
        }
        progress.Report(new InstallProgress(InstallStage.Installing, package.Size, package.Size));
        var result = await adb.ShellAsync(serial, "pm install " + string.Join(' ', flags) + " " + remotePath, cancellationToken);
        EnsureSuccess(result);
        }
        catch (Exception exception) { installationError = exception; }
        try
        {
            using var cleanupTimeout = new CancellationTokenSource(TimeSpan.FromSeconds(5));
            cleanupError = await adb.ShellAsync(serial, "rm -f " + remotePath, cleanupTimeout.Token);
        }
        catch (Exception exception) { cleanupError = exception.Message; }
        if (installationError is not null)
        {
            if (cleanupError.Length != 0)
                throw new AggregateException("安装失败，且设备临时文件未能清理：" + cleanupError, installationError);
            System.Runtime.ExceptionServices.ExceptionDispatchInfo.Capture(installationError).Throw();
        }
        progress.Report(new InstallProgress(InstallStage.Succeeded, package.Size, package.Size, Detail: cleanupError.Length == 0 ? "" : "APK 已安装，但设备临时文件清理失败：" + cleanupError));
    }

    private static async Task WriteSyncMessageAsync(AdbConnection connection, string command, ReadOnlyMemory<byte> payload, CancellationToken cancellationToken)
    {
        var header = new byte[8];
        Encoding.ASCII.GetBytes(command).CopyTo(header, 0);
        BinaryPrimitives.WriteUInt32LittleEndian(header.AsSpan(4), (uint)payload.Length);
        await connection.Stream.WriteAsync(header, cancellationToken);
        await connection.Stream.WriteAsync(payload, cancellationToken);
    }

    private static void EnsureSuccess(string output)
    {
        if (!output.StartsWith("Success", StringComparison.Ordinal))
            throw new AdbException(output.Length == 0 ? "设备未返回安装结果，请在设备上确认安装状态。" : output);
    }

    [GeneratedRegex(@"\[(\d+)\]")]
    private static partial Regex SessionPattern();
}
