using System.Collections.Concurrent;
using System.IO.Compression;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using System.Text.Json;

namespace PuduLauncher.ContentScanning;

/// <summary>
/// Downloads and caches the upstream standalone scanner, checking for updates before each scan.
/// </summary>
public sealed class CodeScanClient(HttpClient httpClient, string cacheDirectory, string runtimeIdentifier)
{
    public const string LatestReleaseUrl = "https://api.github.com/repos/unitystation/codescan/releases/latest";
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
            using var request = new HttpRequestMessage(HttpMethod.Get, LatestReleaseUrl);
            request.Headers.UserAgent.ParseAdd("PuduLauncher");
            request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/vnd.github+json"));
            using HttpResponseMessage response = await httpClient.SendAsync(request, ct);
            response.EnsureSuccessStatusCode();
            string json = await response.Content.ReadAsStringAsync(ct);
            CodeScanRelease release = JsonSerializer.Deserialize(json, CodeScanJsonContext.Default.CodeScanRelease)
                ?? throw new CodeScanningException("GitHub returned an empty CodeScan release.");
            CodeScanAsset asset = release.Assets.SingleOrDefault(a => a.Name == $"{runtimeIdentifier}.zip")
                ?? throw new CodeScanningException($"CodeScan {release.TagName} has no {runtimeIdentifier} asset.");
            if (release.Id <= 0 || asset.Id <= 0 || !Uri.TryCreate(asset.DownloadUrl, UriKind.Absolute, out Uri? uri)
                || uri.Scheme != Uri.UriSchemeHttps)
                throw new CodeScanningException("GitHub returned invalid CodeScan release metadata.");

            if (cached != null && cached.ReleaseId == release.Id && cached.AssetId == asset.Id
                && cached.AssetUpdatedAt == asset.UpdatedAt)
                return Path.Combine(platformCache, cached.DirectoryName);

            log($"Downloading CodeScan {release.TagName} ({runtimeIdentifier}).");
            return await DownloadAsync(platformCache, manifestPath, release, asset, ct);
        }
        catch (Exception ex) when (!ct.IsCancellationRequested && cached != null &&
                                   ex is HttpRequestException or IOException or InvalidDataException or JsonException or CodeScanningException or TaskCanceledException)
        {
            log($"Could not update CodeScan; using the cached scanner: {ex.Message}");
            return Path.Combine(platformCache, cached.DirectoryName);
        }
    }

    private async Task<string> DownloadAsync(string platformCache, string manifestPath,
        CodeScanRelease release, CodeScanAsset asset, CancellationToken ct)
    {
        string directoryName = Guid.NewGuid().ToString("N");
        string destination = Path.Combine(platformCache, directoryName);
        string zipPath = destination + ".zip";
        string temporaryManifest = destination + ".json";
        bool committed = false;
        try
        {
            await using (Stream source = await httpClient.GetStreamAsync(asset.DownloadUrl, ct))
            await using (FileStream target = File.Create(zipPath))
                await source.CopyToAsync(target, ct);

            if (asset.Digest is { } digest && digest.StartsWith("sha256:", StringComparison.OrdinalIgnoreCase))
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
                ReleaseId = release.Id,
                AssetId = asset.Id,
                AssetUpdatedAt = asset.UpdatedAt,
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
        && File.Exists(Path.Combine(directory, runtimeIdentifier, "CodeScan.dll"))
        && File.Exists(Path.Combine(directory, runtimeIdentifier, "CodeScan.runtimeconfig.json"))
        && File.Exists(Path.Combine(directory, "FilesToMoveToManaged.json"));
}
