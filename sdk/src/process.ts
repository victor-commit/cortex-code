// Process-management helpers for launching and stopping a local `cortex serve`.
//
// Stopping is by the exact listener PID: we resolve the process bound to the loopback port via
// `lsof -tiTCP:<port> -sTCP:LISTEN` and signal that PID (falling back to the spawned child PID),
// so a server is never orphaned even if the binder differs from the launcher. This mirrors the
// mission's exact-PID process hygiene rule for ports 3200-3299.

import { spawnSync, type ChildProcess } from "node:child_process";
import { createServer } from "node:net";

export const PORT_RANGE_MIN = 3200;
export const PORT_RANGE_MAX = 3299;

export function assertPortInRange(port: number): void {
  if (!Number.isInteger(port) || port < PORT_RANGE_MIN || port > PORT_RANGE_MAX) {
    throw new Error(
      `port ${port} is outside the allowed loopback range ${PORT_RANGE_MIN}-${PORT_RANGE_MAX}`,
    );
  }
}

/** Resolve the PID of the process listening on a TCP port, if any. */
export function listenerPid(port: number): number | undefined {
  const result = spawnSync("lsof", [`-tiTCP:${port}`, "-sTCP:LISTEN"], { encoding: "utf8" });
  if (result.status !== 0 || !result.stdout) return undefined;
  const first = result.stdout.trim().split(/\s+/)[0] ?? "";
  const pid = Number.parseInt(first, 10);
  return Number.isFinite(pid) ? pid : undefined;
}

export function isAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForExit(pid: number, timeoutMs: number): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!isAlive(pid)) return true;
    await delay(50);
  }
  return !isAlive(pid);
}

export interface StopOptions {
  timeoutMs?: number;
}

/**
 * Stop a launched server by its exact listener PID. Resolves the PID bound to `port`, sends
 * SIGTERM, waits for exit, then escalates to SIGKILL. Also signals the spawned child if it differs.
 */
export async function stopServerProcess(
  proc: ChildProcess,
  port: number,
  options: StopOptions = {},
): Promise<void> {
  const timeoutMs = options.timeoutMs ?? 5000;
  const bound = listenerPid(port);
  const target = bound ?? proc.pid;

  if (target !== undefined && isAlive(target)) {
    try {
      process.kill(target, "SIGTERM");
    } catch {
      /* already gone */
    }
  }
  if (proc.pid !== undefined && proc.pid !== target && isAlive(proc.pid)) {
    try {
      process.kill(proc.pid, "SIGTERM");
    } catch {
      /* already gone */
    }
  }

  if (target !== undefined) {
    const exited = await waitForExit(target, timeoutMs);
    if (!exited && isAlive(target)) {
      try {
        process.kill(target, "SIGKILL");
      } catch {
        /* already gone */
      }
      await waitForExit(target, 1000);
    }
  }
}

/** Test whether a loopback TCP port is currently free to bind. */
export function isPortFree(port: number, hostname = "127.0.0.1"): Promise<boolean> {
  return new Promise((resolve) => {
    const server = createServer();
    server.once("error", () => resolve(false));
    server.once("listening", () => {
      server.close(() => resolve(true));
    });
    server.listen(port, hostname);
  });
}

/** Find a free port in the reserved 3200-3299 loopback range, skipping `exclude`. */
export async function pickPort(
  hostname = "127.0.0.1",
  exclude?: ReadonlySet<number>,
): Promise<number> {
  for (let port = PORT_RANGE_MIN; port <= PORT_RANGE_MAX; port += 1) {
    if (exclude?.has(port)) continue;
    if (await isPortFree(port, hostname)) return port;
  }
  throw new Error(`no free port available in ${PORT_RANGE_MIN}-${PORT_RANGE_MAX}`);
}
