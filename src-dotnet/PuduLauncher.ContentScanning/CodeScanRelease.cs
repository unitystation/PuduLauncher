using System.Text.Json.Serialization;

namespace PuduLauncher.ContentScanning;

internal sealed class CodeScanRelease
{
    public long Id { get; set; }
    [JsonPropertyName("tag_name")]
    public string TagName { get; set; } = "";
    public List<CodeScanAsset> Assets { get; set; } = [];
}

internal sealed class CodeScanAsset
{
    public long Id { get; set; }
    public string Name { get; set; } = "";
    [JsonPropertyName("browser_download_url")]
    public string DownloadUrl { get; set; } = "";
    [JsonPropertyName("updated_at")]
    public string UpdatedAt { get; set; } = "";
    public string? Digest { get; set; }
}

internal sealed record CodeScanDownload(string TagName, long ReleaseId, long AssetId, string AssetUpdatedAt,
    string Url, string? Digest);

internal sealed class CodeScanCacheEntry
{
    public long ReleaseId { get; set; }
    public long AssetId { get; set; }
    public string AssetUpdatedAt { get; set; } = "";
    public string DirectoryName { get; set; } = "";
}

[JsonSourceGenerationOptions(PropertyNameCaseInsensitive = true)]
[JsonSerializable(typeof(CodeScanRelease))]
[JsonSerializable(typeof(CodeScanCacheEntry))]
[JsonSerializable(typeof(string[]))]
internal partial class CodeScanJsonContext : JsonSerializerContext;
