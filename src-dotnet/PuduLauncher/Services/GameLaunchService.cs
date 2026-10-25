using System.Collections.Concurrent;
using System.Diagnostics;
using PuduLauncher.Abstractions.Interfaces;
using PuduLauncher.Models.Enums;
using PuduLauncher.Models.Events;
using PuduLauncher.Services.GameLaunchStrategies;
using PuduLauncher.Services.Interfaces;

namespace PuduLauncher.Services;

public class GameLaunchService(
    IInstallationService installationService,
    IEnvironmentService environmentService,
    IEnumerable<IGameLaunchStrategy> launchStrategies,
    IPreferencesService preferencesService,
    IEventPublisher eventPublisher,
    IDiscordPresenceService discordPresenceService,
    IErrorDisplayServer errorDisplayServer,
    ILogger<GameLaunchService> logger) : IGameLaunchService
{
    private readonly ConcurrentDictionary<string, Process> _runningGames = new();
    private readonly IGameLaunchStrategy _directLaunchStrategy = launchStrategies.Single(strategy => !strategy.UsesSandbox);
    private readonly IGameLaunchStrategy _sandboxLaunchStrategy = launchStrategies.Single(strategy => strategy.UsesSandbox);
    private int _sandboxDisabledForSession;

    private int _minimumTimeoutSecondsForSandboxInit = 30;

    public async Task LaunchGameAsync(
        Guid installationId,
        string? serverIp = null,
        int? serverPort = null)
    {
        var installation = installationService.GetInstallationById(installationId)
            ?? throw new InvalidOperationException($"Installation not found: {installationId}");

        string gameKey = $"{installation.ForkName}:{installation.BuildVersion}:{serverIp}:{serverPort}";
        bool sandboxEnabled = false;
        var sandboxDiagnostics = new ConcurrentQueue<string>();

        try
        {
            var preferences = preferencesService.GetPreferences();
            var sandboxPreferences = preferences.Sandbox;
            sandboxEnabled = sandboxPreferences.Enabled && Volatile.Read(ref _sandboxDisabledForSession) == 0;

            string executable = FindExecutable(installation.InstallationPath)
                ?? throw new InvalidOperationException(
                    $"Could not find executable in: {installation.InstallationPath}");

            EnsureExecutablePermissions(executable);

            IGameLaunchStrategy launchStrategy = sandboxEnabled
                ? _sandboxLaunchStrategy
                : _directLaunchStrategy;
            ProcessStartInfo? startInfo = launchStrategy.CreateProcessStartInfo(new GameLaunchRequest(
                executable,
                installation.InstallationPath,
                [preferences.Installations.InstallationPath, installation.InstallationPath],
                serverIp,
                serverPort,
                sandboxPreferences.MemoryLimitMb,
                sandboxPreferences.Backend));

            if (startInfo == null)
            {
                throw new InvalidOperationException(
                    $"Unsupported platform: {environmentService.GetCurrentEnvironment()}");
            }

            startInfo.UseShellExecute = false;
            startInfo.CreateNoWindow = true;
            startInfo.RedirectStandardOutput = true;
            startInfo.RedirectStandardError = true;

            var sandboxReady = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
            int sandboxInitializationComplete = 0;
            int processExited = 0;
            var process = new Process { StartInfo = startInfo, EnableRaisingEvents = true };
            // Drain child output so game engine logs do not bubble up to launcher logs.
            process.OutputDataReceived += (_, e) =>
            {
                if (sandboxEnabled && e.Data == "PUDU_SANDBOX_READY")
                {
                    Interlocked.Exchange(ref sandboxInitializationComplete, 1);
                    sandboxReady.TrySetResult(true);
                }
                else if (sandboxEnabled && e.Data is not null
                    && Volatile.Read(ref sandboxInitializationComplete) == 0)
                {
                    sandboxDiagnostics.Enqueue(e.Data);
                    logger.LogInformation("Sandbox runner: {Message}", e.Data);
                }
            };
            process.ErrorDataReceived += (_, e) =>
            {
                if (sandboxEnabled && e.Data is not null
                    && Volatile.Read(ref sandboxInitializationComplete) == 0)
                {
                    sandboxDiagnostics.Enqueue(e.Data);
                    logger.LogWarning("Sandbox runner: {Message}", e.Data);
                }
            };

            var eventServerIp = serverIp ?? string.Empty;
            var eventServerPort = serverPort ?? 0;

            process.Exited += async (_, _) =>
            {
                Interlocked.Exchange(ref processExited, 1);
                if (sandboxEnabled)
                {
                    sandboxReady.TrySetResult(false);
                }

                _runningGames.TryRemove(gameKey, out _);
                process.Dispose();

                logger.LogInformation("Game exited: {GameKey}", gameKey);
                if (_runningGames.IsEmpty)
                {
                    discordPresenceService.SetLauncherState();
                }

                await eventPublisher.PublishAsync(new GameStateChangedEvent
                {
                    ServerIp = eventServerIp,
                    ServerPort = eventServerPort,
                    IsRunning = false,
                });
            };

            bool started;
            if (!_runningGames.TryAdd(gameKey, process))
            {
                process.Dispose();
                throw new InvalidOperationException($"Game is already running for: {gameKey}");
            }

            try
            {
                started = process.Start();
            }
            catch
            {
                _runningGames.TryRemove(gameKey, out _);
                process.Dispose();
                throw;
            }

            if (!started)
            {
                _runningGames.TryRemove(gameKey, out _);
                process.Dispose();
                throw new InvalidOperationException($"Game process failed to start for executable: {executable}");
            }
            if (Volatile.Read(ref processExited) != 0)
            {
                _runningGames.TryRemove(gameKey, out _);
            }

            process.BeginOutputReadLine();
            process.BeginErrorReadLine();

            if (sandboxEnabled)
            {
                // Don't report a game as running until the selected sandbox strategy accepts its policy.
                bool ready;
                try
                {
                    ready = await sandboxReady.Task.WaitAsync(TimeSpan.FromSeconds(_minimumTimeoutSecondsForSandboxInit));
                }
                catch (TimeoutException)
                {
                    if (!process.HasExited)
                    {
                        process.Kill(entireProcessTree: true);
                    }

                    process.Dispose();
                    throw new InvalidOperationException($"The sandbox did not become ready within {_minimumTimeoutSecondsForSandboxInit} seconds.");
                }

                if (!ready)
                {
                    process.Dispose();
                    throw new InvalidOperationException(
                        "The sandbox could not start this game. Check the launcher log for the sandbox error.");
                }

                if (Volatile.Read(ref processExited) != 0)
                {
                    throw new InvalidOperationException(
                        "The sandbox exited before the game could start. Check the launcher log for the sandbox error.");
                }
            }

            discordPresenceService.StartGameSession(new GameSessionPresenceInfo(
                installation.ForkName,
                installation.BuildVersion,
                serverIp,
                serverPort));

            logger.LogInformation(
                "Launched game: Fork={ForkName} Version={BuildVersion} Server={ServerIp}:{ServerPort}",
                installation.ForkName, installation.BuildVersion, serverIp, serverPort);

            await eventPublisher.PublishAsync(new GameStateChangedEvent
            {
                ServerIp = eventServerIp,
                ServerPort = eventServerPort,
                IsRunning = true,
            });

            await installationService.MarkAsPlayedAsync(installationId);
        }
        catch (Exception ex)
        {
            if (sandboxEnabled)
            {
                await errorDisplayServer.ShowErrorAsync(
                    "backend.game-launch.sandbox",
                    "Could not launch the game within a contained sandbox. You can disable sandboxing for this session and try again, or check that your operating system supports the sandbox backend and is up to date.",
                    "SANDBOX_LAUNCH_FAILED",
                    $"{ex}{Environment.NewLine}{Environment.NewLine}Sandbox runner output:{Environment.NewLine}{string.Join(Environment.NewLine, sandboxDiagnostics)}",
                    isTransient: false);
            }

            logger.LogError(ex,
                "Failed to launch game: InstallationId={InstallationId} Fork={ForkName} Version={BuildVersion} Path={Path} Server={ServerIp}:{ServerPort}",
                installationId, installation.ForkName, installation.BuildVersion, installation.InstallationPath, serverIp, serverPort);
            throw;
        }
    }

    public bool IsGameRunning(string forkName, int buildVersion)
    {
        return _runningGames.ContainsKey($"{forkName}:{buildVersion}");
    }

    public void DisableSandboxForSession()
    {
        Interlocked.Exchange(ref _sandboxDisabledForSession, 1);
        logger.LogInformation("Sandbox disabled for the current launcher session");
    }

    private string? FindExecutable(string? installPath)
    {
        if (string.IsNullOrWhiteSpace(installPath) || !Directory.Exists(installPath))
        {
            return null;
        }

        string? executable = environmentService.GetCurrentEnvironment() switch
        {
            CurrentEnvironment.WindowsStandalone
                => Path.Combine(installPath, "Unitystation.exe"),
            CurrentEnvironment.MacOsStandalone
                => Path.Combine(installPath, "Unitystation.app", "Contents", "MacOS", "unitystation"),
            CurrentEnvironment.LinuxStandalone or CurrentEnvironment.LinuxFlatpak
                => Path.Combine(installPath, "Unitystation"),
            _ => null
        };

        return !string.IsNullOrWhiteSpace(executable) && File.Exists(executable)
            ? executable
            : null;
    }

    private void EnsureExecutablePermissions(string executablePath)
    {
        if (environmentService.GetCurrentEnvironment() == CurrentEnvironment.WindowsStandalone)
        {
            return;
        }

        try
        {
            var process = Process.Start("chmod", $"+x \"{executablePath}\"");
            process?.WaitForExit(5000);
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "Failed to set executable permissions on {Path}", executablePath);
        }
    }

}
