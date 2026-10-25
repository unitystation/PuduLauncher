using System.Diagnostics;
using PuduLauncher.Models.Enums;
using PuduLauncher.Services.Interfaces;

namespace PuduLauncher.Services.GameLaunchStrategies;

public sealed class SandboxGameLaunchStrategy(
    IEnvironmentService environmentService,
    ILogger<SandboxGameLaunchStrategy> logger) : IGameLaunchStrategy
{
    public bool UsesSandbox => true;

    public ProcessStartInfo CreateProcessStartInfo(GameLaunchRequest request)
    {
        if (environmentService.GetCurrentEnvironment() == CurrentEnvironment.LinuxFlatpak)
        {
            throw new InvalidOperationException("Flatpaks already run in a sandbox. What are you doing?");
        }

        if (request.SandboxMemoryLimitMb is < 512 or > 131_072)
        {
            throw new InvalidOperationException("Sandbox memory limit must be between 512 and 131072 MB.");
        }

        string hostExecutable = Environment.GetEnvironmentVariable("PUDU_HOST_EXE") ?? string.Empty;
        if (string.IsNullOrWhiteSpace(hostExecutable) || !File.Exists(hostExecutable))
        {
            throw new InvalidOperationException("Could not locate the launcher executable for sandboxing.");
        }

        string gameDataDirectory = GetGameDataDirectory();
        Directory.CreateDirectory(gameDataDirectory);

        var startInfo = new ProcessStartInfo(hostExecutable)
        {
            WorkingDirectory = request.InstallationPath,
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };

        startInfo.ArgumentList.Add("--pudu-sandbox-runner");
        startInfo.ArgumentList.Add("--memory-limit-mb");
        startInfo.ArgumentList.Add(request.SandboxMemoryLimitMb.ToString(System.Globalization.CultureInfo.InvariantCulture));
        startInfo.ArgumentList.Add("--sandbox-backend");
        startInfo.ArgumentList.Add(GetSandboxBackendArgument(request.SandboxBackend));
        foreach (string readOnlyPath in request.ReadOnlyPaths)
        {
            startInfo.ArgumentList.Add("--read-only-dir");
            startInfo.ArgumentList.Add(Path.GetFullPath(readOnlyPath));
        }
        startInfo.ArgumentList.Add("--game-data-dir");
        startInfo.ArgumentList.Add(Path.GetFullPath(gameDataDirectory));
        startInfo.ArgumentList.Add("--working-dir");
        startInfo.ArgumentList.Add(Path.GetFullPath(request.InstallationPath));
        startInfo.ArgumentList.Add("--program");
        startInfo.ArgumentList.Add(Path.GetFullPath(request.Executable));
        startInfo.ArgumentList.Add("--");
        foreach (string argument in BuildArgumentList(request.ServerIp, request.ServerPort))
        {
            startInfo.ArgumentList.Add(argument);
        }

        return startInfo;
    }

    private static string GetSandboxBackendArgument(string backend) => backend switch
    {
        "Automatic" => "auto",
        "Windows Sandbox API" => "windows-sandbox-api",
        "AppContainer" => "appcontainer",
        _ => throw new InvalidOperationException($"Unknown sandbox backend: {backend}")
    };

    private string GetGameDataDirectory()
    {
        CurrentEnvironment currentEnvironment = environmentService.GetCurrentEnvironment();
        string dataRoot = currentEnvironment switch
        {
            CurrentEnvironment.MacOsStandalone => Environment.GetFolderPath(
                Environment.SpecialFolder.ApplicationData),
            CurrentEnvironment.LinuxStandalone => GetLinuxConfigDirectory(),
            CurrentEnvironment.WindowsStandalone => GetWindowsLocalLowDirectory(),
            _ => throw new InvalidOperationException(
                $"Sandboxing is unavailable in {currentEnvironment}.")
        };

        if (string.IsNullOrWhiteSpace(dataRoot))
        {
            throw new InvalidOperationException("Could not resolve the current user's application data directory.");
        }

        string gameDataDirectory = currentEnvironment switch
        {
            CurrentEnvironment.LinuxStandalone => Path.Combine(dataRoot, "unity3d", "Unitystation", "unitystation"),
            _ => Path.Combine(dataRoot, "Unitystation", "unitystation")
        };

        gameDataDirectory = Path.GetFullPath(gameDataDirectory);
        logger.LogInformation("Resolved game data directory: {GameDataDirectory}", gameDataDirectory);
        return gameDataDirectory;
    }

    private static string GetLinuxConfigDirectory()
    {
        string? xdgConfigHome = Environment.GetEnvironmentVariable("XDG_CONFIG_HOME");
        if (!string.IsNullOrWhiteSpace(xdgConfigHome) && Path.IsPathFullyQualified(xdgConfigHome))
        {
            return xdgConfigHome;
        }

        return Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
    }

    private static string GetWindowsLocalLowDirectory()
    {
        string localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        if (string.IsNullOrWhiteSpace(localAppData))
        {
            throw new InvalidOperationException("Could not resolve the current user's local application data directory.");
        }

        string appDataRoot = Directory.GetParent(localAppData)?.FullName
            ?? throw new InvalidOperationException("Could not resolve the current user's local application data directory.");

        return Path.Combine(appDataRoot, "LocalLow");
    }

    private static IReadOnlyList<string> BuildArgumentList(string? serverIp, int? serverPort)
    {
        if (string.IsNullOrWhiteSpace(serverIp))
        {
            return [];
        }

        return serverPort.HasValue
            ? ["--server", serverIp, "--port", serverPort.Value.ToString(System.Globalization.CultureInfo.InvariantCulture)]
            : ["--server", serverIp];
    }
}
