// Typed HTTP client for the Cortex Code local server.
//
// Every request is routed through the generated OPERATIONS table so paths, methods, and path
// parameters always match the served OpenAPI document. Grouped helpers (session/config/provider/
// auth) provide typed request/response shapes; `request()` exposes any operation generically.

import { OPERATIONS } from "./gen/operations.ts";
import type { OperationId } from "./gen/types.ts";
import type {
  AbortResponse,
  AuthListResponse,
  AuthStateResponse,
  AuthorizeRequest,
  AuthorizeResponse,
  ConfigPatchRequest,
  CreateSessionRequest,
  HealthResponse,
  Message,
  MessageListResponse,
  MessageRequest,
  PromptResponse,
  ProviderStateList,
  ProvidersCatalog,
  Session,
  SessionStatus,
  SetAuthRequest,
  UpdateSessionRequest,
} from "./gen/types.ts";
import { CortexEventStream, type CortexEventStreamOptions } from "./events.ts";

export interface CortexClientConfig {
  /** Base URL of a running `cortex serve`, e.g. `http://127.0.0.1:3247`. */
  baseUrl: string;
  /** Bearer token when the server was started with `--token`. */
  token?: string;
  /** Override the fetch implementation (defaults to the global `fetch`). */
  fetch?: typeof fetch;
  /** Extra headers to attach to every request. */
  headers?: Record<string, string>;
}

/** A normalized API error carrying the HTTP status and optional server error kind. */
export class CortexApiError extends Error {
  readonly status: number;
  readonly kind: string | undefined;
  readonly operationId: OperationId | undefined;

  constructor(message: string, status: number, kind?: string, operationId?: OperationId) {
    super(message);
    this.name = "CortexApiError";
    this.status = status;
    this.kind = kind;
    this.operationId = operationId;
  }
}

export interface RequestInput {
  readonly pathParams?: Readonly<Record<string, string>>;
  readonly query?: Readonly<Record<string, string | number | boolean | undefined>>;
  readonly body?: unknown;
  readonly signal?: AbortSignal;
}

interface ErrorBody {
  error?: string;
  kind?: string;
}

export class CortexClient {
  readonly baseUrl: string;
  readonly #token: string | undefined;
  readonly #fetch: typeof fetch;
  readonly #headers: Record<string, string>;

  constructor(config: CortexClientConfig) {
    this.baseUrl = config.baseUrl.replace(/\/+$/, "");
    this.#token = config.token;
    this.#fetch = config.fetch ?? fetch;
    this.#headers = { ...config.headers };
  }

  /** Issue a typed request for any documented operation. */
  async request<T>(operationId: OperationId, input: RequestInput = {}): Promise<T> {
    const meta = OPERATIONS[operationId];
    let path = meta.path;
    for (const name of meta.pathParams) {
      const value = input.pathParams?.[name];
      if (value === undefined) {
        throw new Error(`missing path parameter "${name}" for ${operationId}`);
      }
      path = path.replace(`{${name}}`, encodeURIComponent(value));
    }
    const url = new URL(this.baseUrl + path);
    if (input.query) {
      for (const [key, value] of Object.entries(input.query)) {
        if (value !== undefined) url.searchParams.set(key, String(value));
      }
    }
    const headers: Record<string, string> = { accept: "application/json", ...this.#headers };
    if (this.#token) headers.authorization = `Bearer ${this.#token}`;
    const hasBody = input.body !== undefined;
    if (hasBody) headers["content-type"] = "application/json";

    const response = await this.#fetch(url, {
      method: meta.method.toUpperCase(),
      headers,
      body: hasBody ? JSON.stringify(input.body) : undefined,
      signal: input.signal,
    });

    if (!response.ok) {
      let message = `${operationId} failed with HTTP ${response.status}`;
      let kind: string | undefined;
      const text = await response.text();
      if (text) {
        try {
          const parsed = JSON.parse(text) as ErrorBody;
          if (parsed.error) message = parsed.error;
          if (parsed.kind) kind = parsed.kind;
        } catch {
          message = text;
        }
      }
      throw new CortexApiError(message, response.status, kind, operationId);
    }

    if (response.status === 204) return undefined as T;
    const text = await response.text();
    if (!text) return undefined as T;
    return JSON.parse(text) as T;
  }

