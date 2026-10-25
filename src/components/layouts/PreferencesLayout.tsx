import { useState } from "react";
import { Alert, Box, Button, CircularProgress, Modal, ModalDialog, Stack, Typography } from "@mui/joy";
import { preferencesSchema } from "../../pudu/generated";
import { usePreferencesContext } from "../../contextProviders/PreferencesContextProvider";
import {
    themeIdToPreferenceValue,
    themePreferenceValueToThemeId,
    useThemeContext,
} from "../../contextProviders/ThemeProvider";
import PreferenceCategory from "../molecules/preferences/PreferenceCategory";

export default function PreferencesLayout() {
    const { preferences, isLoading, isSaving, updateField, moveInstallationPath } = usePreferencesContext();
    const { changeThemeId } = useThemeContext();
    const [sandboxToggleDialog, setSandboxToggleDialog] = useState<"enable" | "disable" | null>(null);

    const fieldOverridesByCategory: Record<string, Record<string, (value: unknown) => void | Promise<void>>> = {
        launcher: {
            theme: (value) => {
                const themeId = themePreferenceValueToThemeId(value);
                if (themeId) {
                    changeThemeId(themeId);
                    updateField("launcher", "theme", themeIdToPreferenceValue(themeId));
                    return;
                }

                if (typeof value === "string") {
                    updateField("launcher", "theme", value);
                }
            },
        },
        installations: {
            installationPath: (value) => moveInstallationPath(value as string),
        },
        sandbox: {
            enabled: (value) => {
                if (value === true && preferences?.sandbox.enabled !== true) {
                    setSandboxToggleDialog("enable");
                    return;
                }
                if (value === false && preferences?.sandbox.enabled === true) {
                    setSandboxToggleDialog("disable");
                    return;
                }

                updateField("sandbox", "enabled", value);
            },
        },
    };

    return (
        <Box sx={{ height: "100%", minWidth: 0, display: "flex", flexDirection: "column" }}>
            <Stack direction="row" justifyContent="space-between" alignItems="center" sx={{ p: 3, pb: 2 }}>
                <Stack spacing={0.5}>
                    <Typography level="h1">Preferences</Typography>
                    <Typography level="body-sm" sx={{ color: "text.secondary" }}>
                        {isSaving ? "Saving..." : "Changes are saved automatically"}
                    </Typography>
                </Stack>
            </Stack>

            <Stack spacing={2} sx={{ px: 3, pb: 3, minHeight: 0, overflowY: "auto" }}>
                {isLoading && (
                    <Stack direction="row" spacing={1} alignItems="center" sx={{ py: 4, justifyContent: "center" }}>
                        <CircularProgress size="sm" />
                        <Typography level="body-sm">Loading preferences...</Typography>
                    </Stack>
                )}

                {preferences &&
                    preferencesSchema.map((category) => (
                        <PreferenceCategory
                            key={category.key}
                            schema={category}
                            preferences={preferences}
                            updateField={updateField}
                            fieldOverrides={fieldOverridesByCategory[category.key]}
                        />
                    ))}
            </Stack>

            <Modal
                open={sandboxToggleDialog !== null}
                onClose={() => setSandboxToggleDialog(null)}
            >
                <ModalDialog>
                    <Stack spacing={2}>
                        <Typography level="h3">
                            {sandboxToggleDialog === "enable"
                                ? "Experimental Feature: Sandboxing"
                                : "Disable sandboxing?"}
                        </Typography>
                        <Alert color="warning" variant="soft">
                            {sandboxToggleDialog === "enable"
                                ? "Sandbox support is still experimental, and may prevent some builds from launching or working correctly. This will be enabled by default in the future when UnityStation builds fully work within the bounds of sandboxing, but for now; only enable it if you want to test it out as a developer."
                                : "Sandboxing adds a layer of security on top of codescans when running the custom unitystation builds. Disable it only if you know what you are doing, and trust the builds you are running."}
                        </Alert>
                        <Stack direction="row" spacing={1} justifyContent="flex-end">
                            <Button variant="outlined" onClick={() => setSandboxToggleDialog(null)}>
                                {sandboxToggleDialog === "enable" ? "Keep disabled" : "Keep enabled"}
                            </Button>
                            <Button
                                color="warning"
                                onClick={() => {
                                    const enabled = sandboxToggleDialog === "enable";
                                    setSandboxToggleDialog(null);
                                    updateField("sandbox", "enabled", enabled);
                                }}
                            >
                                {sandboxToggleDialog === "enable" ? "Enable sandbox" : "Disable sandbox"}
                            </Button>
                        </Stack>
                    </Stack>
                </ModalDialog>
            </Modal>
        </Box>
    );
}
