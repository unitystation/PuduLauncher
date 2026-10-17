using PuduLauncher.Services.Interfaces;

namespace PuduLauncher.Services;

/// <summary>
/// Ensures the TTS server is stopped when the host shuts down gracefully.
/// </summary>
public class TtsShutdownHostedService(
    ITtsServerService serverService,
    IPreferencesService preferencesService,
    ILogger<TtsShutdownHostedService> logger) : IHostedLifecycleService
{
    public Task StartingAsync(CancellationToken cancellationToken) => Task.CompletedTask;

    public Task StartAsync(CancellationToken cancellationToken) => Task.CompletedTask;

    public Task StartedAsync(CancellationToken cancellationToken) => Task.CompletedTask;
    
    public async Task StoppingAsync(CancellationToken cancellationToken)
    {
        try
        {
            var prefs = preferencesService.GetPreferences();
            // Pass None: cleanup must complete even though the shutdown token may
            // already be cancelling. StopAsync bounds itself with its own grace timeout.
            await serverService.StopAsync(prefs.Tts.InstallPath, CancellationToken.None);
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "Failed to stop TTS server during shutdown");
        }
    }

    public Task StopAsync(CancellationToken cancellationToken) => Task.CompletedTask;

    public Task StoppedAsync(CancellationToken cancellationToken) => Task.CompletedTask;
}