  /** Subscribe to the typed SSE event stream. */
  events(options: CortexEventStreamOptions = {}): CortexEventStream {
    const headers: Record<string, string> = {
      ...this.#headers,
      ...options.headers,
    };
    if (this.#token) headers.authorization = `Bearer ${this.#token}`;
    return new CortexEventStream(this.baseUrl, {
      ...options,
      fetch: options.fetch ?? this.#fetch,
      headers,
    });
  }

  health(): Promise<HealthResponse> {
    return this.request<HealthResponse>("getHealth");
  }

  openapi(): Promise<Record<string, unknown>> {
    return this.request<Record<string, unknown>>("getOpenApiDocument");
  }

  readonly session = {
    create: (body: CreateSessionRequest = {}): Promise<Session> =>
      this.request<Session>("createSession", { body }),
    list: (): Promise<Session[]> => this.request<Session[]>("listSessions"),
    get: (id: string): Promise<Session> =>
      this.request<Session>("getSession", { pathParams: { id } }),
    update: (id: string, body: UpdateSessionRequest): Promise<Session> =>
      this.request<Session>("updateSession", { pathParams: { id }, body }),
    delete: (id: string): Promise<void> =>
      this.request<void>("deleteSession", { pathParams: { id } }),
    prompt: (id: string, body: MessageRequest, signal?: AbortSignal): Promise<PromptResponse> =>
      this.request<PromptResponse>("postSessionMessage", { pathParams: { id }, body, signal }),
    messages: (id: string): Promise<MessageListResponse> =>
      this.request<MessageListResponse>("listSessionMessages", { pathParams: { id } }),
    message: (id: string, messageId: string): Promise<Message> =>
      this.request<Message>("getSessionMessage", {
        pathParams: { id, message_id: messageId },
      }),
    status: (id: string): Promise<SessionStatus> =>
      this.request<SessionStatus>("getSessionStatus", { pathParams: { id } }),
    abort: (id: string): Promise<AbortResponse> =>
      this.request<AbortResponse>("abortSessionTurn", { pathParams: { id } }),
  };

  readonly config = {
    get: (): Promise<Record<string, unknown>> =>
      this.request<Record<string, unknown>>("getConfig"),
    patch: (body: ConfigPatchRequest): Promise<Record<string, unknown>> =>
      this.request<Record<string, unknown>>("patchConfig", { body }),
    providers: (): Promise<ProvidersCatalog> =>
      this.request<ProvidersCatalog>("listConfigProviders"),
  };

  readonly provider = {
    list: (): Promise<ProviderStateList> => this.request<ProviderStateList>("listProviders"),
  };

  readonly auth = {
    list: (): Promise<AuthListResponse> => this.request<AuthListResponse>("listAuth"),
    set: (provider: string, body: SetAuthRequest): Promise<AuthStateResponse> =>
      this.request<AuthStateResponse>("setAuth", { pathParams: { provider }, body }),
    remove: (provider: string): Promise<AuthStateResponse> =>
      this.request<AuthStateResponse>("removeAuth", { pathParams: { provider } }),
    authorize: (provider: string, body: AuthorizeRequest): Promise<AuthorizeResponse> =>
      this.request<AuthorizeResponse>("oauthAuthorize", { pathParams: { provider }, body }),
    callback: (query: {
      code?: string;
      state?: string;
      error?: string;
    }): Promise<AuthStateResponse> => this.request<AuthStateResponse>("oauthCallback", { query }),
  };
}

export function createCortexClient(config: CortexClientConfig): CortexClient {
  return new CortexClient(config);
}
