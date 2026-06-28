// VAL-GAP-CROSS-009: a typed SDK event client reconstructs a live rich-part turn exactly from
// server SSE and the reconstruction equals the persisted session export.
//
// The flow drives one real OpenRouter turn that produces text + a tool call + a patch part (a real
// `write` mutating the scratch filesystem), reconstructs the assistant message purely from the SSE
// frames (`message.part.updated` rebuild the `message.parts` snapshot; `assistant.text_delta`
// rebuild the final text; `tool.call` proves the tool path), then proves the reconstruction equals
// BOTH persisted session exports — the typed message list and the wire-compatible share/export
// artifact — for part ids, ordering, payload types, and final message content. Persistence assigns
// part timestamps, so part bodies are compared with `created_at` stripped (every other field must
// match exactly), matching the contract's "part IDs, ordering, payload types, and final text".

import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { createCortexServer } from "../src/server.ts";
import { createCortexClient } from "../src/client.ts";
import type { CortexEvent } from "../src/events.ts";
import type { Message, RichMessagePart } from "../src/gen/types.ts";
import { cortexBin, delay, hasOpenRouterKey, makeScratch } from "./helpers.ts";

/** The importable share artifact (wire-compatible with `cortex export`). */
interface ShareArtifact {
  version: number;
  session: { id: string; title?: string | null; model?: string | null };
  messages: { role: string; content: string; parts?: RichMessagePart[] }[];
}

/** Compare part bodies ignoring the persistence-assigned timestamp. */
function withoutTimestamp(part: RichMessagePart): Record<string, unknown> {
  const { created_at, ...rest } = part;
  void created_at;
  return rest;
}

