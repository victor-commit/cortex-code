// VAL-GAP-SDK-002: the SDK exposes a typed event subscription. During a real OpenRouter turn the
// stream yields typed text and final (turn.done/turn.usage) variants; a typed tool.call variant is
// exercised over the same live SSE connection via the documented publishSurfaceEvent route.

import { test } from "node:test";
import assert from "node:assert/strict";

import { createCortexServer } from "../src/server.ts";
import { createCortexClient } from "../src/client.ts";
import type { CortexEvent } from "../src/events.ts";
import { cortexBin, delay, hasOpenRouterKey, makeScratch } from "./helpers.ts";

test(
  "SDK delivers typed text, tool, and final event variants for a live turn",
  { skip: hasOpenRouterKey() ? false : "OPENROUTER_API_KEY not set" },
  async () => {
    const scratch = makeScratch("events");
    const server = await createCortexServer({
      bin: cortexBin(),
      cwd: scratch.dir,
      env: { CORTEX_DATA_DIR: scratch.dir },
      timeoutMs: 20000,
    });
    const client = createCortexClient({ baseUrl: server.url });

    try {
      const session = await client.session.create({});

      // Collect events until this session's turn completes.
      const stream = client.events();
      const collected: Promise<CortexEvent[]> = stream.collectUntil(
        (event) => event.kind === "turn.done" && event.session_id === session.id,
        { timeoutMs: 90000 },
      );

      // Ensure the SSE subscription is established before injecting/prompting.
      await delay(700);

      // A real, documented server route that republishes a typed event onto the stream.
      await client.request("publishSurfaceEvent", {
        body: {
          kind: "tool.call",
          session_id: session.id,
          message_id: "turn-probe",
          id: "call_probe",
          name: "demo_tool",
          arguments: '{"q":1}',
        },
      });

      // A real OpenRouter turn drives the text deltas and final events.
      const prompt = await client.session.prompt(session.id, {
        text: "Say hello in one short sentence.",
      });
      assert.equal(prompt.status, "completed");

      const events = await collected;

      // Typed TEXT event.
      const textEvent = events.find((event) => event.kind === "assistant.text_delta");
      assert.ok(textEvent, "expected an assistant.text_delta event");
      if (textEvent.kind === "assistant.text_delta") {
        assert.equal(typeof textEvent.text, "string");
        assert.ok(textEvent.text.length > 0, "text delta carries content");
        assert.equal(textEvent.session_id, session.id);
      }

      // Typed TOOL event.
      const toolEvent = events.find((event) => event.kind === "tool.call");
      assert.ok(toolEvent, "expected a tool.call event");
      if (toolEvent.kind === "tool.call") {
        assert.equal(toolEvent.name, "demo_tool");
        assert.equal(typeof toolEvent.arguments, "string");
      }

      // Typed FINAL events.
      const done = events.find((event) => event.kind === "turn.done");
      assert.ok(done, "expected a turn.done event");
      if (done.kind === "turn.done") {
        assert.equal(typeof done.reason, "string");
        assert.equal(typeof done.steps, "number");
      }
      const usage = events.find((event) => event.kind === "turn.usage");
      assert.ok(usage, "expected a turn.usage event");
      if (usage.kind === "turn.usage") {
        assert.equal(typeof usage.total_tokens, "number");
        assert.ok(usage.total_tokens > 0, "usage reports tokens");
      }
    } finally {
      await server.close();
      scratch.cleanup();
    }
  },
);
