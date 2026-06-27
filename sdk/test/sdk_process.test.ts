// VAL-GAP-SDK-004: the SDK process launcher starts and stops the loopback server safely.

import { test } from "node:test";
import assert from "node:assert/strict";

import { createCortexServer } from "../src/server.ts";
import { createCortexClient } from "../src/client.ts";
import { isAlive, isPortFree, listenerPid, assertPortInRange } from "../src/process.ts";
import { cortexBin, makeScratch } from "./helpers.ts";

test("SDK launches cortex serve on a 3200-3299 port and stops the exact listener PID", async () => {
  const scratch = makeScratch("process");
  const server = await createCortexServer({
    bin: cortexBin(),
    cwd: scratch.dir,
    env: { CORTEX_DATA_DIR: scratch.dir, CORTEX_NO_NET: "1" },
    timeoutMs: 20000,
  });

  try {
    assert.ok(server.port >= 3200 && server.port <= 3299, `port ${server.port} must be in 3200-3299`);

    const client = createCortexClient({ baseUrl: server.url });
    const health = await client.health();
    assert.equal(health.status, "ok");

    const pid = listenerPid(server.port);
    assert.equal(pid, server.pid, "resolved listener PID matches the reported server PID");
    assert.ok(pid !== undefined && isAlive(pid), "listener PID is alive while serving");
  } finally {
    await server.close();
  }

  assert.equal(listenerPid(server.port), undefined, "no listener remains on the port");
  assert.equal(await isPortFree(server.port), true, "port is released after close");
  assert.equal(isAlive(server.pid), false, "the listener PID is no longer alive");

  scratch.cleanup();
});

test("the launcher refuses ports outside the reserved 3200-3299 range", () => {
  assert.throws(() => assertPortInRange(3100), /3200-3299/);
  assert.throws(() => assertPortInRange(8080), /3200-3299/);
  assert.doesNotThrow(() => assertPortInRange(3247));
});
