namespace PuduLauncher.Services.GameLaunchStrategies;

public sealed record GameLaunchRequest(
    string Executable,
    string InstallationPath,
    IReadOnlyList<string> ReadOnlyPaths,
    string? ServerIp,
    int? ServerPort,
    int SandboxMemoryLimitMb,
    string SandboxBackend);
