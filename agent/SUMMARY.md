# Summary — what happened while you were away

## Done
- Bootstrapped repo: concept brief imported, agent state, decision log (D1–D14 resolved).
- Phase 0 decisions recorded with your answers (targets: opencode, Antigravity CLI, Claude Code, Codex CLI, Qwen Code; core-stable-first; gate4agent evaluated first; MIT OR Apache-2.0).
- **Foundation step 1 complete**: `mimae-core` crate — versioned event vocabulary:
  - `Event` envelope (schema_version, seq, actor, payload, causal refs, time), `Actor`, `KnownPayload` (14 typed variants), `EventPayload::Unknown` raw-preserving fallback (old binaries replay new logs losslessly; malformed known tags fail loudly).
  - Entities: Goal/Budget, Task (full lifecycle states), Agent, Lease, Contract, Decision, Message (closed 11-type taxonomy, 2048-byte body cap enforced).
  - Typed IDs (GoalId/TaskId/AgentId/...) serialize as plain numbers.
  - Verified: 8/8 tests pass, `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt --check` clean. Committed `e102a07`.

## In progress
- Nothing — step 1 closed.

## Blocked — needs a decision
- None blocking. Open (non-blocking): gate4agent evaluation outcome; Antigravity CLI control surface (undocumented in brief — needs research before its adapter); Qwen Code channel verification.

## Queued next (foundation, in order)
1. Step 2: append-only event log (JSON-lines, single writer, crash-safe replay)
2. Step 3: task graph + atomic claim, concurrent-claim stress test
3. Step 4: leases + heartbeats, expiry self-recovery
4. Step 5: deterministic coordinator against fake agents (scripted goal end-to-end)
5. Step 6: fake agent harness (success/fail/hang/crash on cue), crash-reassignment test
6. Then: gate4agent evaluation → first real adapter (opencode)
