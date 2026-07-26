# TUI port plan — shared chrome first (post mockup-v3 seal)

> **Do not start this mission until** mockup v3 is sealed  
> (features `v3-png-baseline-full-rerender` + `v3-png-baseline-vision-rebind` done,  
> human-gate approved on the **live** PNG digest, mission validators green).

## Goal

Port the validated v3 **visual language and shell anatomy** into the real product TUI
(`crates/tui`, `crates/tui-components`, `crates/frame`) without waiting for 6-flow 
full behavioral rebound. Pixel-faithful design-match in a truecolor terminal
(design-match, not PNG byte-diff).

## Source of truth (after seal)

| Asset | Path |
| --- | --- |
| Gallery 86 states | `plans/assets/mockups-v3/` (+ README) |
| Gate frames 35 | `plans/assets/mockups-v3-gates/` |
| Harness implementation of chrome | `tools-throwaway/mockup-harness-v3/src/**` |
| Design do/don't living log | skill `cortex-code-design` + any `library/*design*` notes in mission |
| Comparison-only | v2 harness + gallery remain immutable |

## P0 milestone — Shared chrome (this plan’s first mission)

### Scope IN

1. **Palette tokens**  
   mono-orange system: `ORANGE` / `ORANGE_D` / `AMBER` on near-black `#0d0d0d` (or final
   sealed tokens from v3 design foundation). Explicit background on every cell. No
   residual “ocean_cyan as dark” defaults.
2. **Four-surface shell** (persistent anatomy, empty states ok at P0):  
   - conversation / transcript surface  
   - action dock  
   - contextual inspector  
   - temporary detail view  
   Ratatui layout matches mock regions, gutters, title rules, safe glyphs only  
   (`✦`, `▶`/`▼`, rounded `╭╮╰╯`; **no braille**).
3. **Shared chrome primitives**  
   cards, panels, modals, selection-bar (inverse), statusline segments, banner task line,
   toast chrome area. Prefer shared components over per-screen snowflakes.
4. **Themes gallery skeleton**  
   at least sealed default + one alternate; preview must render each theme in its palette.
5. **TTY-gated CLI style** left mostly for a later milestone; P0 may touch only shared
   tokens if CLI and TUI share a style crate.

### Scope OUT of P0 (explicit)

- Full Flow 1–6 behavioral/port completeness  
- Mission/workflow orchestration UX (Flow 6)  
- Provider/MCP connect progressive disclosure full rewrite  
- Engine 7-step / plans/00-MASTER-PLAN semantics  
- Brand-new agent capabilities  
- Pixel-diff vs PIL PNG (design-match via capture + vision, not byte equal)

## P0 acceptance (sketch for future validation-contract)

- Launch real `cortex` TUI in truecolor terminal; capture via tuistory/PTY.  
- Home / empty / one chat scaffold / dock present / inspector present visually align with
  sealed v3 counterparts (layout columns, tokens, glyphs, density).  
- Selection and focus rings use inverse bar, not decorative noise.  
- `--no-anim` still works.  
- No regressions on headless `cortex run` JSON/JSONL byte-stable suite.  
- Design skill `cortex-code-design` rules respected and extended with any new owner
  directives from seal review.

## Follow-on waves (after P0 lands)

| Wave | Content |
| --- | --- |
| P1 | Flow 1 conversation end-to-end in product TUI (stream, tool cards, continue) |
| P2 | Flow 2 input + permissions + diff apply chrome |
| P3 | Flow 3 model/agent/session unified picker |
| P4 | Flow 4 provider + MCP connect progressive |
| P5 | Flow 5 recovery / errors |
| P6 | Flow 6 mission/workflow exception-first surface |
| P7 | Headless CLI human-TTY styling parity with chrome tokens |

Each wave = its own milestone with tuistory + live OpenRouter only where the flow requires
a real turn (P1+). Chrome P0 can prove offline against fixture transcript replay.

## Constraints for the future mission

- Product code may change `crates/tui*`, `crates/frame`, shared style modules only for P0.  
- Keep greenfield residuals (research/09) on a separate queue.  
- Never reintroduce competitor brand strings.  
- Kill by exact PID in tests; ports 3100-3199 / reserved go 3200-3299 for serve.  
- Format on-touch (`rustfmt --edition 2024 --skip-children`).  
- Mockup harness remains the visual reference; do not delete after port.

## What to drop into a new mission proposal later

Copy this file + sealed gallery README + 3–5 “reference PNGs” from home/chat/dock/inspector
into the proposal’s Expected Functionality as **M0 Shared chrome**. Keep subsequent flows
as M1+ so the first validation gate can land early.
