# Providers and subscription OAuth

Cortex reaches models through a **catalogue-driven** provider layer plus a smaller set of
**first-class profiles**. Most providers need only an API key and a models.dev catalogue
entry. A few support **subscription OAuth** (browser or device code) in addition to keys.

This page covers:

1. How catalogue fallback and first-class profiles work together
2. Built-in first-class profiles (ids, env keys, protocols)
3. ChatGPT Plus/Pro subscription OAuth (Codex backend)
4. xAI subscription OAuth (browser + device code)
5. The `CORTEX_OAUTH_*` override matrix
6. Why Claude Pro/Max OAuth is deliberately unsupported

Credentials land in a mode-`0600` `auth.json` (`AuthEntry { key: String }`). Never commit
tokens, refresh tokens, or API keys. Public OAuth **client ids** below are product app ids
(not secrets).

---

## Catalogue-driven providers

At runtime, base URL and credential env resolution follow:

```text
config override → first-class profile → models.dev catalogue → OpenRouter default → error
```

- **Catalogue source:** [models.dev](https://models.dev) (live refresh when network is
  allowed, offline disk cache, embedded snapshot). Zen is excluded.
- **`CORTEX_NO_NET=1`:** no outbound catalogue fetch; offline/embedded catalogue only.
- **Implication:** dozens of OpenAI-compatible providers work **without** a dedicated
  Cortex profile, as long as models.dev supplies `api` and `env` and you set the key.

List and refresh:

```bash
cortex models
cortex models --refresh
```

Connect or store a key:

```bash
cortex auth login <provider> --api-key "$KEY"
# or inside the TUI: /connect
```

---

## First-class provider profiles

First-class profiles pin protocol, optional default base URL, and primary API-key env.
They are **fallbacks**, not gates: when `default_base_url` is unset, the live catalogue
`api` wins (so URLs do not drift in source).

Local/keyless rows keep an explicit loopback base URL.

