// Typed Server-Sent Events client for the Cortex Code local server `/event` stream.
//
// The server publishes one event per SSE frame as `event: <kind>` + `data: <json>` where the JSON
// payload carries a `kind` discriminator plus flat fields. This module decodes each frame into a
// discriminated `CortexEvent` union so consumers narrow on `kind` without `any` casts.
//
// Streaming/turn correlation note: granular text/reasoning deltas
// (`assistant.text_delta` / `session.next.text.delta`) carry the engine turn id, which is the
// user/prompt `message_id` that identifies the turn, not the persisted assistant message id. The
// persisted assistant message id appears on the post-turn `message.parts` / `message.updated` /
// `message.part.updated` frames. Correlate streaming text to a turn by `session_id`, and read the
// persisted assistant id from those message frames. See library/server.md.

import type { CortexEventKind, RichMessagePart } from "./gen/types.ts";

/** A bounded tool-result projection safe to present to a model or a UI. */
export type ToolResultPart =
  | { type: "text"; text: string }
  | { type: "image" | "document"; media_type: string; path: string };

/** File reference for a typed payload too large to relay inline. */
export interface McpSpillReference {
  path: string;
  sha256: string;
  byte_length: number;
  encoding: string;
}

/** Complete MCP source result emitted with `tool.result` for MCP-routed calls. */
export interface McpToolResult {
  blocks: Array<{ type: string; [key: string]: unknown }>;
  structured_content?:
    | { kind: "inline"; value: unknown }
    | { kind: "spill"; value: { spill: McpSpillReference } };
  metadata?: unknown;
  is_error?: boolean;
}

/** Strongly typed payloads for the turn-relevant event families. */
export interface KnownEventPayloads {
  "turn.started": { session_id: string; message_id: string };
  "assistant.text_delta": { session_id: string; message_id: string; text: string };
  "assistant.reasoning_delta": { session_id: string; message_id: string; text: string };
  "session.next.text.start": { session_id: string; message_id: string };
  "session.next.text.delta": { session_id: string; message_id: string; text: string };
  "session.next.text.end": { session_id: string; message_id: string };
  "session.next.reasoning.delta": { session_id: string; message_id: string; text: string };
  "tool.call": {
    session_id?: string;
    message_id?: string;
    id: string;
    name: string;
    arguments: string;
  };
  "tool.result": {
    session_id?: string;
    message_id?: string;
    id: string;
    name: string;
    /** Bounded model-facing projection, independent from raw MCP source blocks. */
    parts: ToolResultPart[];
    /** Complete source result for MCP-routed tool calls. */
    mcp_result?: McpToolResult;
    is_error: boolean;
  };
  "turn.usage": {
    session_id: string;
    message_id: string;
    input_tokens: number;
    output_tokens: number;
    total_tokens: number;
    cost?: number;
  };
  "turn.done": { session_id: string; message_id: string; reason: string; steps: number };
  "turn.error": { session_id: string; message_id: string; error_kind: string; message: string };
  "message.parts": { session_id: string; message_id: string; parts: RichMessagePart[] };
  "message.updated": { session_id: string; message_id: string; role: string; time_created?: string };
  "message.part.updated": {
    session_id: string;
    message_id: string;
    part_id: string;
    part: RichMessagePart;
  };
  "session.status": { session_id: string; status: string };
  "session.idle": { session_id: string };
}

/** A typed known event: the literal `kind` plus its payload fields. */
export type KnownEvent = {
  [K in keyof KnownEventPayloads]: { kind: K } & KnownEventPayloads[K];
}[keyof KnownEventPayloads];

/** Event kinds that do not (yet) have a hand-typed payload above. */
export type OtherEventKind = Exclude<CortexEventKind, keyof KnownEventPayloads>;

/** A long-tail event: a valid kind with best-effort flat fields. */
export interface GenericEvent {
  kind: OtherEventKind;
  session_id?: string;
  message_id?: string;
  [field: string]: unknown;
}

/** Every decoded SSE event. Narrow on `kind` to access typed payload fields. */
export type CortexEvent = KnownEvent | GenericEvent;

// Compile-time guard: every hand-typed key must be a real server event kind.
type AssertKnownAreKinds = Exclude<keyof KnownEventPayloads, CortexEventKind> extends never
  ? true
  : never;
const _knownAreKinds: AssertKnownAreKinds = true;
void _knownAreKinds;

export interface CortexEventStreamOptions {
  signal?: AbortSignal;
  fetch?: typeof fetch;
  headers?: Record<string, string>;
}

export interface CollectUntilOptions {
  timeoutMs?: number;
  signal?: AbortSignal;
}

function parseFrame(frame: string): CortexEvent | undefined {
  const dataLines: string[] = [];
  for (const rawLine of frame.split("\n")) {
    const line = rawLine.endsWith("\r") ? rawLine.slice(0, -1) : rawLine;
    if (line.startsWith("data:")) {
      dataLines.push(line.slice(5).replace(/^ /, ""));
    }
  }
  if (dataLines.length === 0) return undefined;
  try {
    return JSON.parse(dataLines.join("\n")) as CortexEvent;
  } catch {
    return undefined;
  }
}

/** An async-iterable, typed view of the server `/event` SSE stream. */
export class CortexEventStream implements AsyncIterable<CortexEvent> {
  readonly #baseUrl: string;
  readonly #options: CortexEventStreamOptions;

  constructor(baseUrl: string, options: CortexEventStreamOptions = {}) {
    this.#baseUrl = baseUrl.replace(/\/+$/, "");
    this.#options = options;
  }

  async *[Symbol.asyncIterator](): AsyncGenerator<CortexEvent> {
    const doFetch = this.#options.fetch ?? fetch;
    const response = await doFetch(`${this.#baseUrl}/event`, {
      headers: { accept: "text/event-stream", ...this.#options.headers },
      signal: this.#options.signal,
    });
    if (!response.ok || !response.body) {
      throw new Error(`event stream failed with HTTP ${response.status}`);
    }
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        let index = buffer.indexOf("\n\n");
        while (index >= 0) {
          const frame = buffer.slice(0, index);
          buffer = buffer.slice(index + 2);
          const event = parseFrame(frame);
          if (event) yield event;
          index = buffer.indexOf("\n\n");
        }
      }
    } finally {
      await reader.cancel().catch(() => {});
    }
  }

  /**
   * Collect events until `predicate` returns true (inclusive) or the timeout elapses. Returns every
   * event observed. Throws on timeout so callers get a clear failure instead of hanging.
   */
  async collectUntil(
    predicate: (event: CortexEvent) => boolean,
    options: CollectUntilOptions = {},
  ): Promise<CortexEvent[]> {
    const controller = new AbortController();
    const timeoutMs = options.timeoutMs ?? 30000;
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    if (options.signal) {
      if (options.signal.aborted) controller.abort();
      else options.signal.addEventListener("abort", () => controller.abort(), { once: true });
    }
    const stream = new CortexEventStream(this.#baseUrl, {
      ...this.#options,
      signal: controller.signal,
    });
    const collected: CortexEvent[] = [];
    try {
      for await (const event of stream) {
        collected.push(event);
        if (predicate(event)) return collected;
      }
      throw new Error("event stream ended before predicate matched");
    } catch (error) {
      if (controller.signal.aborted) {
        throw new Error(`timed out after ${timeoutMs}ms waiting for event; saw ${collected.length}`);
      }
      throw error;
    } finally {
      clearTimeout(timer);
    }
  }
}
