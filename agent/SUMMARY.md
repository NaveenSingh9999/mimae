# Summary — what happened while you were away

- Phase 0.5 decisions D16 (late join = instant capacity, no re-gate) and D17 (hierarchical heads, no committee chat) recorded.
## Done
- Bootstrapped repo: concept brief imported, agent state, decision log (D1–D14 resolved).
- Phase 0 decisions recorded with your answers (targets: opencode, Antigravity CLI, Claude Code, Codex CLI, Qwen Code; core-stable-first; gate4agent evaluated first; MIT OR Apache-2.0).
- **Foundation step 1 complete**: `mimae-core` crate — versioned event vocabulary:
  - `Event` envelope (schema_version, seq, actor, payload, causal refs, time), `Actor`, `KnownPayload` (16 typed variants incl. team_created / agent_ready), `EventPayload::Unknown` raw-preserving fallback (old binaries replay new logs losslessly; malformed known tags fail loudly).
  - Entities: Goal/Budget, Task (full lifecycle states), Agent, Lease, Contract, Decision, Message (closed 11-type taxonomy, 2048-byte body cap enforced).
  - Typed IDs (GoalId/TaskId/AgentId/...) serialize as plain numbers.
  - Verified: 8/8 tests pass, `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt --check` clean. Committed `e102a07`.
- **Step 2 complete**: `mimae-core/src/log.rs` — append-only JSON-lines `EventLog`, flock single-writer (unix), fsync per append, torn-tail truncation recovery vs corrupt-line hard error, `EventStore` trait for swappable storage. 7/7 tests pass, clippy/fmt clean. Committed `091522a`.

- **Step 3 complete**: `TaskGraph` fold over `EventStore::stream()` (O(tasks) memory, strict 8.2 transition table, dep-gated readiness, deterministic sorted ready list); atomic `claim()` (NoSuchAgent/DepsNotMet/StatusNotClaimable/ActiveLease errors); `TeamRegistry` driving `initializing → ready → running` from D15; concurrency stress + from-log replay equality covered. 27/27 tests, clippy `-D warnings` clean, fmt clean. Committed `c41a22b`.

- **Step 4 complete**: lease TTLs are now first-class (`Lease.ttl_ms`), heartbeats extend expiry by that TTL (fixes a latent bug where heartbeats never kept a lease alive), and `Coordinator::check_for_expired_leases(now_ms)` is the query-time expiry sweep — a dropped lease flips that task back to `Ready` and emits exactly one `LeaseExpired` event (fold also covers `InReview` now). No timer thread; deterministic given caller-supplied time. 36/36 tests, clippy/fmt clean.
- **Step 6 complete**: first real adapter — `OpenCodeAdapter` (src/opencode.rs). Spawns `opencode serve` on loopback, health-polls, creates session, returns normalized SessionEvents from POST .../message responses, aborts/resume/inject via dedicated endpoints, drops the child on teardown. Live conformance test runs against the actual `opencode` binary when available (skipped otherwise). 41 tests, clippy/fmt clean, commit e12eb08.
- **Step 3.5 complete**: centralized `Coordinator` — the single ingest point owning EventLog + TaskGraph + TeamRegistry. `publish`/`claim` are validate-then-commit (dry-run on cloned views → append → swap), so rejected events never enter the log (replays stay clean). D16 recorded: late-joining CLIs are full members, immediately claim-eligible, boot ready-gate is one-shot. D17 recorded: hierarchical heads (head → sub-planners beyond a measured threshold, typed contracts as the only head-channel, single integrator). 31/31 tests, clippy/fmt clean, commit 5113dea.





## Blocked — needs a decision
- None blocking. Open (non-blocking): gate4agent evaluation outcome; Antigravity CLI control surface (undocumented in brief — needs research before its adapter); Qwen Code channel verification.

## Queued next (foundation, in order)
7. Step 7: Claude Code T2 headless JSON/stream-json adapter + hooks (code-ready, live conformance later on a box with claude installed)
8. Step 8: Antigravity CLI adapter after verifying its driver surface
9. Step 9: gate4agent prior-art evaluation (D3)
