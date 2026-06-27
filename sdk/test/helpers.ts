// Shared helpers for the SDK black-box test suite.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";

const here = dirname(fileURLToPath(import.meta.url));

/** Absolute path to the SDK package root (sdk/). */
export const SDK_ROOT = resolve(here, "..");
/** Absolute path to the repository root. */
export const REPO_ROOT = resolve(SDK_ROOT, "..");

/** Path to the cortex binary under test (override with CORTEX_BIN). */
export function cortexBin(): string {
  return process.env.CORTEX_BIN ?? join(REPO_ROOT, "target", "debug", "cortex");
}

/** Whether a live OpenRouter key is present (gates live-turn assertions). */
export function hasOpenRouterKey(): boolean {
  const key = process.env.OPENROUTER_API_KEY;
  return typeof key === "string" && key.length > 0;
}

export interface Scratch {
  readonly dir: string;
  cleanup(): void;
}

/** Create an isolated scratch directory used as the server data dir and cwd. */
export function makeScratch(label: string): Scratch {
  const dir = mkdtempSync(join(tmpdir(), `cortex-sdk-${label}-`));
  return {
    dir,
    cleanup(): void {
      try {
        rmSync(dir, { recursive: true, force: true });
      } catch {
        /* best effort */
      }
    },
  };
}

export function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Assemble a synthetic API-key-shaped value from fragments (never a contiguous literal). */
export function syntheticApiKey(): string {
  return ["sk", "or-v1", randomUUID().replace(/-/g, "")].join("-");
}
