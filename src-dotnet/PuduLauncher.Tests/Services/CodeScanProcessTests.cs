using PuduLauncher.ContentScanning;

namespace PuduLauncher.Tests.Services;

public sealed class CodeScanProcessTests : IDisposable
{
    private readonly string _root = Path.Combine(Path.GetTempPath(), "PuduCodeScanTests", Guid.NewGuid().ToString("N"));

    [Theory]
    [InlineData("[]", 0, true)]
    [InlineData("[\"Disallowed method\"]", 0, false)]
    [InlineData("[]", 1, false)]
    [InlineData("null", 0, false)]
    [InlineData("{}", 0, false)]
    [InlineData("partial json", 0, false)]
    public async Task ScanResult_RequiresSuccessfulProcessAndEmptyErrorArray(string report, int exitCode, bool expected)
    {
        Directory.CreateDirectory(_root);
        string reportPath = Path.Combine(_root, "errors.json");
        await File.WriteAllTextAsync(reportPath, report);
        var messages = new List<string>();

        Assert.Equal(expected, await CodeScanProcess.ReadResultAsync(reportPath, exitCode, messages.Add, CancellationToken.None));
        if (report.Contains("Disallowed method")) Assert.Contains("Disallowed method", messages);
    }

    [Fact]
    public async Task MissingReport_FailsScan()
    {
        Assert.False(await CodeScanProcess.ReadResultAsync(Path.Combine(_root, "errors.json"), 0, _ => { }, CancellationToken.None));
    }

    [Theory]
    [InlineData("Windows", "Unitystation_Data/Managed")]
    [InlineData("Linux", "Unitystation_Data/Managed")]
    [InlineData("Mac", "Mac.app/Contents/Resources/Data/Managed")]
    public void PrepareBuild_AdaptsLayoutAndKeepsTrustedFilesOutOfScan(string platform, string relativeManagedPath)
    {
        string managed = Path.Combine(_root, "Original.app", "Managed");
        Directory.CreateDirectory(managed);
        File.WriteAllText(Path.Combine(managed, "Trusted.dll"), "trusted dependency");
        File.WriteAllText(Path.Combine(managed, "Content.dll"), "scan me");
        File.WriteAllText(Path.Combine(managed, "Content.pdb"), "symbols");
        string runDirectory = Path.Combine(_root, "run");
        string buildDirectory = Path.Combine(runDirectory, "build");

        CodeScanProcess.PrepareBuild(managed, new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "trusted.DLL" }, buildDirectory, runDirectory, platform, CancellationToken.None);

        Assert.True(File.Exists(Path.Combine(buildDirectory, relativeManagedPath, "Content.dll")));
        Assert.True(File.Exists(Path.Combine(buildDirectory, relativeManagedPath, "Trusted.dll")));
        string trustedDirectory = Path.Combine(runDirectory, "MissingAssemblies", platform);
        Assert.Equal("trusted dependency", File.ReadAllText(Path.Combine(trustedDirectory, "Trusted.dll")));
        Assert.Single(Directory.GetFiles(trustedDirectory));
        Assert.Equal(3, Directory.GetFiles(managed).Length);
    }

    public void Dispose()
    {
        if (Directory.Exists(_root)) Directory.Delete(_root, recursive: true);
    }
}
