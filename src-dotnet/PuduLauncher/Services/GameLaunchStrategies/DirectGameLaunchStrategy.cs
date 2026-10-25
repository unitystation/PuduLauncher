using System.Diagnostics;
using PuduLauncher.Services.Interfaces;

namespace PuduLauncher.Services.GameLaunchStrategies;

public sealed class DirectGameLaunchStrategy(IEnvironmentService environmentService) : IGameLaunchStrategy
{
    public bool UsesSandbox => false;

    public ProcessStartInfo? CreateProcessStartInfo(GameLaunchRequest request)
    {
        string arguments = string.IsNullOrWhiteSpace(request.ServerIp)
            ? string.Empty
            : request.ServerPort.HasValue
                ? $"--server {request.ServerIp} --port {request.ServerPort.Value}"
                : $"--server {request.ServerIp}";

        return environmentService.GetGameProcessStartInfo(request.Executable, arguments);
    }
}
