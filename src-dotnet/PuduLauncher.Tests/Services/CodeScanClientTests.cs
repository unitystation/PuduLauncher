using System.IO.Compression;
using System.Net;
using System.Security.Cryptography;
using System.Text.Json;
using PuduLauncher.ContentScanning;
using PuduLauncher.Tests.Infrastructure;

namespace PuduLauncher.Tests.Services;

public sealed class CodeScanClientTests : IDisposable
{
    private readonly string _root = Path.Combine(Path.GetTempPath(), "PuduCodeScanTests", Guid.NewGuid().ToString("N"));
    private const string AssetUrl = "https://github.com/unitystation/codescan/releases/download/build-1-1/win-x64.zip";

    [Fact]
    public async Task MissingScanner_DownloadsAndCachesLatestRelease()
    {
        byte[] archive = CreateArchive();
        var handler = new DelegateHttpMessageHandler((request, _) =>
        {
            if (request.RequestUri!.ToString() == CodeScanClient.LatestReleaseUrl)
            {
                Assert.Contains("PuduLauncher", request.Headers.UserAgent.ToString());
                return DelegateHttpMessageHandler.Json(ReleaseJson(archive));
            }
            Assert.Equal(AssetUrl, request.RequestUri.ToString());
            return ZipResponse(archive);
        });
        using var client = new HttpClient(handler);
        var scanner = new CodeScanClient(client, _root, "win-x64");

        string path = await scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None);
        Assert.True(File.Exists(Path.Combine(path, "win-x64", "CodeScan.exe")));
        Assert.True(File.Exists(Path.Combine(_root, "win-x64", "current.json")));
        Assert.Equal(2, handler.RequestedUris.Count);
        Assert.Empty(Directory.GetFiles(Path.Combine(_root, "win-x64"), "*.zip"));
    }

    [Fact]
    public async Task CachedScanner_ChecksLatestReleaseBeforeReusingWithoutDownloading()
    {
        byte[] archive = CreateArchive();
        var handler = ReleaseHandler(archive);
        using var client = new HttpClient(handler);
        string first = await new CodeScanClient(client, _root, "win-x64").EnsureAvailableAsync(_ => { }, CancellationToken.None);
        string second = await new CodeScanClient(client, _root, "win-x64").EnsureAvailableAsync(_ => { }, CancellationToken.None);

        Assert.Equal(first, second);
        Assert.Equal(2, handler.RequestedUris.Count(u => u.ToString() == CodeScanClient.LatestReleaseUrl));
        Assert.Single(handler.RequestedUris, u => u.ToString() == AssetUrl);
    }

    [Theory]
    [InlineData(2, 2, "2026-11-19T00:00:00Z")]
    [InlineData(1, 2, "2026-11-18T00:00:00Z")]
    [InlineData(1, 1, "2026-11-19T00:00:00Z")]
    public async Task ReleaseOrAssetChanges_InstallsUpdateBeforeReturning(long releaseId, long assetId, string updatedAt)
    {
        byte[] archive = CreateArchive();
        int checks = 0;
        var handler = new DelegateHttpMessageHandler((request, _) =>
            request.RequestUri!.ToString() == CodeScanClient.LatestReleaseUrl
                ? DelegateHttpMessageHandler.Json(++checks == 1 ? ReleaseJson(archive) : ReleaseJson(archive, releaseId, assetId, updatedAt))
                : ZipResponse(archive));
        using var client = new HttpClient(handler);
        var scanner = new CodeScanClient(client, _root, "win-x64");

        string first = await scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None);
        string second = await scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None);
        Assert.NotEqual(first, second);
        Assert.True(File.Exists(Path.Combine(second, "win-x64", "CodeScan.exe")));
        Assert.Equal(2, handler.RequestedUris.Count(u => u.ToString() == AssetUrl));
    }

    [Theory]
    [InlineData(HttpStatusCode.Forbidden)]
    [InlineData(HttpStatusCode.NotFound)]
    [InlineData(HttpStatusCode.ServiceUnavailable)]
    public async Task UpdateCheckFails_UsesCachedScanner(HttpStatusCode status)
    {
        byte[] archive = CreateArchive();
        int checks = 0;
        var handler = new DelegateHttpMessageHandler((request, _) =>
            request.RequestUri!.ToString() == CodeScanClient.LatestReleaseUrl
                ? DelegateHttpMessageHandler.Json(ReleaseJson(archive), ++checks == 1 ? HttpStatusCode.OK : status)
                : ZipResponse(archive));
        using var client = new HttpClient(handler);
        var scanner = new CodeScanClient(client, _root, "win-x64");
        string first = await scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None);
        var messages = new List<string>();

        Assert.Equal(first, await scanner.EnsureAvailableAsync(messages.Add, CancellationToken.None));
        Assert.Contains(messages, message => message.Contains("using the cached scanner"));
    }

    [Fact]
    public async Task NoCacheAndNoNetwork_FailsInsteadOfSkippingScan()
    {
        var handler = new DelegateHttpMessageHandler((_, _) => throw new HttpRequestException("offline"));
        using var client = new HttpClient(handler);
        var scanner = new CodeScanClient(client, _root, "win-x64");
        await Assert.ThrowsAsync<HttpRequestException>(() => scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None));
    }

    [Theory]
    [InlineData("invalid-zip")]
    [InlineData("missing-files")]
    [InlineData("digest-mismatch")]
    public async Task BrokenUpdate_PreservesPreviousCache(string failure)
    {
        byte[] archive = CreateArchive();
        byte[] update = failure == "invalid-zip" ? [1, 2, 3] : CreateArchive(complete: failure != "missing-files");
        int checks = 0;
        var handler = new DelegateHttpMessageHandler((request, _) =>
            request.RequestUri!.ToString() == CodeScanClient.LatestReleaseUrl
                ? DelegateHttpMessageHandler.Json(++checks == 1 ? ReleaseJson(archive)
                    : ReleaseJson(failure == "digest-mismatch" ? [99] : update, 2, 2))
                : ZipResponse(checks == 1 ? archive : update));
        using var client = new HttpClient(handler);
        var scanner = new CodeScanClient(client, _root, "win-x64");
        string first = await scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None);
        string manifest = await File.ReadAllTextAsync(Path.Combine(_root, "win-x64", "current.json"));

        Assert.Equal(first, await scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None));
        Assert.Equal(manifest, await File.ReadAllTextAsync(Path.Combine(_root, "win-x64", "current.json")));
        Assert.Single(Directory.GetDirectories(Path.Combine(_root, "win-x64")));
        Assert.Empty(Directory.GetFiles(Path.Combine(_root, "win-x64"), "*.zip"));
    }

    [Fact]
    public async Task IncompleteCache_DownloadsAgain()
    {
        byte[] archive = CreateArchive();
        var handler = ReleaseHandler(archive);
        using var client = new HttpClient(handler);
        var scanner = new CodeScanClient(client, _root, "win-x64");
        string first = await scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None);
        File.Delete(Path.Combine(first, "win-x64", "CodeScan.exe"));

        string second = await scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None);
        Assert.NotEqual(first, second);
        Assert.True(File.Exists(Path.Combine(second, "win-x64", "CodeScan.exe")));
    }

    [Fact]
    public async Task ConcurrentRequests_DownloadOnlyOnce()
    {
        byte[] archive = CreateArchive();
        var handler = ReleaseHandler(archive);
        using var client = new HttpClient(handler);
        var scanner = new CodeScanClient(client, _root, "win-x64");

        string[] paths = await Task.WhenAll(Enumerable.Range(0, 4)
            .Select(_ => scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None)));
        Assert.Single(paths.Distinct());
        Assert.Single(handler.RequestedUris, u => u.ToString() == AssetUrl);
    }

    [Fact]
    public async Task CancellationDuringUpdate_DoesNotFallBackToCache()
    {
        byte[] archive = CreateArchive();
        using var cancellation = new CancellationTokenSource();
        int checks = 0;
        var handler = new DelegateHttpMessageHandler((request, _) =>
        {
            if (request.RequestUri!.ToString() != CodeScanClient.LatestReleaseUrl) return ZipResponse(archive);
            if (++checks > 1)
            {
                cancellation.Cancel();
                throw new OperationCanceledException(cancellation.Token);
            }
            return DelegateHttpMessageHandler.Json(ReleaseJson(archive));
        });
        using var client = new HttpClient(handler);
        var scanner = new CodeScanClient(client, _root, "win-x64");
        await scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None);
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => scanner.EnsureAvailableAsync(_ => { }, cancellation.Token));
    }

    [Fact]
    public async Task MissingPlatformAsset_FailsWithoutCache()
    {
        var handler = new DelegateHttpMessageHandler((_, _) => DelegateHttpMessageHandler.Json("{\"id\":1,\"assets\":[]}"));
        using var client = new HttpClient(handler);
        var scanner = new CodeScanClient(client, _root, "win-x64");
        await Assert.ThrowsAsync<CodeScanningException>(() => scanner.EnsureAvailableAsync(_ => { }, CancellationToken.None));
    }

    private static DelegateHttpMessageHandler ReleaseHandler(byte[] archive) => new((request, _) =>
        request.RequestUri!.ToString() == CodeScanClient.LatestReleaseUrl
            ? DelegateHttpMessageHandler.Json(ReleaseJson(archive)) : ZipResponse(archive));

    private static string ReleaseJson(byte[] archive, long releaseId = 1, long assetId = 1, string updatedAt = "2026-11-18T00:00:00Z") =>
        $$"""
          {"id":{{releaseId}},"tag_name":"build-1-1","assets":[{"id":{{assetId}},"name":"win-x64.zip",
          "updated_at":"{{updatedAt}}","browser_download_url":"{{AssetUrl}}",
          "digest":"sha256:{{Convert.ToHexString(SHA256.HashData(archive))}}"}]}
          """;

    private static HttpResponseMessage ZipResponse(byte[] archive) => new(HttpStatusCode.OK) { Content = new ByteArrayContent(archive) };

    private static byte[] CreateArchive(bool complete = true)
    {
        using var stream = new MemoryStream();
        using (var zip = new ZipArchive(stream, ZipArchiveMode.Create, leaveOpen: true))
        {
            foreach (string path in new[] { "win-x64/CodeScan.exe", "win-x64/CodeScan.dll", "win-x64/CodeScan.runtimeconfig.json", "FilesToMoveToManaged.json" })
            {
                if (!complete && path.EndsWith("CodeScan.dll")) continue;
                using var writer = new StreamWriter(zip.CreateEntry(path).Open());
                writer.Write("[]");
            }
        }
        return stream.ToArray();
    }

    public void Dispose()
    {
        if (Directory.Exists(_root)) Directory.Delete(_root, recursive: true);
    }
}
