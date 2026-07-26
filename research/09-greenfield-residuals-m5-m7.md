# Greenfield mission residuals (M5 / M6 / M7) — documentation only

**Mission:** `mis_a2ebcb7c` /  
`/root/.factory/missions/a611e317-f6eb-4bc0-80fa-eb5387ddc00c`  
**Product:** `/projects/cortex-code`  
**Policy (locked 2026-07-15):** **do not execute** this mission until mockup v3 is sealed.
Document residuals only. Status derived from features.json + handoffs + research/07.

## Snapshot

| Metric | Value |
| --- | --- |
| Features | 176 total |
| Completed | 132 |
| In progress | 1 (`m5-fix-command-path-audit-sink`) |
| Pending | 43 |
| Validation assertions | 386 passed / 226 pending / 0 failed |
| Product reality | Crate map + README already expose serve/ACP/SDK/stats/export/replay — **code often ahead of tracker IDs** |

## M0–M4

All tracker features completed (skeleton, live LLM, tools/permissions, full TUI, extensibility core).

## M5 — plugins / web / audit (closeout)

Still open on the tracker (names may partially be fixed in the working tree):

| Feature id (representative) | Intent |
| --- | --- |
| `m5-fix-command-path-audit-sink` | **in_progress** — wire audit sink on CLI command path |
| `m5-fix-background-bash-exit-audit` | bash job exit reported into audit |
| `m5-fix-server-dispose-on-shutdown` | plugin `dispose` on server shut-down |
| `m5-fix-request-hook-order-contract` | document/enforce request hook order |
| `m5-fix-*` (other scrutiny follow-ups) | plugin / no-net / redaction / pure-mode |
| `scrutiny-validator-m5-plugins-web-audit` | re-run after fixes |
| `user-testing-validator-m5-plugins-web-audit` | e2e re-test after fixes |

**Jul-11 scrutiny themes still relevant until re-gate:** background bash exit audit, server dispose on shutdown, hook-order contract collated; several other plugin/no-net items already land in dirty working tree (uncommitted: `plugins` loopback + `CORTEX_NO_NET`, `skills` YAML block scalars, plugin_runtime tests). Those dirty edits belong to greenfield and should **not** mix into v3 seal commits.

## M6 — server / API / SSE / ACP (18 pending)

Tracker still has an M6 list even though much of `crates/server` + `crates/acp` + `sdk/` exists in tree:

- lifecycle, bind loopback, session CRUD + validation  
- prompt turn path, SSE `/event`  
- OpenAPI document  
- remote TUI control  
- local share  
- ACP subset methods  
- related validation features  

**Rule when resuming:** audit which IDs are “claim with live proof” vs “true missing circuit”; do not re-implement complete surfaces.

## M7 — finishing (15 pending) + misc

Typical M7 bag (from original proposal; verify against live features.json on resume):

- models.dev refresh path hardening  
- stats / usage / costs  
- export / import  
- completions / man  
- real `replay`  
- user docs  
- anti-brand CI guard  
- no-mock e2e hardening  
- misc-hardening-1 / misc-cleanup-1 leftovers  

## Conflict with mockup v3

- Same working directory `/projects/cortex-code`.  
- Mockup v3 may only touch `tools-throwaway/mockup-harness-v3`, `plans/assets/mockups-v3*`, and mission evidence.  
- Greenfield owns `crates/**`, `sdk/**`, product README changes.  
- **Do not** `start_mission_run` greenfield while seal is open; uncommitted greenfield fixes should stay uncommitted or be committed under greenfield-only messages after v3 seal (never fold into mockup commits).

## Resume order AFTER mockup v3 seal

1. Park/review dirty tree (`git status`) — separate greenfield commit(s).  
2. Finish M5 residual features → re-run M5 scrutiny + user-testing.  
3. Inventory M6: map each pending id to existing server/ACP code; claim-with-proof or implement gaps.  
4. M7 finishing + README accuracy pass.  
5. Do **not** start `plans/00-MASTER-PLAN` 7-step engine redesign until product owner re-opens that track (separate mission).

## Out of scope while paused

- No new greenfield features authored from this session.  
- No port of mockup chrome into `crates/tui` under the greenfield mission id (that is a **new** TUI-port mission; see research/10).
