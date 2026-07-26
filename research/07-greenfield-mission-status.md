# Greenfield Mission Status (Cortex Code)

**Analysis only — no code changes.**  
**Mission:** `/root/.factory/missions/a611e317-f6eb-4bc0-80fa-eb5387ddc00c` (`mis_a2ebcb7c`)  
**Product WD:** `/projects/cortex-code`  
**Report date:** 2026-07-15  
**Mission `state.json`:** `running` (updated 2026-07-15T14:00:11Z); `lastReviewedHandoffCount`: 142  
**Note:** A root `blocked.marker` (empty, 2026-07-15) exists under the product tree; purpose unclear from mission metadata alone.

---

## 1. Features (`features.json`)

| Metric | Count |
|--------|------:|
| Total features | 176 |
| **completed** | **132** |
| **in_progress** | **1** |
| **pending** | **43** |
| Non-completed | 44 |

### By milestone

| Milestone | completed | in_progress | pending | total |
|-----------|----------:|------------:|--------:|------:|
| m0-skeleton | 15 | 0 | 0 | 15 |
| m1-llm-runner | 16 | 0 | 0 | 16 |
| m2-tools-permissions-checkpoints | 19 | 0 | 0 | 19 |
| m3-tui | 26 | 0 | 0 | 26 |
| m4-extensibility | 42 | 0 | 0 | 42 |
| **m5-plugins-web-audit** | **14** | **1** | **5** | **20** |
| **m6-server-acp** | **0** | **0** | **18** | **18** |
| **m7-finishing** | **0** | **0** | **15** | **15** |
| misc-cleanup-1 | 0 | 0 | 1 | 1 |
| misc-hardening-1 | 0 | 0 | 4 | 4 |

**Interpretation:** M0–M4 are fully claimed complete. The mission is mid–late: still closing M5 scrutiny recovery + validator re-run, then entire formal M6 and M7 feature queues remain open (even if product code has advanced ahead of tracker — see §4).

### Remaining non-completed features