| Provider id | Display | API key env | Protocol | Notes |
| --- | --- | --- | --- | --- |
| `openrouter` | OpenRouter | `OPENROUTER_API_KEY` | openai-chat | Product default routing |
| `anthropic` | Anthropic | `ANTHROPIC_API_KEY` | anthropic-messages | Direct API key only (see [Claude Pro/Max](#claude-promax-oauth-unsupported)) |
| `openai` | OpenAI | `OPENAI_API_KEY` | openai-responses | Platform API key (not ChatGPT subscription) |
| `google` | Google | `GEMINI_API_KEY` | gemini | |
| `bedrock` | Amazon Bedrock | `AWS_BEARER_TOKEN_BEDROCK` | anthropic-messages | |
| `azure` | Azure OpenAI | `AZURE_OPENAI_API_KEY` | openai-chat | |
| `vertex` | Google Vertex AI | `GOOGLE_API_KEY` | gemini | |
| `copilot` | GitHub Copilot | `GITHUB_COPILOT_TOKEN` | openai-chat | OAuth supported |
| `chatgpt` | ChatGPT | *(OAuth)* | openai-responses | Subscription OAuth → Codex backend |
| `xai` | xAI | `XAI_API_KEY` | openai-chat | API key and/or xAI OAuth |
| `groq` | Groq | `GROQ_API_KEY` | openai-chat | |
| `mistral` | Mistral | `MISTRAL_API_KEY` | openai-chat | |
| `cohere` | Cohere | `COHERE_API_KEY` | openai-chat | |
| `deepinfra` | DeepInfra | `DEEPINFRA_API_KEY` | openai-chat | |
| `cerebras` | Cerebras | `CEREBRAS_API_KEY` | openai-chat | |
| `perplexity` | Perplexity | `PERPLEXITY_API_KEY` | openai-chat | |
| `together` | Together | `TOGETHER_API_KEY` | openai-chat | |
| `vercel` | Vercel | `VERCEL_API_KEY` | openai-chat | |
| `alibaba` | Alibaba | `ALIBABA_API_KEY` | openai-chat | |
| `cloudflare` | Cloudflare | `CLOUDFLARE_API_TOKEN` | openai-chat | |
| `openai-compatible` | OpenAI-Compatible | `OPENAI_API_KEY` | openai-chat | Generic base URL via config |
| `ollama` | Ollama | *(none)* | openai-chat | `http://127.0.0.1:11434/v1` |
| `lmstudio` | LM Studio | *(none)* | openai-chat | `http://127.0.0.1:1234/v1` |
| `llamacpp` | llama.cpp | *(none)* | openai-chat | `http://127.0.0.1:8080/v1` |
| `deepseek` | DeepSeek | `DEEPSEEK_API_KEY` | openai-chat | |
| `moonshotai` | Moonshot AI | `MOONSHOT_API_KEY` | openai-chat | |
| `zai` | Z.AI | `ZAI_API_KEY` | openai-chat | |
| `fireworks-ai` | Fireworks AI | `FIREWORKS_API_KEY` | openai-chat | |
| `huggingface` | Hugging Face | `HF_TOKEN` | openai-chat | |
| `nvidia` | NVIDIA | `NVIDIA_API_KEY` | openai-chat | |
| `baseten` | Baseten | `BASETEN_API_KEY` | openai-chat | |
| `minimax` | MiniMax | `MINIMAX_API_KEY` | openai-chat | |
| `nebius` | Nebius | `NEBIUS_API_KEY` | openai-chat | |
| `venice` | Venice | `VENICE_API_KEY` | openai-chat | |

Wave-2 “first-class” additions (direct protocol pins + local servers) include:
`anthropic`, `openai`, `google`, `deepseek`, `moonshotai`, `zai`, `fireworks-ai`,
`huggingface`, `nvidia`, `baseten`, `minimax`, `nebius`, `venice`, `lmstudio`, `llamacpp`.

OAuth-capable built-ins today: **`copilot`**, **`chatgpt`**, **`xai`**.

---

## OAuth engine (shared)

Subscription and Copilot OAuth share one engine:

| Capability | Behavior |
| --- | --- |
| Authorization code + PKCE | S256: `code_challenge` / `code_challenge_method=S256` on authorize; matching `code_verifier` on token exchange |
| Device code (RFC 8628) | Device authorization request, poll with `interval`, `slow_down` backoff, terminal `expired_token` / `access_denied` |
| Token refresh | Proactive refresh when expiry is known |
| 401 retry | **Single-shot** refresh-and-retry on HTTP 401 when `refresh_token`, `token_endpoint`, and `client_id` are present; prior refresh/id tokens preserved if the server omits replacements |
| Loopback callback | Reserved scan range **3200-3299**, plus product pins **1455** (ChatGPT) and **56121** (xAI). Pinned ports do not silently fall back to another port when occupied |
| Secrets | Access, refresh, and id tokens never printed to stdout/stderr or audit sinks |

CLI shapes:

```bash
cortex auth login chatgpt --method oauth
cortex auth login xai --method oauth
cortex auth login xai --method device
cortex auth list
```

TUI: `/connect` offers the same methods for OAuth-capable providers.

---

## ChatGPT Plus / Pro (Codex backend)

Use provider id **`chatgpt`** when you want a **ChatGPT subscription** path, not a
platform `OPENAI_API_KEY` against `api.openai.com`.

### Defaults

| Field | Value |
| --- | --- |
| Authorize | `https://auth.openai.com/oauth/authorize` |
| Token | `https://auth.openai.com/oauth/token` |
| Client id (public) | `app_EMoamEEZ73f0CkXaXp7hrann` |
| Scopes | `openid` `profile` `email` `offline_access` |
| Callback | `http://localhost:1455/auth/callback` (port **1455**, path `/auth/callback`) |
| Authorize extras | `id_token_add_organizations=true`, `codex_cli_simplified_flow=true` |
| API base | `https://chatgpt.com/backend-api/codex` (stream path appends `/responses`) |
| Protocol | openai-responses |

### Account routing header

After login, requests carry **`ChatGPT-Account-Id`**, derived from the stored OIDC
`id_token` JWT payload (signature not verified; used only for routing). Claim order:

1. top-level `chatgpt_account_id`
2. `https://api.openai.com/auth` → `chatgpt_account_id`
3. `organizations[0].id`

The raw `id_token` is **never** sent as a header and must not appear in logs.

### Login

```bash
cortex auth login chatgpt --method oauth
```

Browser completes the loopback callback on port 1455. If that port is already bound,
login fails with an actionable error (no silent port hop).

---

## xAI

Provider id **`xai`**. API key (`XAI_API_KEY`) and xAI OAuth are both supported.
OAuth can use the browser code flow or **device code**.

### Defaults

| Field | Value |
| --- | --- |
| Authorize | `https://auth.x.ai/oauth2/authorize` |
| Token | `https://auth.x.ai/oauth2/token` |
| Device authorization | `https://auth.x.ai/oauth2/device/code` |
| Client id (public) | `b1a00492-073a-47ea-816f-4c329264a828` |
| Scopes | `openid` `profile` `email` `offline_access` `grok-cli:access` `api:access` |
| Browser callback | `http://127.0.0.1:56121/callback` (port **56121**, path `/callback`) |
| API base | catalogue / `https://api.x.ai/v1` |
| Protocol | openai-chat |

### Login

```bash
# Browser (pinned 56121)
cortex auth login xai --method oauth

# Device code (prints verification URI + user code)
cortex auth login xai --method device
```

If port **56121** is occupied during browser OAuth, Cortex reports an actionable error
instead of binding a different port (the redirect URI is registered to that pin).

---

## `CORTEX_OAUTH_*` override matrix

Every OAuth field is overridable for fixtures, enterprise proxies, or upstream client
changes without a code patch.

### Precedence (highest wins)

1. `CORTEX_OAUTH_<PROVIDER>_<FIELD>` (provider-specific)
2. `CORTEX_OAUTH_<FIELD>` (generic)
3. Built-in `OAUTH_DEFAULTS` for that provider

Provider segment: uppercase provider id, non-alphanumeric → `_`
(e.g. `chatgpt` → `CHATGPT`, `openai-compatible` → `OPENAI_COMPATIBLE`).

A set override **replaces** the built-in list for extras/headers (it does not merge).

### Field suffixes

| Suffix | Meaning | Value shape |
| --- | --- | --- |
| `AUTHORIZE_ENDPOINT` | Authorization URL | absolute URL |
| `TOKEN_ENDPOINT` | Token URL | absolute URL |
| `DEVICE_AUTHORIZATION_ENDPOINT` | RFC 8628 device authorization URL | absolute URL |
| `CLIENT_ID` | Public OAuth client id | non-empty string |
| `SCOPES` | Scopes | space- or comma-separated |
| `CALLBACK_PORT` | Pinned loopback port | `3200`-`3299`, or pins `1455` / `56121` |
| `CALLBACK_PATH` | Loopback callback path | e.g. `/callback`, `/auth/callback` |
| `AUTHORIZE_EXTRA_PARAMS` | Extra authorize query params | `key=value,key2=value2` |
| `TOKEN_EXTRA_PARAMS` | Extra token body fields | `key=value,key2=value2` |
| `EXTRA_HEADERS` | Extra headers on token/device/refresh HTTP | `Header-Name=value,Other=value` |

### Examples

```bash
# Point ChatGPT OAuth at a local fixture (mechanics tests)
export CORTEX_OAUTH_CHATGPT_AUTHORIZE_ENDPOINT="http://127.0.0.1:3210/authorize"
export CORTEX_OAUTH_CHATGPT_TOKEN_ENDPOINT="http://127.0.0.1:3210/token"

# Generic fallback when no provider-specific key is set
export CORTEX_OAUTH_CLIENT_ID="fixture-client"

# Replace authorize extras entirely for chatgpt
export CORTEX_OAUTH_CHATGPT_AUTHORIZE_EXTRA_PARAMS="id_token_add_organizations=true,codex_cli_simplified_flow=true"

# xAI device endpoint override
export CORTEX_OAUTH_XAI_DEVICE_AUTHORIZATION_ENDPOINT="http://127.0.0.1:3211/device/code"
```

CLI also accepts `--auth-input key=value` for one-shot overrides on `cortex auth login`.

OAuth is refused for a provider that neither has built-in defaults nor enough
`CORTEX_OAUTH_*` values to resolve authorize, token, and client id.

---

## Claude Pro/Max OAuth: unsupported

**Claude Pro and Claude Max subscription OAuth are deliberately not implemented.**

| Fact | Detail |
| --- | --- |
| Why | Anthropic’s terms prohibit third-party clients from using Claude consumer subscription OAuth |
| Product posture | Same as other third-party coding agents that removed Claude subscription OAuth after Anthropic’s legal request (public PR **#18186**, v1.3.0+) |
| What works | Direct Anthropic **API key** via provider `anthropic` and `ANTHROPIC_API_KEY` |
| What will not ship | Borrowed Anthropic OAuth `client_id`, `claude-cli` User-Agent spoofing, forced “You are Claude Code…” system prompts, or request-body identity rewriting |

There is no supported `cortex auth login` path for Claude Pro/Max OAuth. Do not expect
docs or flags for that flow.

---

## Related commands

```bash
cortex auth login <provider> [--method oauth|device|api-key] ...
cortex auth list
cortex auth logout <provider>
cortex models
cortex models --refresh
cortex config --format json
```

Network kill switch:

```bash
CORTEX_NO_NET=1 cortex run "..."
CORTEX_NO_NET=1 cortex net check
```

---

## Implementation map (for contributors)

| Area | Location |
| --- | --- |
| Profiles + `OAUTH_DEFAULTS` + env matrix | `crates/providers/src/provider_profiles.rs` |
| Catalogue memo + refresh | `crates/providers/src/refresh.rs` |
| Base URL / credential resolution | `crates/providers/src/runtime_config.rs` |
| OAuth credential refresh + 401 retry + ChatGPT account header | `crates/providers/src/runtime_auth.rs` |
| PKCE S256 + device poll + loopback pins | `crates/mcp/src/oauth.rs` |
| CLI auth dispatch | `crates/cli/src/auth_cmd.rs` |
