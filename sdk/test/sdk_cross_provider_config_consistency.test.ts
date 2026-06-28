// VAL-GAP-CROSS-011 (SDK half): applying a provider-configuration change is reflected consistently
// through the typed SDK provider surfaces. The SDK consumes the same `/config/providers` and
// `/provider` routes asserted in the Rust cross-flow test
// (`crates/cli/tests/gap_cross_provider_config_picker_consistency.rs`), so this proves the SDK
// provider list agrees with the CLI/server availability metadata after the change.
//
// The applied config ADDS a `local` provider and DISABLES the built-in `openrouter` provider (which
// serves every embedded model). After the change both `client.config.providers()` and
// `client.provider.list()` advertise `local`, omit `openrouter`, and report the configured model as
// the default. Network-safe: only the local loopback server is contacted (CORTEX_NO_NET=1) and the
// config/credentials live in a scratch filesystem layer.

import { test } from "node:test";
import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { createCortexServer } from "../src/server.ts";
import { createCortexClient } from "../src/client.ts";
import { cortexBin, makeScratch } from "./helpers.ts";

const MODEL_ID = "local/test-model";

function providerIds(providers: { id?: unknown }[]): string[] {
  return providers
    .map((provider) => (typeof provider.id === "string" ? provider.id : undefined))
    .filter((id): id is string => typeof id === "string")
    .sort();
}

test("SDK provider list reflects an applied provider-config change consistently", async () => {
  const scratch = makeScratch("cross-provider");
  // Apply the provider change in the server's project config (its cwd). `api_key` is a synthetic
  // config-layer value, never a real secret.
  writeFileSync(
    join(scratch.dir, "cortex.toml"),
    [
      `model = "${MODEL_ID}"`,
      "",
      "[provider]",
      'disabled = ["openrouter"]',
      "",
      "[provider.local]",
      'base_url = "http://127.0.0.1:3299"',
      'api_key = "cfg-cross-provider-value"',
      `models = [{ id = "${MODEL_ID}", name = "Local Test Model", tool_call = true, context_length = 8192, max_output_tokens = 1024 }]`,
      "",
    ].join("\n"),
  );

  const server = await createCortexServer({
    bin: cortexBin(),
    cwd: scratch.dir,
    env: { CORTEX_DATA_DIR: scratch.dir, CORTEX_NO_NET: "1", OPENROUTER_API_KEY: "" },
    timeoutMs: 20000,
  });
  const client = createCortexClient({ baseUrl: server.url });

  try {
    // config providers -> catalog mirroring `cortex models`.
    const catalog = await client.config.providers();
    assert.equal(catalog.default_model, MODEL_ID, "SDK catalog default model matches the config");
    const catalogIds = providerIds(catalog.providers);
    assert.ok(catalogIds.includes("local"), `added provider present in catalog: ${catalogIds}`);
    assert.ok(
      !catalogIds.includes("openrouter"),
      `disabled provider absent from catalog: ${catalogIds}`,
    );

    // provider list -> providers with resolved authenticated state.
    const state = await client.provider.list();
    assert.equal(state.default_model, MODEL_ID, "SDK provider list default model matches the config");
    const stateIds = providerIds(state.providers);
    assert.ok(stateIds.includes("local"), `added provider present in provider list: ${stateIds}`);
    assert.ok(
      !stateIds.includes("openrouter"),
      `disabled provider absent from provider list: ${stateIds}`,
    );

    // Both SDK provider surfaces agree with each other (and therefore with CLI/server).
    assert.deepEqual(catalogIds, stateIds, "SDK catalog and provider-list ids agree");

    // The synthetic api key must never leak through the typed surface.
    assert.ok(
      !JSON.stringify(catalog).includes("cfg-cross-provider-value"),
      "the configured api key must be redacted in the SDK catalog",
    );
  } finally {
    await server.close();
    scratch.cleanup();
  }
});
