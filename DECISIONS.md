# Mimae Decision Log

Per mimae-concept.md Section 0 rule 5. Every resolved [OPEN] item or changed [PROPOSED] item is recorded here.

| # | Date | Question (brief ref) | Decision | Source |
|---|------|----------------------|----------|--------|
| D1 | 2026-10-04 | Q1: First CLIs | opencode, Antigravity CLI, Claude Code, Codex CLI, Qwen Code. New CLIs roll out later. (Gemini CLI slot replaced by Antigravity CLI.) | Owner |
| D2 | 2026-10-04 | Scope of first delivery | Rust only — core library + adapters. No daemon, no bindings until core is stable (brief Phase 9). | Owner |
| D2b | 2026-10-04 | Build strategy | Foundation and core made stable first (event model, state store, task graph, leases, coordinator against fake agents), then real adapters, then higher layers. Core-heavy ordering, phases 4-9 follow the brief. | Owner |
| D3 | 2026-10-04 | Q5: Reuse transport crates | Evaluate gate4agent first; depend only if licence/activity/coverage (incl. antigravity, qwen) hold. Otherwise own adapter layer using its shapes as prior art. | Owner |
| D4 | 2026-10-04 | Q4: Licence | MIT OR Apache-2.0 (dual). | Owner |
| D5 | 2026-10-04 | Q2: Local-first | Confirmed per brief Section 6 resolution (P1 hard). | Brief default, unopposed |
| D6 | 2026-10-04 | Q3: Planner/worker hierarchy | Accepted, automatic (brief Section 6 resolution). | Brief default, unopposed |
| D7 | 2026-10-04 | Q12: Windows | Linux/macOS first; Windows best-effort later. PTY/process code isolated per-OS from day one. | Default, unopposed |
| D8 | 2026-10-04 | Q7: Success criteria | Match-or-beat solo on CooperBench-style pairs; coordination tokens < naive chat; zero lost work on crashes; zero human relays in scenarios (brief 13.4). | Default, unopposed |
| D9 | 2026-10-04 | Q10: Nexa | Moot — not in first-wave target list. | Owner |
| D10 | 2026-10-04 | Q8: discovery detection | Explicit structured reports + lightweight rules first; classifier later (brief recommendation). | Default, unopposed |
| D11 | 2026-10-04 | Q6: MCP shim | Optional, off by default (brief recommendation). | Default, unopposed |
| D12 | 2026-10-04 | Q11: A2A bridge | Later phase, not core (brief recommendation). | Default, unopposed |
| D13 | 2026-10-04 | Q9: Sandboxing | Worktrees first; containers optional per host. | Default, unopposed |

## Open items

- **Antigravity CLI control surface**: not covered in the brief's Section 10. Needs research (structured mode? ACP?) before its adapter. [OPEN]
- **Qwen Code channels**: fork of Gemini CLI — verify headless stream-json / ACP status before adapter work. [OPEN]
- **gate4agent evaluation**: first Phase-1 task; outcome recorded here as D14 when done. [OPEN]
