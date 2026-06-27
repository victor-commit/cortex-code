// VAL-GAP-SDK-003: SDK auth/provider/config methods match the server spec. Network-safe: only the
// local loopback server is contacted (CORTEX_NO_NET=1) and credential/config writes hit a scratch
// filesystem layer.

import { test } from "node:test";
import assert from "node:assert/strict";

import { createCortexServer } from "../src/server.ts";
import { createCortexClient, CortexApiError } from "../src/client.ts";
import { cortexBin, makeScratch, syntheticApiKey } from "./helpers.ts";

const PROVIDER = "sdktestprovider";

test("SDK config/provider/auth methods exist, typecheck, and change server state", async () => {
  const scratch = makeScratch("authcfg");
  const server = await createCortexServer({
    bin: cortexBin(),
    cwd: scratch.dir,
    env: { CORTEX_DATA_DIR: scratch.dir, CORTEX_NO_NET: "1", OPENROUTER_API_KEY: "" },
    timeoutMs: 20000,
  });
  const client = createCortexClient({ baseUrl: server.url });

  try {
    // config get -> redacted effective config table.
    const config = await client.config.get();
    assert.equal(typeof config, "object");

    // config providers -> catalog mirroring `cortex models`.
    const providers = await client.config.providers();
    assert.equal(typeof providers.default_model, "string");
    assert.ok(Array.isArray(providers.providers));

    // provider list -> providers with resolved authenticated state.
    const providerState = await client.provider.list();
    assert.ok(Array.isArray(providerState.providers));

    // config patch -> persisted and observable on a later get.
    const patched = await client.config.patch({ model: "google/gemini-2.5-flash" });
    assert.equal((patched as { model?: string }).model, "google/gemini-2.5-flash");
    const after = await client.config.get();
    assert.equal((after as { model?: string }).model, "google/gemini-2.5-flash");

    // auth list -> typed methods + authenticated providers.
    const auth = await client.auth.list();
    assert.ok(Array.isArray(auth.authenticated));
    assert.ok(Array.isArray(auth.methods));

    // auth set stores a credential -> provider becomes authenticated.
    const set = await client.auth.set(PROVIDER, { type: "api", key: syntheticApiKey() });
    assert.equal(set.provider, PROVIDER);
    assert.equal(set.authenticated, true);
    const afterSet = await client.auth.list();
    assert.ok(afterSet.authenticated.includes(PROVIDER), "provider authenticated after set");

    // auth remove clears it -> provider no longer authenticated.
    const removed = await client.auth.remove(PROVIDER);
    assert.equal(removed.provider, PROVIDER);
    assert.equal(removed.authenticated, false);
    const afterRemove = await client.auth.list();
    assert.ok(!afterRemove.authenticated.includes(PROVIDER), "provider not authenticated after remove");

    // auth authorize builds an authorize URL and a CSRF state challenge.
    const authorize = await client.auth.authorize("customoauth", {
      authorize_endpoint: "https://example.test/authorize",
      token_endpoint: "https://example.test/token",
      client_id: "client-123",
    });
    assert.ok(
      authorize.authorize_url.startsWith("https://example.test/authorize"),
      `authorize_url should start at the configured endpoint: ${authorize.authorize_url}`,
    );
    assert.ok(authorize.state.length > 0, "authorize returns a CSRF state");
    assert.equal(authorize.provider, "customoauth");

    // auth callback exists and validates the CSRF state (an unknown state is rejected, not 404).
    await assert.rejects(
      () => client.auth.callback({ code: "abc", state: "never-issued" }),
      (error: unknown) => error instanceof CortexApiError && error.status === 400,
      "callback rejects an unissued state with 400",
    );
  } finally {
    await server.close();
    scratch.cleanup();
  }
});
