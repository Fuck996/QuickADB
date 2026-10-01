using System.Globalization;
using System.Net;
using System.Net.Sockets;
using System.Text;

namespace QuickADB.Core.Adb;

public sealed class AdbConnection : IAsyncDisposable
{
    private readonly TcpClient _client;
    public NetworkStream Stream { get; }

    private AdbConnection(TcpClient client)
    {
        _client = client;
        Stream = client.GetStream();
    }

    public static async Task<AdbConnection> OpenAsync(int port, CancellationToken cancellationToken)
    {
        var client = new TcpClient { NoDelay = true };
        try
        {
            await client.ConnectAsync(IPAddress.Loopback, port, cancellationToken);
            return new AdbConnection(client);
        }
        catch
        {
            client.Dispose();
            throw;
        }
    }

    public async Task RequestAsync(string service, CancellationToken cancellationToken)
    {
        var bytes = Encoding.UTF8.GetBytes(service);
        if (bytes.Length > ushort.MaxValue) throw new ArgumentException("ADB 请求过长。", nameof(service));
        await Stream.WriteAsync(Encoding.ASCII.GetBytes(bytes.Length.ToString("X4", CultureInfo.InvariantCulture)), cancellationToken);
        await Stream.WriteAsync(bytes, cancellationToken);
        var status = new byte[4];
        await Stream.ReadExactlyAsync(status, cancellationToken);
        var text = Encoding.ASCII.GetString(status);
        if (text == "FAIL") throw new AdbException(await ReadMessageAsync(cancellationToken));
        if (text != "OKAY") throw new AdbException("ADB 返回无效状态：" + text);
    }

    public async Task<string> ReadMessageAsync(CancellationToken cancellationToken)
    {
        var header = new byte[4];
        await Stream.ReadExactlyAsync(header, cancellationToken);
        if (!int.TryParse(Encoding.ASCII.GetString(header), NumberStyles.HexNumber, CultureInfo.InvariantCulture, out var length))
            throw new AdbException("ADB 返回无效的消息长度。");
        var payload = new byte[length];
        await Stream.ReadExactlyAsync(payload, cancellationToken);
        return Encoding.UTF8.GetString(payload);
    }

    public async Task<string> ReadOutputAsync(CancellationToken cancellationToken)
    {
        using var output = new MemoryStream();
        var buffer = new byte[8192];
        int count;
        while ((count = await Stream.ReadAsync(buffer, cancellationToken)) > 0)
        {
            if (output.Length + count > 1024 * 1024) throw new AdbException("ADB 返回的安装结果超出限制。");
            output.Write(buffer, 0, count);
        }
        return Encoding.UTF8.GetString(output.ToArray()).Trim();
    }

    public ValueTask DisposeAsync()
    {
        _client.Dispose();
        return ValueTask.CompletedTask;
    }
}
