# Resume package for mockup-v3 mission orchestrator

**Mission:** `mis_27669868` / dir  
`/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc`  
**Working dir:** `/projects/cortex-code`  
**Product crate tree is OFF-LIMITS** for this seal track (harness + assets only).

## Queue (exact order)

1. **`v3-png-baseline-full-rerender`** — status `in_progress`  
   skill: `v3-sweep-worker`
2. **`v3-png-baseline-vision-rebind`** — status `pending`  
   skill: `v3-sweep-worker` (do not start until #1 is sealed)

Everything else in features.json is completed. Production flow UX validation has already
passed; residual is **artifact seal only**.

---

## Paste-ready: `messageToWorker` (if resuming the paused worker)

```
Resume v3-png-baseline-full-rerender as the PNG integrity closeout after
commit eb3c312 (CELL_BASELINE=16 baseline registration). Reread:
- features.json entry v3-png-baseline-full-rerender
- library notes under mission library/ about png baseline
- tools-throwaway/mockup-harness-v3/README paths via plans/assets/mockups-v3/README.md
- research/06-mockup-v3-status.md for context

Do exactly this, nothing more:
1) From /projects/cortex-code/tools-throwaway/mockup-harness-v3, publish ALL
   86 canonical + 35 gate PNG+TXT via the public CLI:
   cargo run --quiet --bin mockup-harness-v3 --features png -- --all
   (or the project’s documented serialized publish path if that is what
   sweep_checks expects — mirror the path used in v3-png-baseline-registration-fix).
2) Refresh renderer manifests with the NEW PNG SHAs. Keep TXT 120x40.
3) Prove:
   ./sweep_determinism.sh   # double full render hash-identical, exit 0
   ./sweep_checks.sh        # path/size/glyph inventory, exit 0
   cargo test --features png
   cargo clippy -p mockup-harness-v3 --features png -- -D warnings
4) Materialize a NEW human-gate awaiting_user request for the product-changed
   pack ONLY (review_bundle under evidence/v3-human-gate/<new-digest>/).
   NEVER author or overwrite decision.json yourself. Old approved decisions
   are hash-stale after rehash — bulk-continuity auto-approval does NOT apply.
5) Record old-vs-new sample SHA + live review_bundle_sha256 in library notes.
6) Do NOT start vision rebind in this feature — that is v3-png-baseline-vision-rebind.
7) Real product crates/ under /projects/cortex-code are off-limits.

On handoff: success only if sweeps green AND awaiting_user REQUEST exists without
decision.json authored by the worker.
```

---

## Paste-ready: orchestrator resume brief (for the other session)

```
You are the orchestrator of mission mis_27669868 (Cortex Code TUI mockup v3).
Working directory: /projects/cortex-code.
State reports: research/06-mockup-v3-status.md and research/08-mockup-v3-resume-for-orchestrator.md.

Current state (2026-07-15):
- 143/145 features completed (~98.6%). UX of six flows already validated (pass).
- Open track ONLY: milestone v3-png-baseline-fix
  1) in_progress: v3-png-baseline-full-rerender
  2) pending:     v3-png-baseline-vision-rebind
- Latest useful commit: eb3c312 (CELL_BASELINE registration; PNGs republished but
  vision evidence most-likely stale). Final-sweep user-testing r2 + scrutiny r3 = pass.
- validation-state: 454 passed / 8 pending (baseline-related residuals only).
- Greenfield mission mis_a2ebcb7c shares this repo — do NOT touch crates/ product
  work; harness tools-throwaway/mockup-harness-v3 and plans/assets/mockups-v3* only.

ACTION:
1. Call start_mission_run with restartFeature=false if the paused worker should resume,
   and pass the messageToWorker block from research/08 (section above).
   OR restartFeature=true if the in-progress session is impure — then the same
   message becomes the feature description delta.
2. After full-rerender succeeds: require a NEW human-gate pack at
   evidence/v3-human-gate/<digest>/ with awaiting_user; present it to the product owner.
   Workers must never write decision.json.
3. Once decision.json status=approved for the live digest, unpause/run
   v3-png-baseline-vision-rebind:
   - vision via openrouter-multimodal analyze_image, model anthropic/claude-opus-4.8-fast
   - bind VAL-V3-GATE-012, VAL-V3-CROSS-017/018, VAL-V3-HARNESS-033, VAL-V3-DS-026
   - require monospaced baseline/typography claims on home, chat-turn, one gate frame
   - write library/png-baseline-vision-rebind-notes.md
4. After both features complete → auto scrutiny + user-testing seal for
   v3-png-baseline-fix → if green, mission can be sealed.
5. Port to real TUI is OUT OF SCOPE for this mission; after seal, use
   research/10-tui-port-chrome-first.md as the brief for a NEW mission.

Do not expand scope to greenfield M5/M6/M7 or plans/00-MASTER-PLAN product engine work.
```

---

## Human-gate note for product owner

After rerender, you will be asked to approve a **new** bundle digest (old
`decision.json` files under `evidence/v3-human-gate/*` are stale once PNGs rehash).

- Approve only by creating/updating `decision.json` as product-owner actor with
  matching `request_digest` / pack SHA.
- Reject → open polish features; do not rewrite the rejected pack in place.

Check:  
`./tools-throwaway/mockup-harness-v3/check_gate_approval.sh`  
`/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc/evidence/v3-human-gate`