| id | status | milestone | skill | one-line desc |
|----|--------|-----------|-------|---------------|
| m5-fix-command-path-audit-sink | in_progress | m5-plugins-web-audit | agents-plugins-worker | RECOVERY: verify prior commit `6987ff1` command-path audit sink; min-diff only if gaps |
| m5-fix-background-bash-exit-audit | pending | m5-plugins-web-audit | agents-plugins-worker | Record background bash final exit/cwd on complete/reap, not only start (`exit_status: null`) |
| m5-fix-request-hook-order-contract | pending | m5-plugins-web-audit | agents-plugins-worker | Publish VAL-PLUGIN-026 configuration-order request-hook contract as plugin-author-facing docs |
| m5-fix-server-dispose-on-shutdown | pending | m5-plugins-web-audit | agents-plugins-worker | VAL-PLUGIN-025: dispose plugin host on clean `serve` shutdown, not per message Drop |
| scrutiny-validator-m5-plugins-web-audit | pending | m5-plugins-web-audit | scrutiny-validator | Re-run M5 scrutiny after fix features close |
| user-testing-validator-m5-plugins-web-audit | pending | m5-plugins-web-audit | user-testing-validator | M5 user-testing after scrutiny |
| m6-server-lifecycle | pending | m6-server-acp | server-worker | `cortex serve` lifecycle: loopback 3200–3299, `/health`, busy port, graceful shutdown |
| m6-server-session-crud | pending | m6-server-acp | server-worker | HTTP session create/list/get/delete via shared db |
| m6-server-session-validation | pending | m6-server-acp | server-worker | 404/4xx + JSON content-type discipline on session routes |
| m6-server-prompt-turn | pending | m6-server-acp | server-worker | Prompt/message/abort endpoints driving real OpenRouter turns |
| m6-server-sse-event | pending | m6-server-acp | server-worker | SSE `/event` from typed event bus |
| m6-server-openapi | pending | m6-server-acp | server-worker | OpenAPI document at `/openapi` |
| m6-server-auth-middleware | pending | m6-server-acp | server-worker | Optional `--public`/`--token` + CORS/preflight |
| m6-server-remote-control | pending | m6-server-acp | server-worker | Remote TUI control: append/submit/clear buffer |
| m6-server-local-share | pending | m6-server-acp | server-worker | Loopback-only local share (no external upload) |
| m6-acp-handshake-prompt | pending | m6-server-acp | server-worker | ACP subset stdio handshake + prompt |
| m6-acp-protocol-multisession | pending | m6-server-acp | server-worker | ACP JSON-RPC errors, multi-turn, multi-session |
| cross-m6-config-layering-all-surfaces | pending | m6-server-acp | core-worker | Config/policy/agent layers identical across headless/TUI/server |
| cross-m6-firstrun-reachability-discovery | pending | m6-server-acp | server-worker | First-run / discovery across surfaces |
| cross-m6-memory-plugins-surface-agnostic | pending | m6-server-acp | agents-plugins-worker | Memory + plugins work on server too |
| cross-m6-security-invariants-all-surfaces | pending | m6-server-acp | agents-plugins-worker | Security/audit invariants uniform across surfaces |
| cross-m6-server-tui-remote-control | pending | m6-server-acp | server-worker | Server↔TUI remote control integration |
| cross-m6-session-visibility-server-acp | pending | m6-server-acp | server-worker | Server/ACP sessions visible in shared store |
| cross-m6-shared-store-persistence-concurrency | pending | m6-server-acp | core-worker | Shared store consistency under restart/concurrency |
| m7-model-catalog | pending | m7-finishing | core-worker | Offline `cortex models` from embedded/cache catalog |
| m7-model-refresh | pending | m7-finishing | core-worker | Live `cortex models --refresh` (models.dev) |
| m7-stats | pending | m7-finishing | core-worker | Usage/cost stats from persistence |
| m7-search | pending | m7-finishing | core-worker | Working-tree `cortex search` |
| m7-export-import | pending | m7-finishing | core-worker | Session export/import round-trip + redaction |
| m7-session-cli | pending | m7-finishing | core-worker | Session list/show/delete + import durability |
| m7-replay | pending | m7-finishing | core-worker | Real `cortex replay` of stored transcript |
| m7-auth-lifecycle | pending | m7-finishing | core-worker | `auth login/list/logout` + 0600 store |
| m7-auth-precedence | pending | m7-finishing | core-worker | Env vs store precedence; secret protection |
| m7-completions-man | pending | m7-finishing | core-worker | Shell completions + man pages |
| m7-minimal-build | pending | m7-finishing | core-worker | `--no-default-features` offline binary |
| m7-no-mock-hardening | pending | m7-finishing | core-worker | Live-by-default no-mock hardening |
| m7-brand-sweep | pending | m7-finishing | core-worker | Final anti-`opencode` brand sweep |
| m7-readme-docs | pending | m7-finishing | core-worker | Accurate user README |
| cross-m7-finishing-aggregation-roundtrips | pending | m7-finishing | core-worker | M7 stats/export etc. across surfaces |
| misc-cleanup-lint-debt-1 | pending | misc-cleanup-1 | core-worker | Batch non-blocking lint/style debt (max 6) |
| m2-hardening-debt-1 | pending | misc-hardening-1 | tools-worker | Non-blocking M2 robustness debt batch |
| m3-fix-trust-overrides-saved-decision | pending | misc-hardening-1 | tools-worker | SECURITY: trust-deny must beat saved allow-always (VAL-CROSS-031) |
| m3-tui-altarrow-nav-robustness | pending | misc-hardening-1 | tui-worker | TUI message nav polish |
| m3-tui-full-theming | pending | misc-hardening-1 | tui-worker | Theme through full render path, not footer only |

---

## 2. Last handoffs (unfinished work & discovered issues)

### Latest three (2026-07-15 ~13:46–13:52) — M5 fix features

All three report `successState: success` but handoff body is **endfeature-proxy auto-fill stubs** (empty `whatWasLeftUndone`, empty `discoveredIssues`, no real command evidence):

1. `m5-fix-command-no-net-before-plugin-load` → commit `a0ef208` — “Feature work completed (auto-filled).”
2. `m5-fix-pure-mode-plugin-management` → commit `f14574b` — same stub.
3. `m5-fix-websearch-result-redaction` → commit `49a226d` — same stub.

Git history confirms those commits exist plus subsequent `6987ff1 fix(audit): attach session audit sink on run --command path` (matches the recovery feature still `in_progress`).

### Most useful earlier handoffs (2026-07-11)

**Scrutiny M5 (`scrutiny-validator-m5-plugins-web-audit`) — FAILURE, returnToOrchestrator**

