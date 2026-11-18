using System.Collections.Concurrent;
using System.IO.Compression;
using PuduLauncher.Constants;
using PuduLauncher.ContentScanning;
using PuduLauncher.Services.Interfaces;

namespace PuduLauncher.Services;

public class ScannerService(
    IHttpClientFactory httpClientFactory,
    IEnvironmentService environmentService,
    IErrorDisplayServer errorDisplayServer,
    ILogger<ScannerService> logger) : IScannerService
{
    private readonly ConcurrentDictionary<string, SemaphoreSlim> _goodFilesLocks =
        new(StringComparer.OrdinalIgnoreCase);

    public async Task<bool> ScanInstallationAsync(string installPath, string goodFileVersion, CancellationToken ct)
    {
        string? managedDir = FindManagedDirectory(installPath);
        if (managedDir == null)
        {
            logger.LogWarning("No managed directory found in {Path}, skipping code scan", installPath);
            return true;
        }

        using HttpClient client = httpClientFactory.CreateClient();
        HashSet<string> goodFileNames = await GetGoodFileNamesAsync(goodFileVersion, ct);
        logger.LogInformation("Good files loaded: {Count} trusted DLLs for version {Version}",
            goodFileNames.Count, goodFileVersion);

        string runtimeIdentifier = environmentService.GetRuntimeIdentifier();
        var scanner = new CodeScanClient(client, Path.Combine(environmentService.GetUserdataDirectory(), "CodeScan"), runtimeIdentifier);
        return await scanner.ScanAsync(managedDir, goodFileNames, message => logger.LogInformation("[CodeScan] {Message}", message), ct);
    }

    private async Task<HashSet<string>> GetGoodFileNamesAsync(string goodFileVersion, CancellationToken ct)
    {
        if (string.IsNullOrWhiteSpace(goodFileVersion))
        {
            logger.LogWarning("No good file version specified, all DLLs will be scanned");
            return [];
        }

        try
        {
            string goodFilesDir = await DownloadGoodFilesAsync(goodFileVersion, ct);
            string? goodManagedDir = FindManagedDirectory(goodFilesDir);
            if (goodManagedDir == null)
            {
                logger.LogWarning("No managed directory found in good files, all DLLs will be scanned");
                return [];
            }

            return new DirectoryInfo(goodManagedDir)
                .GetFiles("*.dll")
                .Select(f => f.Name)
                .ToHashSet(StringComparer.OrdinalIgnoreCase);
        }
        catch (Exception ex) when (ex is not OperationCanceledException || !ct.IsCancellationRequested)
        {
            logger.LogError(ex, "Failed to download good files for version {Version}, all DLLs will be scanned",
                goodFileVersion);
            _ = errorDisplayServer.ShowExceptionAsync(
                ex,
                source: "scanner-service.download-good-files",
                userMessage: "Could not download trusted good files. A full scan will be used instead.",
                code: "GOOD_FILES_DOWNLOAD_FAILED",
                isTransient: true);
            return [];
        }
    }

    private async Task<string> DownloadGoodFilesAsync(string goodFileVersion, CancellationToken ct)
    {
        string platformSuffix = environmentService.GetPlatformName();

        string folderName = $"{goodFileVersion}_{platformSuffix}";
        string cachePath = Path.Combine(
            environmentService.GetUserdataDirectory(), "GoodFiles", goodFileVersion, folderName);

        if (Directory.Exists(cachePath))
        {
            logger.LogDebug("Using cached good files at {Path}", cachePath);
            return cachePath;
        }

        SemaphoreSlim perVersionLock = _goodFilesLocks.GetOrAdd(cachePath, _ => new SemaphoreSlim(1, 1));
        await perVersionLock.WaitAsync(ct);
        try
        {
            // Another concurrent scan may have completed while we waited.
            if (Directory.Exists(cachePath))
            {
                logger.LogDebug("Using cached good files at {Path}", cachePath);
                return cachePath;
            }

            string zipUrl = $"{Api.GoodFilesBaseUrl}/{goodFileVersion}/{folderName}.zip";
            logger.LogInformation("Downloading good files from {Url}", zipUrl);

            using HttpClient client = httpClientFactory.CreateClient();
            using HttpResponseMessage response = await client.GetAsync(zipUrl, HttpCompletionOption.ResponseHeadersRead, ct);
            response.EnsureSuccessStatusCode();

            string extractPath = Path.Combine(
                environmentService.GetUserdataDirectory(), "GoodFiles", goodFileVersion);
            Directory.CreateDirectory(extractPath);

            await using Stream responseStream = await response.Content.ReadAsStreamAsync(ct);
            await using ZipArchive archive = new(responseStream);
            await archive.ExtractToDirectoryAsync(extractPath, overwriteFiles: true, cancellationToken: ct);

            // The zip contains a subfolder like StandaloneWindows64; rename to the expected cache name.
            string zipFolderName = environmentService.GetCanonicalEnvironment();

            string zipExtractedDir = Path.Combine(extractPath, zipFolderName);
            if (Directory.Exists(zipExtractedDir) && !string.Equals(zipExtractedDir, cachePath, StringComparison.OrdinalIgnoreCase))
            {
                Directory.Move(zipExtractedDir, cachePath);
            }

            logger.LogInformation("Good files extracted to {Path}", cachePath);
            return cachePath;
        }
        finally
        {
            perVersionLock.Release();
        }
    }

    private static string? FindManagedDirectory(string installPath)
    {
        if (!Directory.Exists(installPath)) return null;

        // Look for *_Data/Managed/ pattern (Unity convention)
        foreach (string dir in Directory.GetDirectories(installPath))
        {
            string managedPath = Path.Combine(dir, "Managed");
            if (dir.EndsWith("_Data", StringComparison.Ordinal) && Directory.Exists(managedPath))
            {
                return managedPath;
            }
        }

        // macOS: Unitystation.app/Contents/Resources/Data/Managed
        string macPath = Path.Combine(installPath, "Unitystation.app", "Contents", "Resources", "Data", "Managed");
        if (Directory.Exists(macPath))
        {
            return macPath;
        }

        return null;
    }
}
