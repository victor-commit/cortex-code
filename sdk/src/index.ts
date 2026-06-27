// Cortex Code SDK entry point.
//
// Re-exports the typed client, the typed SSE event client, the process launcher, the generated
// schema types, and the operation table. `createCortex` is the one-call helper that launches a
// local server and returns a connected client (mirroring the reference SDK ergonomics).

export * from "./gen/types.ts";
export * from "./client.ts";
export * from "./server.ts";
export * from "./events.ts";
export {
  assertPortInRange,
  isAlive,
  isPortFree,
  listenerPid,
  pickPort,
  stopServerProcess,
  PORT_RANGE_MIN,
  PORT_RANGE_MAX,
} from "./process.ts";
export { OPERATIONS, type OperationMeta } from "./gen/operations.ts";
export {
  EVENT_FAMILIES,
  OPENAPI_SCHEMA_NAMES,
  REQUIRED_FIELDS,
} from "./gen/schemas.ts";

import { createCortexClient, type CortexClient } from "./client.ts";
import { createCortexServer, type CortexServer, type CortexServerOptions } from "./server.ts";

export interface Cortex {
  server: CortexServer;
  client: CortexClient;
}

/** Launch a local `cortex serve` and return a connected, typed client. */
export async function createCortex(options: CortexServerOptions = {}): Promise<Cortex> {
  const server = await createCortexServer(options);
  const client = createCortexClient({ baseUrl: server.url, token: options.token });
  return { server, client };
}
