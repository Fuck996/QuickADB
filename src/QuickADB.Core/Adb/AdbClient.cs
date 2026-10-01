using QuickADB.Core.Devices;

namespace QuickADB.Core.Adb;

public sealed class AdbClient(int port = 5037)
{
    public int Port { get; } = port;

    public async Task<string> QueryAsync(string service, CancellationToken cancellationToken)
    {
        await using var connection = await AdbConnection.OpenAsync(Port, cancellationToken);
        await connection.RequestAsync(service, cancellationToken);
        return await connection.ReadMessageAsync(cancellationToken);
    }

    public async Task<IReadOnlyList<AdbDevice>> GetDevicesAsync(CancellationToken cancellationToken)
        => AdbDevice.ParseList(await QueryAsync("host:devices-l", cancellationToken));

    public async Task<HashSet<string>> GetFeaturesAsync(string serial, CancellationToken cancellationToken)
        => new((await QueryAsync($"host-serial:{serial}:features", cancellationToken)).Split(',', StringSplitOptions.RemoveEmptyEntries), StringComparer.Ordinal);

    public async Task<AdbConnection> OpenDeviceServiceAsync(string serial, string service, CancellationToken cancellationToken)
    {
        var connection = await AdbConnection.OpenAsync(Port, cancellationToken);
        try
        {
            await connection.RequestAsync("host:transport:" + serial, cancellationToken);
            await connection.RequestAsync(service, cancellationToken);
            return connection;
        }
        catch
        {
            await connection.DisposeAsync();
            throw;
        }
    }

    public async Task<string> ShellAsync(string serial, string command, CancellationToken cancellationToken)
    {
        await using var connection = await OpenDeviceServiceAsync(serial, "exec:" + command, cancellationToken);
        return await connection.ReadOutputAsync(cancellationToken);
    }
}
