using System.Diagnostics;
using PuduLauncher.Models.Enums;
using PuduLauncher.Services.Interfaces;

namespace PuduLauncher.Tests.Infrastructure;

internal sealed class TestEnvironmentService(
    string userdataDirectory,
    CurrentEnvironment currentEnvironment = CurrentEnvironment.WindowsStandalone) : IEnvironmentService
{
    public CurrentEnvironment GetCurrentEnvironment()
    {
        return currentEnvironment;
    }

    public string GetUserdataDirectory()
    {
        return userdataDirectory;
    }

    public string GetCanonicalEnvironment()
    {
        return currentEnvironment switch
        {
            CurrentEnvironment.WindowsStandalone => "StandaloneWindows64",
            CurrentEnvironment.LinuxStandalone or CurrentEnvironment.LinuxFlatpak => "StandaloneLinux64",
            CurrentEnvironment.MacOsStandalone => "StandaloneOSX",
            _ => throw new ArgumentOutOfRangeException(nameof(currentEnvironment), currentEnvironment, null),
        };
    }

    public string GetRuntimeIdentifier()
    {
        return currentEnvironment switch
        {
            CurrentEnvironment.WindowsStandalone => "win-x64",
            CurrentEnvironment.LinuxStandalone or CurrentEnvironment.LinuxFlatpak => "linux-x64",
            CurrentEnvironment.MacOsStandalone => "osx-x64",
            _ => throw new ArgumentOutOfRangeException(nameof(currentEnvironment), currentEnvironment, null),
        };
    }

    public string GetPlatformName()
    {
        return currentEnvironment switch
        {
            CurrentEnvironment.WindowsStandalone => "Windows",
            CurrentEnvironment.LinuxStandalone or CurrentEnvironment.LinuxFlatpak => "Linux",
            CurrentEnvironment.MacOsStandalone => "Mac",
            _ => throw new ArgumentOutOfRangeException(nameof(currentEnvironment), currentEnvironment, null),
        };
    }

    public bool ShouldDisableUpdateCheck()
    {
        return currentEnvironment == CurrentEnvironment.LinuxFlatpak;
    }

    public ProcessStartInfo? GetGameProcessStartInfo(string executable, string arguments)
    {
        return null;
    }
}
