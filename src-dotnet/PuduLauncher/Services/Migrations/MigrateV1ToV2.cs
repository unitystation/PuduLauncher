using System.Text.Json.Nodes;

namespace PuduLauncher.Services.Migrations;

public class MigrateV1ToV2 : IPreferencesMigration
{
    public int FromVersion => 1;
    public int ToVersion => 2;

    public JsonObject Migrate(JsonObject preferences)
    {
        var launcher = preferences["launcher"] as JsonObject ?? new JsonObject();
        var servers = preferences["servers"] as JsonObject ?? new JsonObject();
        var installations = preferences["installations"] as JsonObject ?? new JsonObject();
        var sandbox = preferences["sandbox"] as JsonObject ?? new JsonObject();

        bool hasCategorizedData = preferences.ContainsKey("launcher")
            || preferences.ContainsKey("servers")
            || preferences.ContainsKey("installations");
        if (!hasCategorizedData || HasLegacyFlatFields(preferences))
        {
            launcher["isTtsEnabled"] ??= TakeOrDefault(preferences, "isTtsEnabled", JsonValue.Create(true)!);
            launcher["ignoreVersionUpdate"] ??= TakeOrDefault(preferences, "ignoreVersionUpdate", JsonValue.Create(0)!);
            servers["serverListApi"] ??= TakeOrDefault(preferences, "serverListApi", JsonValue.Create(string.Empty)!);
            servers["serverListFetchIntervalSeconds"] ??= TakeOrDefault(preferences, "serverListFetchIntervalSeconds", JsonValue.Create(10)!);
            installations["autoRemove"] ??= TakeOrDefault(preferences, "autoRemove", JsonValue.Create(false)!);
            installations["installationPath"] ??= TakeOrDefault(preferences, "installationPath", JsonValue.Create(string.Empty)!);
        }

        MoveProperty(launcher, "enableNanoSandbox", sandbox, "enabled");
        MoveProperty(launcher, "nanoSandboxMemoryLimitMb", sandbox, "memoryLimitMb");
        sandbox["backend"] ??= "Automatic";

        preferences["launcher"] = launcher;
        preferences["servers"] = servers;
        preferences["installations"] = installations;
        preferences["sandbox"] = sandbox;
        preferences["version"] = ToVersion;
        return preferences;
    }

    private static JsonNode TakeOrDefault(JsonObject source, string name, JsonNode fallback)
    {
        if (!source.TryGetPropertyValue(name, out JsonNode? value) || value is null)
        {
            return fallback;
        }

        source.Remove(name);
        return value;
    }

    private static bool HasLegacyFlatFields(JsonObject preferences)
    {
        return preferences.ContainsKey("isTtsEnabled")
            || preferences.ContainsKey("ignoreVersionUpdate")
            || preferences.ContainsKey("serverListApi")
            || preferences.ContainsKey("serverListFetchIntervalSeconds")
            || preferences.ContainsKey("autoRemove")
            || preferences.ContainsKey("installationPath");
    }

    private static void MoveProperty(JsonObject source, string sourceName, JsonObject destination, string destinationName)
    {
        if (!source.TryGetPropertyValue(sourceName, out JsonNode? value))
        {
            return;
        }

        source.Remove(sourceName);
        destination[destinationName] ??= value;
    }
}
