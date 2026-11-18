using System.Diagnostics;
using System.Runtime.CompilerServices;
using System.Text.Json;

[assembly: InternalsVisibleTo("PuduLauncher.Tests")]

namespace PuduLauncher.ContentScanning;

internal static class CodeScanProcess
{
    internal static async Task<bool> ScanAsync(string runDirectory, string runtimeIdentifier, string managedDirectory, IReadOnlySet<string> goodFileNames, Action<string> log, CancellationToken ct)
    {
        string platform = GetPlatformName(runtimeIdentifier);

        try
        {
            string buildDirectory = Path.Combine(runDirectory, "build");
            PrepareBuild(managedDirectory, goodFileNames, buildDirectory, runDirectory, platform, ct);

            string executableDirectory = Path.Combine(runDirectory, runtimeIdentifier);
            string executablePath = Path.Combine(executableDirectory, GetExecutableName(runtimeIdentifier));
            if (!OperatingSystem.IsWindows())
                File.SetUnixFileMode(executablePath, UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute);

            string reportPath = Path.Combine(executableDirectory, "errors.json");
            File.Delete(reportPath);
            var startInfo = new ProcessStartInfo(executablePath)
            {
                WorkingDirectory = executableDirectory,
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true
            };
            // Upstream treats '@' anywhere in an argument as "skip scanning" and strips
            // every '>'. A relative path keeps characters in the user's cache path out
            // of its argument parser. ArgumentList avoids shell interpretation.
            startInfo.ArgumentList.Add(">" + Path.Combine("..", "build"));
            startInfo.ArgumentList.Add("-supressCorrectedWhitelist");

            ct.ThrowIfCancellationRequested();
            using var process = new Process { StartInfo = startInfo };
            process.Start();
            process.StandardInput.Close();
            Task stdout = ReadOutputAsync(process.StandardOutput, log);
            Task stderr = ReadOutputAsync(process.StandardError, log);
            try
            {
                await process.WaitForExitAsync(ct);
                await Task.WhenAll(stdout, stderr);
            }
            catch
            {
                if (!process.HasExited) process.Kill(entireProcessTree: true);
                await process.WaitForExitAsync(CancellationToken.None);
                await Task.WhenAll(stdout, stderr);
                throw;
            }

            return await ReadResultAsync(reportPath, process.ExitCode, log, ct);
        }
        finally
        {
            if (Directory.Exists(runDirectory)) Directory.Delete(runDirectory, recursive: true);
        }
    }

    internal static string GetExecutableName(string runtimeIdentifier)
    {
        return runtimeIdentifier == "win-x64" ? "CodeScan.exe" : "CodeScan";
    }

    private static string GetPlatformName(string runtimeIdentifier)
    {
        return runtimeIdentifier switch
        {
            "win-x64" => "Windows",
            "linux-x64" => "Linux",
            "osx-x64" => "Mac",
            _ => throw new PlatformNotSupportedException($"No CodeScan platform is known for {runtimeIdentifier}.")
        };
    }

    internal static void PrepareBuild(string managedDirectory, IReadOnlySet<string> goodFileNames, string buildDirectory, string runDirectory, string platform, CancellationToken ct)
    {
        string stagedManagedDirectory = platform == "Mac"
            ? Path.Combine(buildDirectory, "Mac.app", "Contents", "Resources", "Data", "Managed")
            : Path.Combine(buildDirectory, "Unitystation_Data", "Managed");
        CopyDirectory(managedDirectory, stagedManagedDirectory, ct);
        string goodDirectory = Path.Combine(runDirectory, "MissingAssemblies", platform);
        Directory.CreateDirectory(goodDirectory);
        // Upstream uses the filenames in this directory as its trusted-file set.
        foreach (string file in Directory.EnumerateFiles(managedDirectory, "*.dll"))
        {
            ct.ThrowIfCancellationRequested();
            if (goodFileNames.Contains(Path.GetFileName(file)))
            {
                File.Copy(file, Path.Combine(goodDirectory, Path.GetFileName(file)), overwrite: true);
            }
        }
    }

    internal static async Task<bool> ReadResultAsync(string reportPath, int exitCode, Action<string> log, CancellationToken ct)
    {
        if (exitCode != 0)
        {
            log($"CodeScan exited with code {exitCode}.");
            return false;
        }

        try
        {
            string json = await File.ReadAllTextAsync(reportPath, ct);
            string[]? errors = JsonSerializer.Deserialize(json, CodeScanJsonContext.Default.StringArray);
            if (errors == null) throw new JsonException("CodeScan returned a null report.");
            foreach (string error in errors) log(error);
            return errors.Length == 0;
        }
        catch (Exception ex) when (ex is IOException or JsonException)
        {
            log($"CodeScan did not produce a valid errors.json report: {ex.Message}");
            return false;
        }
    }

    private static async Task ReadOutputAsync(StreamReader reader, Action<string> log)
    {
        while (await reader.ReadLineAsync() is { } line) log(line);
    }

    internal static void CopyDirectory(string source, string destination, CancellationToken ct)
    {
        Directory.CreateDirectory(destination);
        foreach (string file in Directory.EnumerateFiles(source))
        {
            ct.ThrowIfCancellationRequested();
            File.Copy(file, Path.Combine(destination, Path.GetFileName(file)), overwrite: true);
        }
        foreach (string directory in Directory.EnumerateDirectories(source))
        {
            ct.ThrowIfCancellationRequested();
            CopyDirectory(directory, Path.Combine(destination, Path.GetFileName(directory)), ct);
        }
    }
}
