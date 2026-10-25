using System.Diagnostics;
using PuduLauncher.Services.GameLaunchStrategies;

namespace PuduLauncher.Services.Interfaces;

public interface IGameLaunchStrategy
{
    bool UsesSandbox { get; }

    ProcessStartInfo? CreateProcessStartInfo(GameLaunchRequest request);
}
