// Detects React Compiler "bailouts" (components/hooks it refuses to optimize)
// and fails when a NEW one is introduced, compared to a committed baseline.
//
// A bailout is not an ESLint error and does not break the build: the code still
// runs, it is just not memoized. That is usually only a perf miss, but for a
// context/provider it is dangerous (an unstable context value re-renders every
// consumer and can cause re-render/remount loops).
//
//   npm run check:react-compiler              compare against the baseline (CI)
//   npm run check:react-compiler -- --update  rewrite the baseline on purpose
//
// The baseline key is "path | category | message" (no line number) so unrelated
// edits that shift lines do not cause churn.

import { readFileSync, writeFileSync, readdirSync, existsSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import babel from "@babel/core";

const { transformAsync } = babel;

const repoRoot = fileURLToPath(new URL("..", import.meta.url));
const SRC_DIR = join(repoRoot, "src");
const GENERATED_DIR = join(SRC_DIR, "pudu", "generated");
const BASELINE_PATH = join(repoRoot, "react-compiler-bailouts.json");

const IGNORE_FILE = /\.(stories|test|spec)\.[tj]sx?$/;

function collectSourceFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (full === GENERATED_DIR) continue;
      out.push(...collectSourceFiles(full));
    } else if (/\.tsx?$/.test(entry.name) && !IGNORE_FILE.test(entry.name)) {
      out.push(full);
    }
  }
  return out;
}

function toRelKey(absPath) {
  return relative(repoRoot, absPath).split(sep).join("/");
}

async function bailoutsForFile(absPath) {
  const rel = toRelKey(absPath);
  const found = [];
  try {
    await transformAsync(readFileSync(absPath, "utf8"), {
      filename: absPath,
      babelrc: false,
      configFile: false,
      parserOpts: { plugins: ["jsx", "typescript"] },
      plugins: [
        [
          "babel-plugin-react-compiler",
          {
            logger: {
              logEvent(_filename, event) {
                if (event?.kind !== "CompileError") return;
                const options = event.detail?.options ?? {};
                const category = options.category ?? "Unknown";
                const reason =
                  options.reason ?? event.detail?.reason ?? "unknown reason";
                const loc = event.detail?.loc?.start ?? event.fnLoc?.start;
                found.push({
                  key: `${rel} | ${category} | ${reason}`,
                  line: loc?.line,
                  column: loc?.column,
                });
              },
            },
          },
        ],
      ],
    });
  } catch (error) {
    console.error(`Failed to compile ${rel}: ${error.message}`);
    process.exitCode = 1;
  }
  return found;
}

async function main() {
  const files = collectSourceFiles(SRC_DIR);
  const all = (await Promise.all(files.map(bailoutsForFile))).flat();

  // Dedupe by key, remembering one representative location for display.
  const current = new Map();
  for (const bailout of all) {
    if (!current.has(bailout.key)) current.set(bailout.key, bailout);
  }
  const currentKeys = [...current.keys()].sort();

  if (process.argv.includes("--update")) {
    writeFileSync(BASELINE_PATH, JSON.stringify(currentKeys, null, 2) + "\n");
    console.log(
      `Updated ${toRelKey(BASELINE_PATH)} with ${currentKeys.length} known bailout(s).`,
    );
    return;
  }

  if (!existsSync(BASELINE_PATH)) {
    console.error(
      `No baseline at ${toRelKey(BASELINE_PATH)}. Create it with: npm run check:react-compiler -- --update`,
    );
    process.exitCode = 1;
    return;
  }

  const baseline = new Set(JSON.parse(readFileSync(BASELINE_PATH, "utf8")));
  const added = currentKeys.filter((key) => !baseline.has(key));
  const removed = [...baseline].filter((key) => !current.has(key)).sort();

  if (removed.length) {
    console.log(
      `${removed.length} baseline bailout(s) no longer occur. Tidy with: npm run check:react-compiler -- --update`,
    );
    for (const key of removed) console.log(`  fixed: ${key}`);
  }

  if (added.length) {
    console.error(
      `\nReact Compiler bailout check FAILED: ${added.length} new bailout(s).`,
    );
    console.error(
      "These will not be optimized by React Compiler. For a provider this can cause re-render loops.\n",
    );
    for (const key of added) {
      const { line, column } = current.get(key);
      const at = line ? ` (around ${line}${column != null ? `:${column}` : ""})` : "";
      console.error(`  ${key}${at}`);
    }
    console.error(
      "\nFix the pattern, or if it is genuinely unavoidable update the baseline:\n  npm run check:react-compiler -- --update",
    );
    process.exitCode = 1;
    return;
  }

  console.log(
    `React Compiler bailout check passed (${currentKeys.length} known, 0 new).`,
  );
}

await main();
