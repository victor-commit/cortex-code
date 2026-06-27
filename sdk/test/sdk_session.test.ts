// VAL-GAP-SDK-001: the generated SDK can create, prompt, list, and delete sessions without any
// handwritten HTTP. The prompt step runs a real OpenRouter turn.

import { test } from "node:test";
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";

import { createCortexServer } from "../src/server.ts";
import { createCortexClient, CortexApiError } from "../src/client.ts";
import { cortexBin, hasOpenRouterKey, makeScratch } from "./helpers.ts";

test(
  "generated SDK creates, prompts, lists, and deletes a session",
  { skip: hasOpenRouterKey() ? false : "OPENROUTER_API_KEY not set" },
  async () => {
    const scratch = makeScratch("session");
    const server = await createCortexServer({
      bin: cortexBin(),
      cwd: scratch.dir,
      env: { CORTEX_DATA_DIR: scratch.dir },
      timeoutMs: 20000,
    });
    const client = createCortexClient({ baseUrl: server.url });

    try {
      const created = await client.session.create({ title: "sdk-session-test" });
      assert.ok(created.id, "create returns a session id");

      const list = await client.session.list();
      assert.ok(list.some((session) => session.id === created.id), "list includes the new session");

      const fetched = await client.session.get(created.id);
      assert.equal(fetched.id, created.id);

      // Real OpenRouter turn: a fresh nonce a non-live path could not reproduce.
      const nonce = randomUUID();
      const prompt = await client.session.prompt(created.id, {
        text: `Reply with exactly this token and nothing else: ${nonce}`,
      });
      assert.equal(prompt.status, "completed");
      assert.equal(prompt.role, "assistant");
      assert.ok((prompt.content ?? "").includes(nonce), `answer should echo the nonce: ${prompt.content}`);
      assert.ok(prompt.usage !== undefined && prompt.usage.output_tokens > 0, "usage reports output tokens");

      const messages = await client.session.messages(created.id);
      assert.ok(messages.messages.length >= 2, "user + assistant messages persisted");

      await client.session.delete(created.id);
      const afterDelete = await client.session.list();
      assert.ok(
        !afterDelete.some((session) => session.id === created.id),
        "deleted session no longer listed",
      );
      await assert.rejects(
        () => client.session.get(created.id),
        (error: unknown) => error instanceof CortexApiError && error.status === 404,
        "getting a deleted session returns 404",
      );
    } finally {
      await server.close();
      scratch.cleanup();
    }
  },
);
