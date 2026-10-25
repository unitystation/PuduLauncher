using PuduLauncher.Abstractions.Attributes;

namespace PuduLauncher.Models.Config;

[PreferenceCategory("Sandbox")]
public class SandboxPreferences
{
    [PreferenceField("Enable sandbox", "toggle",
        Tooltip = "Run game files in a restricted environment. Supported on Windows systems and standalone Linux and macOS; unavailable in Flatpak builds.")]
    public bool Enabled { get; set; }

    [PreferenceField("Backend", "select",
        Options = ["Automatic", "Windows Sandbox API", "AppContainer"],
        Tooltip = "Choose which Windows sandbox backend to use. Automatic tries available backends in order.")]
    public string Backend { get; set; } = "Automatic";

    [PreferenceField("Memory limit (MB)", "number",
        Tooltip = "Maximum memory available to each sandboxed game (512 to 131072 MB).")]
    public int MemoryLimitMb { get; set; } = 8192;
}