test(
  "SDK event client reconstructs a live rich-part turn and it equals the persisted session export",
  { skip: hasOpenRouterKey() ? false : "OPENROUTER_API_KEY not set" },
  async () => {
    const scratch = makeScratch("cross-rich-part");
    const server = await createCortexServer({
      bin: cortexBin(),
      cwd: scratch.dir,
      env: { CORTEX_DATA_DIR: scratch.dir },
      timeoutMs: 20000,
    });
    const client = createCortexClient({ baseUrl: server.url });

    try {
      const session = await client.session.create({ title: "cross-rich-part" });

      // Subscribe BEFORE prompting and collect the whole turn up to its terminal step frame, which
      // the server emits AFTER the persisted message.parts / message.part.updated frames.
      const stream = client.events();
      const collected = stream.collectUntil(
        (event) =>
          event.kind === "session.next.step.finished" && event.session_id === session.id,
        { timeoutMs: 120000 },
      );
      await delay(800);

      const prompt = await client.session.prompt(session.id, {
        text:
          "Use the write tool to create a file named add.py containing a Python function " +
          "add(a, b) that returns a + b. After writing the file, reply with one short sentence " +
          "confirming you created add.py.",
      });
      assert.equal(prompt.status, "completed");

      const events = await collected;

      // --- Reconstruct the assistant turn purely from SSE -------------------------------------
      const snapshotFrame = events.find(
        (event): event is Extract<CortexEvent, { kind: "message.parts" }> =>
          event.kind === "message.parts" && event.session_id === session.id,
      );
      assert.ok(snapshotFrame, "saw a message.parts frame for the assistant turn");
      const assistantId = snapshotFrame.message_id;
      const snapshotParts = snapshotFrame.parts;

      const reconstructed: RichMessagePart[] = [];
      for (const event of events) {
        if (event.kind === "message.part.updated" && event.message_id === assistantId) {
          reconstructed.push(event.part);
        }
      }

      let reconstructedText = "";
      for (const event of events) {
        if (event.kind === "assistant.text_delta" && event.session_id === session.id) {
          reconstructedText += event.text;
        }
      }

      const toolCalls = events.filter(
        (event) => event.kind === "tool.call" && event.session_id === session.id,
      );

      // SSE internal consistency: the per-part frames rebuild the message.parts snapshot exactly.
      assert.deepEqual(
        reconstructed.map(withoutTimestamp),
        snapshotParts.map(withoutTimestamp),
        "message.part.updated frames reconstruct the message.parts snapshot",
      );

      // It is genuinely a rich-part turn: text + tool + patch (file mutation) parts.
      const types = reconstructed.map((part) => part.type);
      assert.ok(types.includes("text"), `expected a text part: ${types.join(",")}`);
      assert.ok(types.includes("tool"), `expected a tool part: ${types.join(",")}`);
      assert.ok(
        types.includes("patch") || types.includes("file"),
        `expected a patch or file part: ${types.join(",")}`,
      );
      assert.ok(toolCalls.length > 0, "saw at least one tool.call frame on the stream");

      // --- Persisted session export #1: the typed message list -------------------------------
      const messageList = await client.session.messages(session.id);
      const assistant = messageList.messages.find(
        (message: Message) => message.id === assistantId,
      );
      assert.ok(assistant, "assistant message persisted in the session export");
      const exportParts = assistant.parts ?? [];

      // --- Persisted session export #2: the wire-compatible share/export artifact -------------
      const share = await client.request<{ share_id: string }>("shareSessionLocal", {
        pathParams: { id: session.id },
      });
      const artifact = await client.request<ShareArtifact>("getLocalShare", {
        pathParams: { share_id: share.share_id },
      });
      const artifactAssistant = artifact.messages.find((message) => message.role === "assistant");
      assert.ok(artifactAssistant, "assistant message present in the export artifact");
      const artifactParts = artifactAssistant.parts ?? [];

      // Part ids + ordering equal across the reconstruction and both exports.
      const reconstructedIds = reconstructed.map((part) => part.id);
      assert.deepEqual(
        reconstructedIds,
        exportParts.map((part) => part.id),
        "part ids/order equal the message-list export",
      );
      assert.deepEqual(
        reconstructedIds,
        artifactParts.map((part) => part.id),
        "part ids/order equal the share/export artifact",
      );

      // Payload types equal across the reconstruction and both exports.
      assert.deepEqual(
        types,
        exportParts.map((part) => part.type),
        "payload types equal the message-list export",
      );
      assert.deepEqual(
        types,
        artifactParts.map((part) => part.type),
        "payload types equal the share/export artifact",
      );

      // Exact rich-part reconstruction (timestamps are assigned by persistence).
      assert.deepEqual(
        reconstructed.map(withoutTimestamp),
        exportParts.map(withoutTimestamp),
        "rich parts reconstructed exactly vs the message-list export",
      );
      assert.deepEqual(
        reconstructed.map(withoutTimestamp),
        artifactParts.map(withoutTimestamp),
        "rich parts reconstructed exactly vs the share/export artifact",
      );

      // Final text agrees across the SSE reconstruction, the persisted text part, and both exports.
      const textPart = exportParts.find((part) => part.type === "text");
      assert.ok(textPart, "the export carries a persisted text part");
      const finalText = reconstructedText.trim();
      assert.ok(finalText.length > 0, "reconstructed final text is non-empty");
      assert.equal(finalText, String(textPart.text), "reconstructed text == persisted text part");
      assert.equal(
        finalText,
        (assistant.content ?? "").trim(),
        "reconstructed text == persisted assistant message content",
      );
      assert.equal(
        finalText,
        (prompt.content ?? "").trim(),
        "reconstructed text == synchronous prompt response content",
      );
      assert.equal(
        (artifactAssistant.content ?? "").trim(),
        finalText,
        "reconstructed text == share/export artifact content",
      );

      // No-mock proof: the patch part really mutated the scratch filesystem.
      const patchPart = reconstructed.find((part) => part.type === "patch");
      assert.ok(patchPart, "a patch part is present in the reconstruction");
      assert.ok(existsSync(join(scratch.dir, "add.py")), "add.py exists on disk after the turn");
      const source = readFileSync(join(scratch.dir, "add.py"), "utf8");
      assert.match(source, /def\s+add\s*\(/, "add.py defines an add() function");
    } finally {
      await server.close();
      scratch.cleanup();
    }
  },
);
