import { createContext, type PropsWithChildren, useContext, useEffect, useRef, useState } from "react";
import { TTS_STATUS } from "../constants/ttsStatus";
import { devBridge } from "../devtools/bridge";
import { EventListener } from "../pudu/events/event-listener";
import type { PuduEventMap, PuduEventType } from "../pudu/generated";
import { TtsApi, type TtsState } from "../pudu/generated";
import { useFeedbackContext } from "./FeedbackContextProvider";

export interface TtsStateContextValue {
    ttsState: TtsState | null;
    status: number | null;
    statusMessage: string | null;
    isLoadingState: boolean;
    installLogs: string[];
    loadStatus: () => Promise<TtsState | null>;
    clearInstallLogs: () => void;
}

const TtsStateContext = createContext<TtsStateContextValue | undefined>(undefined);

export function TtsStateContextProvider(props: PropsWithChildren) {
    const { children } = props;
    const { showError } = useFeedbackContext();
    const [ttsState, setTtsState] = useState<TtsState | null>(null);
    const [statusMessage, setStatusMessage] = useState<string | null>(null);
    const [isLoadingState, setIsLoadingState] = useState(true);
    const [installLogs, setInstallLogs] = useState<string[]>([]);
    const stateLoadedRef = useRef(false);
    const loadSeqRef = useRef(0);

    const loadStatus = async () => {
        const seq = ++loadSeqRef.current;
        const api = new TtsApi();
        const result = await api.getStatus();
        if (!result.success || !result.data) {
            showError({
                source: "frontend.tts.get-status",
                userMessage: "Failed to load TTS status.",
                code: "TTS_STATUS_FETCH_FAILED",
                technicalDetails: result.error ?? "Unknown backend error.",
            });
            return null;
        }

        // Ignore stale responses when a newer load has been started since (for
        // example the bootstrap load racing a load triggered by a status event).
        if (seq === loadSeqRef.current) {
            setTtsState(result.data);
            stateLoadedRef.current = true;
        }
        return result.data;
    };

    useEffect(() => {
        let isDisposed = false;

        void (async () => {
            try {
                await loadStatus();
            } catch (error: unknown) {
                if (!isDisposed) {
                    showError({
                        source: "frontend.tts.get-status",
                        userMessage: "Failed to load TTS status.",
                        code: "TTS_STATUS_FETCH_EXCEPTION",
                        technicalDetails: error instanceof Error ? error.toString() : String(error),
                    });
                }
            } finally {
                if (!isDisposed) {
                    setIsLoadingState(false);
                }
            }
        })();

        const eventListener = new EventListener();

        let unregisterInjector: (() => void) | undefined;
        if (devBridge) {
            unregisterInjector = devBridge.registerEventInjector((eventType, data) => {
                eventListener.injectEvent(eventType as PuduEventType, data as PuduEventMap[PuduEventType]);
            });
        }

        eventListener.on("tts:status-changed", (event) => {
            if (isDisposed) {
                return;
            }

            setStatusMessage(event.message ?? null);

            // If the full state has not loaded yet, merging onto null would
            // silently drop this status change (the cause of a missed startup
            // "server is running" notification). Re-fetch instead; the backend
            // already reflects the new status.
            if (!stateLoadedRef.current) {
                void loadStatus();
                return;
            }

            setTtsState((previous) => {
                if (previous === null) {
                    return previous;
                }

                return {
                    ...previous,
                    status: event.status,
                    errorMessage: event.status === TTS_STATUS.Error
                        ? event.message ?? previous.errorMessage
                        : null,
                };
            });

            if (event.status === TTS_STATUS.Installed
                || event.status === TTS_STATUS.NotInstalled
                || event.status === TTS_STATUS.Error) {
                void loadStatus();
            }
        });

        eventListener.on("tts:update-available", (event) => {
            if (isDisposed) {
                return;
            }

            setTtsState((previous) => {
                if (previous === null) {
                    return previous;
                }

                return {
                    ...previous,
                    updateAvailable: true,
                    latestVersion: event.latestVersion,
                };
            });
        });

        eventListener.on("tts:install-output", (event) => {
            if (isDisposed) {
                return;
            }

            setInstallLogs((previous) => {
                const next = [...previous, event.line];
                if (next.length <= 400) {
                    return next;
                }

                return next.slice(next.length - 400);
            });
        });

        return () => {
            isDisposed = true;
            unregisterInjector?.();
            eventListener.disconnect();
        };
    }, []);

    useEffect(() => {
        if (!devBridge) return;
        return devBridge.registerStateSource("tts", () => ({
            status: ttsState?.status ?? null,
            errorMessage: ttsState?.errorMessage ?? null,
            updateAvailable: ttsState?.updateAvailable ?? false,
            installedVersion: ttsState?.installedVersion ?? null,
            latestVersion: ttsState?.latestVersion ?? null,
            installLogsCount: installLogs.length,
            statusMessage,
            isLoadingState,
        }));
    }, [ttsState, statusMessage, isLoadingState, installLogs.length]);

    const clearInstallLogs = () => {
        setInstallLogs([]);
    };

    const value: TtsStateContextValue = {
        ttsState,
        status: ttsState?.status ?? null,
        statusMessage,
        isLoadingState,
        installLogs,
        loadStatus,
        clearInstallLogs,
    };

    return (
        <TtsStateContext.Provider value={value}>
            {children}
        </TtsStateContext.Provider>
    );
}

export function useTtsState() {
    const context = useContext(TtsStateContext);
    if (context === undefined) {
        throw new Error("useTtsState must be used within a TtsStateContextProvider.");
    }

    return context;
}
