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
| D14 | 2026-10-04 | Step 1 vocabulary shape | Single crate `mimae-core` at repo root (grows into a workspace when adapters land); `Event` uses `seq: u64` as its position/identity in the single-writer log (no separate EventId — YAGNI); `EventPayload::Unknown` catch-all preserves raw JSON so old binaries replay new logs without data loss; message body capped at 2048 bytes via `Message::validate`. | Implementer, recorded for justification |
| D15 | 2026-10-04 | Head-driven team startup protocol | On project boot: Mimae state `initializing`. All agents register profiles (capabilities, roles, model tier, transport tier). Strongest-planning agent becomes `head` unless the ADE/user overrides. Head decides assignments (single authority); user/ADE can edit before start; empty/auto projects get pure head assignment. Coordinator waits for all agents' `ready` pings before flipping state to `ready` and dispatch begins. Project lifecycle: `initializing -> ready -> running -> integrating/completed/failed`. All dialogs/consent/approval UIs belong to the host ADE via callbacks; Mimae emits events and accepts answers only — no UI inside Mimae. | Owner |
| D16 | 2026-10-04 | Late-join policy (Q: what happens when users add CLIs mid-run) | New CLI sessions registering after boot are full members and immediately claim-eligible; the team ready-gate (initializing->ready) is one-shot at boot and never re-engages or regresses on membership change. The head/coordinator absorbs new capacity by re-decomposing pending work (implemented in step 5 planner). | Owner |

| D18 | 2026-10-05 | D3 gate4agent: depend or learn? | Decision: **learn from, do not depend**. gate4agent v0.2.37 (MIT, ZENG3LD, 53 versions over ~4 months) covers Claude Code, Codex, Gemini CLI, OpenCode via Pipe(NDJSON)/PTY/ACP/Daemon transports, but: (1) it does not cover Antigravity, Qwen, Mimo — our v1+ targets; (2) its interchange format is vendor-specific NDJSON, not our normalized `SessionEvent`; (3) it has no concept of Coordinator/TaskGraph/TeamRegistry, leases, or the brief-aligned coordination logic. We keep our concerns thin and already match the same shape at ~250 LoC per CLI. Borrowed ideas: PTY fallback tier (T4) and ACP where a CLI offers it.|

## Open items

- **Antigravity CLI control surface**: not covered in the brief's Section 10. Needs research (structured mode? ACP?) before its adapter. [OPEN]
- **Qwen Code channels**: fork of Gemini CLI — verify headless stream-json / ACP status before adapter work. [OPEN]
- **gate4agent evaluation**: first Phase-1 task; outcome recorded here when done. [OPEN]

## Accepted limitations (recorded 2026-10-04 during review)

- **flock is unix-only**: on Windows the lock stub always returns "acquired" — multi-process safety is unprotected there (matches D7: Linux/macOS first; revisit with Windows support Q12).
- **`Event.schema_version` is reserved, not yet checked**: a v2 log currently parses only through v1 rules; version gate lands with the first real schema change (flash: Unknown catch-all means new *payloads* are safe, but envelope changes need the check).
- **Token usage is not yet accounted in the fold**: no emitter exists until the first real adapter (D1 wave) and UsageReported payload lands then; dead schema rejected in favor of late addition (Unknown catch-all makes that forward-safe).
- **`EventLog::events()` materializes the full log into a Vec**: accepted for clarity; streaming fold path is the one TaskGraph uses. Segmented reading can land with the workspace manager phase if profiles demand it.
- **Duplicate MessageSent deliveries are not deduplicated in the fold**: coordinator dedupes at send time (step 5); fold keeps events idempotent by event identity only.

## Reviewed & cleared on 2026-10-04

- No `unwrap`/`expect`/`panic!` in src/ (grep-verified); no thread spawns; File handles drop-scoped (no leaks); `#![deny(unsafe_code)]` enforced with a single-scoped `#[allow(unsafe_code)]` on the flock wrapper.
- File-level resource lifetime check: `EventLog::open` failure paths drop the `File`; `events()` opens/reads/drops per call; `stream()` borrows while yields - all bounded by caller drop.
