# Journal

## 2026-10-04 — Cold start, workspace bootstrap

- User provided concept brief at ~/storage/downloads/mimae-concept.md; copied to repo root.
- Read the full 681-line document: Mimae is a Rust backend coordination layer enabling multiple agentic coding CLIs to self-organize on one project. Hard constraints: backend library (not app), embeddable, no manual assignment, no handoffs, no MCP, faster/smarter, in Rust. Key thesis: remove the human router, keep structure — planner-driven assignment, typed sparse messages, shared state primary, verification gates, Mimae owns git ops.
- Initialized workspace: git repo, agent state, backlog.
- Next: Phase 0 (decisions) — doc has 12 open questions for the owner. Sensible default path: adopt Section 17 recommendations as defaults (documented in decision log), start Phase 1 adapter foundation with Claude Code as first target.

## 2026-10-04 — Step 1: event vocabulary

- Owner declined TDD workflow; implementation-first, tests as verification.
- Owner decisions recorded: D1 targets (opencode, Antigravity CLI, Claude Code, Codex CLI, Qwen Code), D2b core-stable-first build strategy, D3 gate4agent evaluated first, D4 MIT OR Apache-2.0, D5/D6/D7/D8/D9/D10/D11/D12/D13 brief defaults confirmed.
- Built mimae-core (single crate): ids.rs (typed u64 IDs), actor.rs (tagged Actor), entities.rs (Goal/Budget/Task/Agent/Lease/Contract/Decision/Message + TaskStatus per brief 8.2, MessageKind closed taxonomy per 8.3, body cap 2048 via validate()), event.rs (Event{schema_version, seq, actor, payload, causes, at_ms}, KnownPayload tagged enum, EventPayload::Unknown raw-preserving catch-all, malformed-known-tag errors).
- tests/vocab.rs: 8 tests — per-variant round-trip, tag-list consistency, unknown preservation, malformed-known-tag error, wire-format snapshot, body cap, id and actor serde shapes. All pass; clippy -D warnings clean; cargo fmt applied.
- Next: step 2, append-only event log with crash-safe replay.

## 2026-10-04 — Step 2: event log

- `src/log.rs`: `EventLog::open(path, options)` — append JSON-lines under O_APPEND, flock single-writer (unix, nb), fsync per append (Durability::SyncData default, FlushOnly option). Open scan distinguishes torn tail (truncated silently, recovered) vs corrupt middle line (hard LogError::Corrupt). `EventStore` trait (append/events/high_water_seq/len/is_empty) so storage is swappable (P2). No clock reads in lib — callers pass at_ms.
- tests/log.rs: 7 tests incl. torn-tail recovery, corrupt-line hard error, second-writer rejection until drop, seq/causes preserved in file order, trait-object usage. All pass; clippy clean; fmt applied. Committed 091522a.

## Prior art to study before adapters (Section 18)

ACP schema, gate4agent/fno-agents/fux adapters, Claude Code Agent Teams, Cursor planner/worker, C-compiler harness, CooperBench, MAST, Orca orchestration.
