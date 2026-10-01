namespace QuickADB.Core.Adb;

public sealed class AdbException(string message) : IOException(message);
