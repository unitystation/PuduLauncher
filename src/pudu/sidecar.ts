import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

let cachedPort: number | null = null;
let portPromise: Promise<number> | null = null;

const isTauri = "__TAURI_INTERNALS__" in window;

const SIDECAR_TIMEOUT_MS = 60_000;

/**
 * Returns the sidecar port.
 *
 * In Tauri mode, tries the Rust command first (in case the event already fired),
 * then waits for the `sidecar-ready` Tauri event.
 *
 * In standalone dev mode (no Tauri), reads from VITE_SIDECAR_PORT env var.
 */
export function getSidecarPort(): Promise<number> {
    if (cachedPort) return Promise.resolve(cachedPort);

    if (!isTauri) {
        const envPort = import.meta.env.VITE_SIDECAR_PORT;
        if (!envPort) {
            return Promise.reject(new Error("Running outside Tauri: set VITE_SIDECAR_PORT in .env.local"));
        }
        cachedPort = Number(envPort);
        return Promise.resolve(cachedPort);
    }

    // Deduplicate concurrent callers, one shared promise.
    if (!portPromise) {
        portPromise = discoverPort().catch((error: unknown) => {
            portPromise = null;
            throw error;
        });
    }

    return portPromise;
}

async function discoverPort(): Promise<number> {
    return new Promise<number>((resolve, reject) => {
        let settled = false;
        let unlisten: (() => void) | undefined;

        const timeout = setTimeout(() => {
            finishWithError(new Error("Sidecar did not start in time"));
        }, SIDECAR_TIMEOUT_MS);

        const finishWithPort = (port: number) => {
            if (settled) return;
            settled = true;
            clearTimeout(timeout);
            unlisten?.();
            cachedPort = port;
            resolve(port);
        };

        const finishWithError = (error: Error) => {
            if (settled) return;
            settled = true;
            clearTimeout(timeout);
            unlisten?.();
            reject(error);
        };

        // Subscribe before checking the command. This closes the gap where the
        // backend can emit its one-shot ready event after a failed command check
        // but before the listener is registered.
        void listen<number>("sidecar-ready", (event) => {
            finishWithPort(event.payload);
        }).then((stopListening) => {
            unlisten = stopListening;
            if (settled) {
                unlisten();
                return;
            }

            // The sidecar may already have started before the listener was
            // installed. Query again now that the event subscription is live.
            void invoke<number>("get_sidecar_port")
                .then(finishWithPort)
                .catch(() => {
                    // The event listener will resolve once startup completes.
                });
        }).catch((error: unknown) => {
            finishWithError(error instanceof Error ? error : new Error(String(error)));
        });
    });
}

export async function getSidecarBaseUrl(): Promise<string> {
    const port = await getSidecarPort();
    return `http://localhost:${port}`;
}

export async function getSidecarWsUrl(): Promise<string> {
    const port = await getSidecarPort();
    return `ws://localhost:${port}`;
}
