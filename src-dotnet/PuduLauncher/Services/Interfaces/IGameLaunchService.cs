namespace PuduLauncher.Services.Interfaces;

public interface IGameLaunchService
{
    Task LaunchGameAsync(
        Guid installationId,
        string? serverIp = null,
        int? serverPort = null);
    void DisableSandboxForSession();
    bool IsGameRunning(string forkName, int buildVersion);
}