- Isolated workspace tests, `cargo check`, clippy all green.
- Synthesis: **7 blocking issues** across 6 of 11 reviewed M5 features.
- **Blocking discovered issues** (mission library / fix features now track most):
  1. `cortex run --command` loaded plugins before `CORTEX_NO_NET` (loopback discovery under kill switch).
  2. Request-hook order only private knowledge — need author-facing contract (VAL-PLUGIN-026).
  3. Server dispose hooks run per message Drop, not serve-lifetime shutdown (VAL-PLUGIN-025).
  4. `--pure plugin install/update` still ran validation probes.
  5. Websearch result strings not redacted before model-facing `ToolOutput`.
  6. `run --command` lacked attached audit sink.
  7. Background bash audit records startup only, not final exit status.
- Left undone: implement those seven fixes before user-testing / re-scrutiny.

**Cross M5 (`cross-m5-extensibility-single-turn`) — success**

- Live OpenRouter proof: one turn called MCP `demo_add`, external Bun plugin hooks, permission, typed audit; children reaped.
- **Discovered non-blocking:** concurrent dirty/untracked work under `tools-throwaway/mockup-harness-v3` and fixtures — intentionally not touched (coexistence warning).

**m5-audit-integrity — success (commit ~`6f11dc2`)**

- Durable sequencing under OS file lock; unique plugin-veto audit; real subagent file_mutation attributed to child session.

**Recovery posture today:** Of the seven scrutiny findings, git shows landed fixes for no-net-before-plugin-load, pure-mode plugin mgmt, websearch redaction, and command-path audit sink. Still explicitly queued: **background bash exit audit**, **server dispose-on-shutdown**, **request-hook-order docs**, then **re-scrutiny + user-testing**.

---

## 3. Validation (`validation-state.json`)

| Status | Count |
|--------|------:|
| **passed** | **386** |
| **pending** | **226** |
| **failed** | **0** |
| Total assertions | 612 |

**Pending by family (approx.):** VAL-FINISH 76 · VAL-SERVER 45 · VAL-PLUGIN 35 · VAL-CROSS 29 · VAL-AUDIT 18 · VAL-WEB 13 · VAL-ACP 10.

**Passed families (M0–M4 bulk):** VAL-TUI 95 · VAL-SKEL 69 · VAL-TOOLS 45 · VAL-LLM 33 · VAL-MCP 22 · plus agent/perm/lsp/ckpt/fmt/run/mem/skill.

**Freshness caveat:** `validation-state.json` mtime is **2026-07-11** (pre most 2026-07-15 M5 fix commits). Pending PLUGIN/AUDIT/WEB counts likely include assertions that product code may already satisfy, but the mission has not re-asserted them via M5 re-scrutiny/user-testing.

---

## 4. Real product vs original M0–M7 greenfield (completeness judgment)

### Mission plan (from `mission.md` + product README)

| Milestone | Intent | Mission tracker | Product tree signal |
|-----------|--------|-----------------|---------------------|
| M0 skeleton | workspace, config, events, db, redaction, no-net, CLI | **done** | Foundation crates present |
| M1 LLM + runner | real OpenRouter, durable loop | **done** | `llm`, `providers`, `engine`, live proofs historically |
| M2 tools/perm/ckpt | tools, permissions, shadow-git | **done** | `tools`, `permissions`, `checkpoints`, etc. |
| M3 full TUI | streaming markdown, tools UI, slash, themes… | **done** (formal) | Full TUI crate stack + heavy post-design mockup work in parallel missions |
| M4 extensibility | MCP, LSP, formatters, task, skills, memory | **done** | crates + live cross proofs |
| M5 plugins/web/audit | hooks, webfetch/search, audit | **~70% tracker; finishing fix queue** | crates present; Jul-11 scrutiny left residual correctness gaps |
| M6 server/API/SSE/ACP | axum loopback, OpenAPI, SSE, remote TUI, ACP | **0/18 features completed on tracker** | **Substantial code already present:** `crates/server/**`, `crates/acp/**`, `cli` commands `Serve`, `Acp`, `Attach`, OpenAPI modules, TS SDK |
| M7 finishing | models catalog/refresh, stats, export/import, replay, completions/man, auth polish, brand, docs | **0/15 features completed on tracker** | **CLI modules present and non-trivial:** `models_cmd`, `stats`, `export_import`, `replay`, `search`, `session`, `completion_cmd`, `man_cmd`, `auth_cmd`; root `README.md` already reads productized |

### Crate map (workspace layers, from README)

