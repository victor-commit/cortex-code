// VAL-GAP-SDK-005: generated SDK types and runtime validators match the served OpenAPI document
// with no undocumented route drift. Network-safe: only the local loopback server is contacted.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { createCortexServer } from "../src/server.ts";
import { createCortexClient, CortexApiError } from "../src/client.ts";
import { OPERATIONS } from "../src/gen/operations.ts";
import type { OperationId } from "../src/gen/types.ts";
import { EVENT_FAMILIES, OPENAPI_SCHEMA_NAMES, REQUIRED_FIELDS } from "../src/gen/schemas.ts";
import { SDK_ROOT, cortexBin, makeScratch } from "./helpers.ts";

interface LiveSchema {
  required?: string[];
}
interface LiveOperation {
  operationId?: string;
}

test("generated SDK matches the served OpenAPI document with no route/schema drift", async () => {
  const scratch = makeScratch("openapi");
  const server = await createCortexServer({
    bin: cortexBin(),
    cwd: scratch.dir,
    env: { CORTEX_DATA_DIR: scratch.dir, CORTEX_NO_NET: "1" },
    timeoutMs: 20000,
  });
  const client = createCortexClient({ baseUrl: server.url });

  try {
    const live = await client.openapi();
    const committed = JSON.parse(
      readFileSync(join(SDK_ROOT, "openapi.json"), "utf8"),
    ) as Record<string, unknown>;

    // 1. The committed contract snapshot equals the live document (schema diff is empty).
    assert.deepEqual(live, committed, "committed openapi.json drifted from the served document");

    // 2. Generated schema artifacts cover every served schema and its required fields.
    const components = live.components as { schemas: Record<string, LiveSchema> };
    for (const [name, schema] of Object.entries(components.schemas)) {
      assert.ok(OPENAPI_SCHEMA_NAMES.includes(name), `generated types missing schema ${name}`);
      assert.deepEqual(
        REQUIRED_FIELDS[name] ?? [],
        schema.required ?? [],
        `required-field drift for schema ${name}`,
      );
    }

    // 3. Every served operation appears in the generated OPERATIONS table with matching method/path.
    const paths = live.paths as Record<string, Record<string, LiveOperation>>;
    let operationCount = 0;
    for (const [path, item] of Object.entries(paths)) {
      for (const [method, op] of Object.entries(item)) {
        if (!op || typeof op !== "object" || !op.operationId) continue;
        operationCount += 1;
        const meta = OPERATIONS[op.operationId as OperationId];
        assert.ok(meta, `generated OPERATIONS missing ${op.operationId}`);
        assert.equal(meta.method, method, `method drift for ${op.operationId}`);
        assert.equal(meta.path, path, `path drift for ${op.operationId}`);
      }
    }
    assert.ok(operationCount >= 80, `expected a broad route set, saw ${operationCount}`);

    // 4. Event families match.
    for (const kind of live["x-event-families"] as string[]) {
      assert.ok(EVENT_FAMILIES.includes(kind), `generated event families missing ${kind}`);
    }

    // 5. Sample requests generated from every parameterless GET operation run (no 404 drift).
    //    `streamEvents` is an SSE stream (would block) and `lspStatus` is a heavy, variable-latency
    //    real language-server probe; both are still covered by the drift checks above, so they are
    //    excluded from this request-shape smoke loop. Any other HTTP status (including environmental
    //    5xx) proves the generated request reached the route; only a 404 indicates route drift.
    const skipLiveFire = new Set<OperationId>(["streamEvents", "lspStatus"]);
    let exercised = 0;
    for (const [operationId, meta] of Object.entries(OPERATIONS)) {
      if (meta.method !== "get" || meta.pathParams.length > 0) continue;
      if (skipLiveFire.has(operationId as OperationId)) continue;
      exercised += 1;
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 15000);
      try {
        await client.request(operationId as OperationId, { signal: controller.signal });
      } catch (error) {
        if (error instanceof CortexApiError) {
          assert.notEqual(error.status, 404, `operation ${operationId} drifted (404)`);
        } else {
          throw new Error(`operation ${operationId} failed without an HTTP response: ${String(error)}`);
        }
      } finally {
        clearTimeout(timer);
      }
    }
    assert.ok(exercised >= 10, `expected to exercise several GET operations, ran ${exercised}`);
  } finally {
    await server.close();
    scratch.cleanup();
  }
});
