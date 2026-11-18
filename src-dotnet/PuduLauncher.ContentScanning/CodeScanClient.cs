using System.Collections.Concurrent;
using System.IO.Compression;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using System.Text.Json;

namespace PuduLauncher.ContentScanning;

/// <summary>
/// Downloads and caches the upstream standalone scanner, checking for updates before each scan.
/// </summary>
public sealed class CodeScanClient(HttpClient httpClient, string cacheDirectory, string runtimeIdentifier,
    TimeSpan? updateCheckTimeout = null)
{
    public const string LatestReleaseUrl = "https://api.github.com/repos/unitystation/codescan/releases/latest";
    private readonly TimeSpan _updateCheckTimeout = updateCheckTimeout ?? TimeSpan.FromSeconds(10);
    private static readonly ConcurrentDictionary<string, SemaphoreSlim> CacheLocks = new(StringComparer.OrdinalIgnoreCase);

    public async Task<string> EnsureAvailableAsync(Action<string> log, CancellationToken ct)
    {
        string platformCache = GetPlatformCache();
        SemaphoreSlim cacheLock = CacheLocks.GetOrAdd(platformCache, _ => new SemaphoreSlim(1, 1));
        await cacheLock.WaitAsync(ct);
        try
        {
            return await GetLatestScannerAsync(platformCache, log, ct);
        }
        finally
        {
            cacheLock.Release();
        }
    }

    public async Task<bool> ScanAsync(string managedDirectory, IReadOnlySet<string> goodFileNames,
        Action<string> log, CancellationToken ct)
    {
        string runDirectory = await CopyLatestScannerAsync(log, ct);
        return await CodeScanProcess.ScanAsync(runDirectory, runtimeIdentifier, managedDirectory, goodFileNames, log, ct);
    }

    private async Task<string> CopyLatestScannerAsync(Action<string> log, CancellationToken ct)
    {
        string platformCache = GetPlatformCache();
        SemaphoreSlim cacheLock = CacheLocks.GetOrAdd(platformCache, _ => new SemaphoreSlim(1, 1));
        await cacheLock.WaitAsync(ct);
        try
        {
            string scannerDirectory = await GetLatestScannerAsync(platformCache, log, ct);
            string runDirectory = Path.Combine(platformCache, "runs", Guid.NewGuid().ToString("N"));
            try
            {
                CodeScanProcess.CopyDirectory(scannerDirectory, runDirectory, ct);
            }
            catch
            {
                if (Directory.Exists(runDirectory)) Directory.Delete(runDirectory, recursive: true);
                throw;
            }
            return runDirectory;
        }
        finally
        {
            cacheLock.Release();
        }
    }

    private string GetPlatformCache()
    {
        if (runtimeIdentifier is not ("win-x64" or "linux-x64" or "osx-x64"))
            throw new PlatformNotSupportedException($"No CodeScan release is available for {runtimeIdentifier}.");

        return Path.GetFullPath(Path.Combine(cacheDirectory, runtimeIdentifier));
    }

    private async Task<string> GetLatestScannerAsync(string platformCache, Action<string> log, CancellationToken ct)
    {
        Directory.CreateDirectory(platformCache);
        string manifestPath = Path.Combine(platformCache, "current.json");
        CodeScanCacheEntry? cached = await ReadCacheAsync(manifestPath, platformCache, ct);
        try
        {
            CodeScanDownload download = await FindLatestDownloadAsync(cached != null, log, ct);
            if (cached != null && cached.ReleaseId == download.ReleaseId && cached.AssetId == download.AssetId
                && cached.AssetUpdatedAt == download.AssetUpdatedAt)
                return Path.Combine(platformCache, cached.DirectoryName);

            log($"Downloading CodeScan {download.TagName} ({runtimeIdentifier}).");
            return await DownloadAsync(platformCache, manifestPath, download, ct);
        }
        catch (Exception ex) when (cached != null && IsRecoverable(ex, ct))
        {
            log($"Could not update CodeScan; using the cached scanner: {ex.Message}");
            return Path.Combine(platformCache, cached.DirectoryName);
        }
    }

    private async Task<CodeScanDownload> FindLatestDownloadAsync(bool hasCache, Action<string> log, CancellationToken ct)
    {
        try
        {
            return await GetLatestReleaseDownloadAsync(ct);
        }
        catch (Exception ex) when (!hasCache && IsRecoverable(ex, ct))
        {
            log($"Could not ask GitHub for the latest CodeScan release ({ex.Message}); downloading it directly without a checksum.");
            return new CodeScanDownload("latest", 0, 0, "",
                $"https://github.com/unitystation/codescan/releases/latest/download/{runtimeIdentifier}.zip", null);
        }
    }

    private async Task<CodeScanDownload> GetLatestReleaseDownloadAsync(CancellationToken ct)
    {
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(ct);
        timeout.CancelAfter(_updateCheckTimeout);
        using var request = new HttpRequestMessage(HttpMethod.Get, LatestReleaseUrl);
        request.Headers.UserAgent.ParseAdd("PuduLauncher");
        request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/vnd.github+json"));
        using HttpResponseMessage response = await httpClient.SendAsync(request, timeout.Token);
        response.EnsureSuccessStatusCode();
        string json = await response.Content.ReadAsStringAsync(timeout.Token);
        CodeScanRelease release = JsonSerializer.Deserialize(json, CodeScanJsonContext.Default.CodeScanRelease)
            ?? throw new CodeScanningException("GitHub returned an empty CodeScan release.");
        CodeScanAsset asset = release.Assets.SingleOrDefault(a => a.Name == $"{runtimeIdentifier}.zip")
            ?? throw new CodeScanningException($"CodeScan {release.TagName} has no {runtimeIdentifier} asset.");
        if (release.Id <= 0 || asset.Id <= 0 || !Uri.TryCreate(asset.DownloadUrl, UriKind.Absolute, out Uri? uri)
            || uri.Scheme != Uri.UriSchemeHttps)
            throw new CodeScanningException("GitHub returned invalid CodeScan release metadata.");

        return new CodeScanDownload(release.TagName, release.Id, asset.Id, asset.UpdatedAt, asset.DownloadUrl, asset.Digest);
    }

    private static bool IsRecoverable(Exception ex, CancellationToken ct)
    {
        if (ct.IsCancellationRequested) return false;
        return ex is HttpRequestException or IOException or InvalidDataException or JsonException
            or CodeScanningException or OperationCanceledException;
    }

    private async Task<string> DownloadAsync(string platformCache, string manifestPath,
        CodeScanDownload download, CancellationToken ct)
    {
        string directoryName = Guid.NewGuid().ToString("N");
        string destination = Path.Combine(platformCache, directoryName);
        string zipPath = destination + ".zip";
        string temporaryManifest = destination + ".json";
        bool committed = false;
        try
        {
            await using (Stream source = await httpClient.GetStreamAsync(download.Url, ct))
            await using (FileStream target = File.Create(zipPath))
                await source.CopyToAsync(target, ct);

            if (download.Digest is { } digest && digest.StartsWith("sha256:", StringComparison.OrdinalIgnoreCase))
            {
                await using FileStream archiveStream = File.OpenRead(zipPath);
                string actual = Convert.ToHexString(await SHA256.HashDataAsync(archiveStream, ct));
                if (!actual.Equals(digest[7..], StringComparison.OrdinalIgnoreCase))
                    throw new CodeScanningException("The CodeScan archive SHA256 does not match the release digest.");
            }

            await ZipFile.ExtractToDirectoryAsync(zipPath, destination, ct);
            if (!IsComplete(destination))
                throw new CodeScanningException("The CodeScan release is missing required scanner files.");

            var entry = new CodeScanCacheEntry
            {
                ReleaseId = download.ReleaseId,
                AssetId = download.AssetId,
                AssetUpdatedAt = download.AssetUpdatedAt,
                DirectoryName = directoryName
            };
            await File.WriteAllTextAsync(temporaryManifest,
                JsonSerializer.Serialize(entry, CodeScanJsonContext.Default.CodeScanCacheEntry), ct);
            ct.ThrowIfCancellationRequested();
            File.Move(temporaryManifest, manifestPath, overwrite: true);
            committed = true;
            DeleteOtherReleases(platformCache, directoryName);
            return destination;
        }
        finally
        {
            File.Delete(zipPath);
            File.Delete(temporaryManifest);
            if (!committed && Directory.Exists(destination)) Directory.Delete(destination, recursive: true);
        }
    }

    private static void DeleteOtherReleases(string platformCache, string currentDirectoryName)
    {
        foreach (string directory in Directory.EnumerateDirectories(platformCache))
        {
            string name = Path.GetFileName(directory);
            bool isRelease = Guid.TryParseExact(name, "N", out _);
            if (!isRelease || name == currentDirectoryName) continue;
            try
            {
                Directory.Delete(directory, recursive: true);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
            }
        }
    }

    private async Task<CodeScanCacheEntry?> ReadCacheAsync(string manifestPath, string platformCache, CancellationToken ct)
    {
        try
        {
            string json = await File.ReadAllTextAsync(manifestPath, ct);
            CodeScanCacheEntry? entry = JsonSerializer.Deserialize(json, CodeScanJsonContext.Default.CodeScanCacheEntry);
            return entry != null && Guid.TryParseExact(entry.DirectoryName, "N", out _)
                                 && IsComplete(Path.Combine(platformCache, entry.DirectoryName)) ? entry : null;
        }
        catch (Exception ex) when (ex is IOException or JsonException)
        {
            return null;
        }
    }

    private bool IsComplete(string directory) =>
        File.Exists(Path.Combine(directory, runtimeIdentifier, CodeScanProcess.GetExecutableName(runtimeIdentifier)))
        && File.Exists(Path.Combine(directory, "FilesToMoveToManaged.json"));
}