Hard evidence the product is far past a skeleton: foundation (`protocol`, `errors`, `events`, `redaction`, `settings`, `db`), model/engine (`llm`, `providers`, `engine`, `format`, `memory`), tools/policy (`tools`, `permissions`, `policy`, `checkpoints`, `mcp`, `lsp`, `plugins`, `skills`, `audit`), TUI stack (`tui-*`, `frame`, `tui`), surfaces (`server`, `acp`, `cli`), plus `sdk/`.

### Overall judgment

**Not “effectively done with residual polish.”**

- On the **mission gate**: still **mid–late implement / late M5 → open M6+M7 formal queue**. 44 features open; 226 assertions pending; no failed assertions, but huge unfinished validation surface.
- On the **shipping codebase**: **much of M0–M5 is real and integrated**; M6/M7 **look largely scaffolded or implemented ahead of the greenfield feature IDs**, so residual work is a mix of (a) true M5 correctness closeouts, (b) missionization/proofs for already-present M6/M7 surfaces, (c) security debt (`m3-fix-trust-overrides-saved-decision`), and (d) polish/lint/brand.

**Best one-liner:** Greenfield is **not residual polish only** — it is **~M5 closeout + full formal M6/M7 mission completion still remaining**, with product code **ahead of the feature tracker** for server/finishing surfaces.

---

## 5. Conflict risk with mockup-v3 (same WD)

| Item | Detail |
|------|--------|
| Concurrent mission | `7060a79b-ccfb-445c-9e82-ee4504b320dc` — **Cortex Code TUI v3 mockups** (`mis_27669868`) |
| State | **running** (updated 2026-07-15T14:01:25Z, shortly after greenfield) |
| Working directory | **same:** `/projects/cortex-code` |
| Mockup tracker | ~143 completed / 1 in_progress / 1 pending (145 total) — near done, still active |
| Coexistence policy | Greenfield `AGENTS.md` explicitly warns of unrelated dirty/untracked work under `tools-throwaway/mockup-harness` (and related paths); never reset/clean/commit foreign work |
| Evidence of interleaving | Recent `git log` shows long streaks of `fix/test/feat(mockups-v3)` sentinels **interleaved with** greenfield `fix(audit|plugins|websearch)` commits |
| Active conflict zones | `tools-throwaway/mockup-harness*`, root research/plans assets; risk elevates if either mission edits shared product crates (`crates/tui/**`, `cli`, etc.) |
| Dirty tree observations | Uncommitted: `.gitignore`, `crates/skills/src/lib.rs`; untracked: `.firecrawl/`, `.omo/`, `blocked.marker`, plans zip, `research/05-gap-analysis.md` |

**Conflict risk: HIGH operationally, MEDIUM for product crates if workers stay scoped.** Shared git history + two running missions + dirty unrelated paths means greenfield workers can easily disturb mockup artifacts (or vice versa) or race on commits. Running both coordination-active is the main risk surface for TUI/product crates.

Also note: another mission history entry `17e11d9b` appears to be a **completed** earlier greenfield clone of the same plan; live tracker for this report is `a611e317…` (still `running`).

---

## Recommendation (for orchestrator)

| Option | Fit? | Why |
|--------|------|-----|
| Leave alone / residual polish only | **No** | M5 not Mission-complete; M6+M7 feature IDs all pending; 226 assertions still pending |
| Pause (concept) | **Strong if** mockup-v3 / other work owns the tree | Avoids dual-writer collision on one git repo |
| Complete residual mission features | **Yes for M5 closeout first**; M6/M7 next | 3–4 M5 fix items + dual validators, then formal server/finishing proof pass (may be lighter if code already lands) |

**Suggested stance:** Treat greenfield as **still active mid–late**, not complete. Prefer **pause greenfield while mockup-v3 finishes** (nearly closed) *or* hard-serialize writers; if continuing greenfield, finish remaining **M5 scrutiny findings** first so M5 can clear, then either claim/proof existing M6–M7 code or implement gaps.

---

## Sources

- `/root/.factory/missions/a611e317-f6eb-4bc0-80fa-eb5387ddc00c/{features.json,validation-state.json,state.json,mission.md,handoffs/*,AGENTS.md}`
- `/projects/cortex-code/{README.md,crates/,Cargo.toml,git log/status}`
- Mockup-v3 mission `7060a79b-ccfb-445c-9e82-ee4504b320dc` state/features (WD conflict check only)
