import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
const reportedCompilerBailouts = new Set<string>();

// Logs components React Compiler skips ("bailout"): not memoized, not flagged by
// ESLint. Mostly a perf miss, but dangerous for a provider (unstable context
// value -> consumer re-render loops). category: "Refs"/"Globals"/etc = real
// Rules of React violation; "Todo" = unsupported syntax (correct but unoptimized).
function logReactCompilerBailout(filename: string | null, event: any) {
  if (event?.kind !== "CompileError") return;

  const options = event.detail?.options ?? {};
  const category: string = options.category ?? "Unknown";
  const reason: string =
    options.reason ?? event.detail?.reason ?? "unknown reason";

  const fallbackLoc = event.detail?.loc?.start ?? event.fnLoc?.start;
  const occurrences: Array<{ line?: number; column?: number; message: string }> =
    options.details?.length
      ? options.details.map((detail: any) => ({
        line: detail.loc?.start?.line,
        column: detail.loc?.start?.column,
        message: detail.message ?? reason,
      }))
      : [{ line: fallbackLoc?.line, column: fallbackLoc?.column, message: reason }];

  for (const { line, column, message } of occurrences) {
    const where = `${filename ?? "<unknown>"}:${line ?? "?"}${column != null ? `:${column}` : ""
      }`;
    const key = `${where}|${message}`;
    if (reportedCompilerBailouts.has(key)) continue;
    reportedCompilerBailouts.add(key);
    console.warn(`[react-compiler] skipped [${category}] ${where} -> ${message}`);
  }
}

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [
    react({
      babel: {
        plugins: [
          [
            "babel-plugin-react-compiler",
            {
              logger: { logEvent: logReactCompilerBailout },
            },
          ],
        ],
      },
    }),
  ],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
        protocol: "ws",
        host,
        port: 1421,
      }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
