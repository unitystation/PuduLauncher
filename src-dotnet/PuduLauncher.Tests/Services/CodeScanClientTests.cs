using System.IO.Compression;
using System.Net;
using System.Security.Cryptography;
using PuduLauncher.ContentScanning;

namespace PuduLauncher.Tests.Services;

public sealed class CodeScanClientTests : IDisposable
{
    private static readonly TimeSpan ShortTimeout = TimeSpan.FromMilliseconds(200);
    private readonly string _root = Path.Combine(Path.GetTempPath(), "PuduCodeScanTests", Guid.NewGuid().ToString("N"));

    [Theory]
    [InlineData(ApiFailure.Forbidden)]
    [InlineData(ApiFailure.Offline)]
    [InlineData(ApiFailure.Hangs)]
    public async Task GitHubApiDown_UsesCachedScanner(ApiFailure failure)
    {
        var github = new FakeGitHub { Archive = CreateArchive("v1") };
        await EnsureScannerAsync(github);
        github.ApiFailure = failure;
        github.Archive = CreateArchive("v2");

        string scanner = await EnsureScannerAsync(github);

        Assert.Equal("v1", ReadScannerVersion(scanner));
    }

    [Theory]
    [InlineData(ApiFailure.Forbidden)]
    [InlineData(ApiFailure.Offline)]
    [InlineData(ApiFailure.Hangs)]
    public async Task GitHubApiDownWithoutCache_DownloadsLatestRelease(ApiFailure failure)
    {
        var github = new FakeGitHub { Archive = CreateArchive("v1"), ApiFailure = failure };

        string scanner = await EnsureScannerAsync(github);

        Assert.Equal("v1", ReadScannerVersion(scanner));
    }

    [Fact]
    public async Task UnverifiedScanner_IsReplacedOnceGitHubApiRecovers()
    {
        var github = new FakeGitHub { Archive = CreateArchive("v1"), ApiFailure = ApiFailure.Forbidden };
        string unverified = await EnsureScannerAsync(github);
        github.ApiFailure = ApiFailure.None;

        string verified = await EnsureScannerAsync(github);

        Assert.NotEqual(unverified, verified);
        Assert.False(Directory.Exists(unverified));
    }

    [Fact]
    public async Task OfflineWithoutCache_FailsInsteadOfSkippingScan()
    {
        var github = new FakeGitHub { Archive = CreateArchive("v1"), ApiFailure = ApiFailure.Offline, DownloadsFail = true };

        await Assert.ThrowsAsync<HttpRequestException>(() => EnsureScannerAsync(github));
    }

    [Theory]
    [InlineData("invalid-zip")]
    [InlineData("digest-mismatch")]
    [InlineData("missing-executable")]
    public async Task BrokenRelease_KeepsPreviousScanner(string breakage)
    {
        var github = new FakeGitHub { Archive = CreateArchive("v1") };
        await EnsureScannerAsync(github);
        github.ReleaseId = 2;
        github.Archive = breakage switch
        {
            "invalid-zip" => [1, 2, 3],
            "missing-executable" => CreateArchive("v2", includeExecutable: false),
            _ => CreateArchive("v2")
        };
        github.CorruptDigest = breakage == "digest-mismatch";

        string scanner = await EnsureScannerAsync(github);

        Assert.Equal("v1", ReadScannerVersion(scanner));
    }

    [Fact]
    public async Task NewRelease_ReplacesPreviousScanner()
    {
        var github = new FakeGitHub { Archive = CreateArchive("v1") };
        string previous = await EnsureScannerAsync(github);
        github.ReleaseId = 2;
        github.Archive = CreateArchive("v2");

        string scanner = await EnsureScannerAsync(github);

        Assert.Equal("v2", ReadScannerVersion(scanner));
        Assert.False(Directory.Exists(previous));
    }

    [Fact]
    public async Task UserCancels_StopsInsteadOfUsingCachedScanner()
    {
        var github = new FakeGitHub { Archive = CreateArchive("v1") };
        await EnsureScannerAsync(github);
        github.ApiFailure = ApiFailure.Hangs;
        using var cancellation = new CancellationTokenSource(ShortTimeout);
        using var client = new HttpClient(github);
        var codeScan = new CodeScanClient(client, _root, "win-x64", TimeSpan.FromMinutes(1));

        await Assert.ThrowsAnyAsync<OperationCanceledException>(
            () => codeScan.EnsureAvailableAsync(_ => { }, cancellation.Token));
    }

    private async Task<string> EnsureScannerAsync(FakeGitHub github)
    {
        using var client = new HttpClient(github, disposeHandler: false);
        var codeScan = new CodeScanClient(client, _root, "win-x64", ShortTimeout);
        return await codeScan.EnsureAvailableAsync(_ => { }, CancellationToken.None);
    }

    private static string ReadScannerVersion(string scannerDirectory) =>
        File.ReadAllText(Path.Combine(scannerDirectory, "win-x64", "CodeScan.exe"));

    private static byte[] CreateArchive(string version, bool includeExecutable = true)
    {
        using var stream = new MemoryStream();
        using (var zip = new ZipArchive(stream, ZipArchiveMode.Create, leaveOpen: true))
        {
            if (includeExecutable) WriteEntry(zip, "win-x64/CodeScan.exe", version);
            WriteEntry(zip, "FilesToMoveToManaged.json", "[]");
        }
        return stream.ToArray();
    }

    private static void WriteEntry(ZipArchive zip, string path, string content)
    {
        using var writer = new StreamWriter(zip.CreateEntry(path).Open());
        writer.Write(content);
    }

    public void Dispose()
    {
        if (Directory.Exists(_root)) Directory.Delete(_root, recursive: true);
    }

    public enum ApiFailure { None, Forbidden, Offline, Hangs }

    private sealed class FakeGitHub : HttpMessageHandler
    {
        public byte[] Archive { get; set; } = [];
        public long ReleaseId { get; set; } = 1;
        public ApiFailure ApiFailure { get; set; }
        public bool CorruptDigest { get; set; }
        public bool DownloadsFail { get; set; }

        protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken ct)
        {
            if (request.RequestUri!.Host != "api.github.com")
            {
                if (DownloadsFail) throw new HttpRequestException("offline");
                return new HttpResponseMessage(HttpStatusCode.OK) { Content = new ByteArrayContent(Archive) };
            }

            switch (ApiFailure)
            {
                case ApiFailure.Forbidden:
                    return new HttpResponseMessage(HttpStatusCode.Forbidden);
                case ApiFailure.Offline:
                    throw new HttpRequestException("offline");
                case ApiFailure.Hangs:
                    await Task.Delay(Timeout.Infinite, ct);
                    break;
            }

            string digest = CorruptDigest ? new string('0', 64) : Convert.ToHexString(SHA256.HashData(Archive));
            string json = $$"""
                {"id":{{ReleaseId}},"tag_name":"v{{ReleaseId}}","assets":[{"id":{{ReleaseId}},"name":"win-x64.zip",
                "updated_at":"2026-11-18T00:00:00Z",
                "browser_download_url":"https://github.com/unitystation/codescan/releases/download/v{{ReleaseId}}/win-x64.zip",
                "digest":"sha256:{{digest}}"}]}
                """;
            return new HttpResponseMessage(HttpStatusCode.OK) { Content = new StringContent(json) };
        }
    }
}
