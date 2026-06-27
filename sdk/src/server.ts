// Launch and manage a local `cortex serve` process from the SDK.
//
// `createCortexServer` spawns the `cortex` binary directly (no shell wrapper) on a loopback port in
// the reserved 3200-3299 range, waits for `/health` to pass (or fails fast if the process exits
// early), and returns a handle whose `close()` stops the exact listener PID and releases the port.

import { spawn, type ChildProcess } from "node:child_process";
import {
  PORT_RANGE_MAX,
  PORT_RANGE_MIN,
  assertPortInRange,
  isAlive,
  listenerPid,
  pickPort,
  stopServerProcess,
} from "./process.ts";

export interface CortexServerOptions {
  /** Path to the cortex binary. Defaults to `cortex` on PATH. */
  bin?: string;
  /** Loopback hostname to bind. Defaults to 127.0.0.1. */
  hostname?: string;
  /** Port in 3200-3299. When omitted a free port in range is chosen. */
  port?: number;
  /** Require `Authorization: Bearer <token>` on protected routes. */
  token?: string;
  /** Working directory for the server process. */
  cwd?: string;
  /** Extra environment variables for the server process. */
  env?: Record<string, string>;
  /** Readiness timeout in milliseconds. Defaults to 10000. */
  timeoutMs?: number;
  /** Abort signal that stops the server if triggered before/while starting. */
  signal?: AbortSignal;
}

export interface CortexServer {
  /** Base URL, e.g. `http://127.0.0.1:3247`. */
  readonly url: string;
  readonly hostname: string;
  readonly port: number;
  /** The exact listener PID bound to the port. */
  readonly pid: number;
  readonly process: ChildProcess;
  /** Stop the server by exact listener PID and wait for the port to be released. */
  close(): Promise<void>;
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const BIND_CONFLICT = /in use|EADDRINUSE|os error 98|AddrInUse/i;

interface LaunchAttempt {
  spawnError?: Error;
  /** Set when the launch should be retried on a different port (port already taken). */
  retry?: string;
  server?: CortexServer;
}

export async function createCortexServer(options: CortexServerOptions = {}): Promise<CortexServer> {
  const hostname = options.hostname ?? "127.0.0.1";
  const bin = options.bin ?? "cortex";
  const timeoutMs = options.timeoutMs ?? 10000;
  const tried = new Set<number>();
  let lastReason = "";

  // Picking a free port and spawning are not atomic, so concurrent launchers can target the same
  // port. We verify the spawned process actually owns the listener and retry on the next free port
  // when it loses the race (or the chosen port is already bound).
  for (let attempt = 0; attempt < PORT_RANGE_MAX - PORT_RANGE_MIN + 1; attempt += 1) {
    const port =
      attempt === 0 && options.port !== undefined ? options.port : await pickPort(hostname, tried);
    tried.add(port);
    assertPortInRange(port);

    const result = await launchOnce(bin, hostname, port, timeoutMs, options);
    if (result.spawnError) {
      throw new Error(`failed to spawn ${bin}: ${result.spawnError.message}`);
    }
    if (result.server) return result.server;
    lastReason = result.retry ?? "unknown launch failure";
    if (options.port !== undefined) break; // An explicit port must not silently move.
  }

  throw new Error(
    `could not start cortex serve in ${PORT_RANGE_MIN}-${PORT_RANGE_MAX}: ${lastReason}`,
  );
}

async function launchOnce(
  bin: string,
  hostname: string,
  port: number,
  timeoutMs: number,
  options: CortexServerOptions,
): Promise<LaunchAttempt> {
  const args = ["serve", `--hostname=${hostname}`, `--port=${port}`];
  if (options.token) args.push(`--token=${options.token}`);

  const proc = spawn(bin, args, {
    cwd: options.cwd,
    env: { ...process.env, ...options.env },
    stdio: ["ignore", "pipe", "pipe"],
  });

  let stderr = "";
  let stdout = "";
  proc.stderr?.on("data", (chunk: Buffer) => {
    stderr += chunk.toString();
  });
  proc.stdout?.on("data", (chunk: Buffer) => {
    stdout += chunk.toString();
  });

  let exitCode: number | null = null;
  let spawnError: Error | undefined;
  proc.on("exit", (code) => {
    exitCode = code ?? -1;
  });
  proc.on("error", (error) => {
    spawnError = error;
  });

  const url = `http://${hostname}:${port}`;
  const deadline = Date.now() + timeoutMs;
  const healthHeaders: Record<string, string> = options.token
    ? { authorization: `Bearer ${options.token}` }
    : {};

  const killOwn = (): void => {
    if (proc.pid !== undefined && isAlive(proc.pid)) {
      try {
        proc.kill("SIGKILL");
      } catch {
        /* already gone */
      }
    }
  };

  for (;;) {
    if (spawnError) return { spawnError };
    if (exitCode !== null) {
      const log = `${stderr}${stdout}`;
      if (BIND_CONFLICT.test(log)) return { retry: `port ${port} already in use` };
      // A non-bind early exit is a real failure (bad flags, config error, ...).
      return { spawnError: new Error(`cortex serve exited early (code ${exitCode})\n${log}`) };
    }
    if (options.signal?.aborted) {
      await stopServerProcess(proc, port);
      throw new Error("aborted while waiting for cortex serve to start");
    }

    let healthy = false;
    try {
      const response = await fetch(`${url}/health`, { headers: healthHeaders });
      healthy = response.ok;
    } catch {
      /* not listening yet */
    }

    if (healthy) {
      const owner = listenerPid(port);
      if (owner !== undefined && owner === proc.pid) {
        return { server: buildServer(url, hostname, port, owner, proc) };
      }
      if (exitCode !== null || (owner !== undefined && owner !== proc.pid)) {
        // Another process owns this port; relinquish only our own child and retry elsewhere.
        killOwn();
        return { retry: `port ${port} owned by pid ${owner ?? "?"}, not ${proc.pid ?? "?"}` };
      }
      // Healthy but the listener PID has not resolved yet; keep polling briefly.
    }

    if (Date.now() > deadline) {
      await stopServerProcess(proc, port);
      return { spawnError: new Error(`timed out waiting for cortex serve on ${url} after ${timeoutMs}ms\n${stderr}`) };
    }
    await delay(100);
  }
}

function buildServer(
  url: string,
  hostname: string,
  port: number,
  pid: number,
  proc: ChildProcess,
): CortexServer {
  return {
    url,
    hostname,
    port,
    pid,
    process: proc,
    async close(): Promise<void> {
      await stopServerProcess(proc, port);
    },
  };
}
