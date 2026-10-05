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

## Step 3 — TaskGraph fold + atomic claim + TeamRegistry (c41a22b)

- src/graph.rs: TaskGraph = fold over EventStore::stream() (never holds Vec<Event>; O(tasks) memory). Strict transition table derived from brief 8.2; duplicates (task/agent) and unknown deps fail; TaskClaimed fold is idempotent; LeaseHeartbeat/LeaseExpired refreshed/reset to Ready; illegal edges yield FoldError with expected vs observed state.
- src/team.rs: TeamRegistry folds TeamCreated/AgentRegistered/AgentReady/TaskClaimed — state stays Initializing until every member pinged ready, then Ready; first claim advances to Running. Rejects unknown members and pre-creation events.
- readiness: task must have all deps in Done plus no active lease; ready() sorted by TaskId (T4 determinism).
- claim(): NoSuchAgent, DepsNotMet, StatusNotClaimable, ActiveLease — emits TaskClaimed via EventLog so seq/durability are the log's job.
- Tests: tests/graph.rs 8 (incl. 50-task barrier-synchronized 2-thread claim stress asserting exactly one owner per task, plus from-log replay equality of full graph state after drop and reopen). tests/team.rs 4. 27/27 total, clippy -D warnings clean, fmt clean.

## Step 3.5 — centralized Coordinator + D16 (5113dea)

- Owner directed: no Redis (P1/D5 local-first stands; EventStore trait keeps a future backend swappable); no "capacity-only" framing — D16 final: late-joining CLI sessions are full members, immediately claim-eligible, and the boot ready-gate never re-engages; head re-decomposes pending work to absorb capacity (step 5).
- D17 recorded (hierarchical heads): single accountable head; beyond a measured threshold the head spawns sub-planners with scoped sub-graphs; head-to-subplanner communication is typed contracts only (no committee chat — F1/F2/F4); single integrator gate preserved.
- src/coordinator.rs: Coordinator::open replays log into fresh views; publish() = validate on cloned views THEN append THEN swap (rejected events never touch the log, so replays stay clean); claim() = clone graph, claim, append TaskClaimed, apply to both views, swap. CoreError wraps Log/Fold/Team/Claim.
- src/team.rs: AgentRegistered re-added as membership append (late joiner becomes member); state recompute still gated on Initializing only.
- tests/coordinator.rs: 4 (publish flows all three stores; late joiner claimable with no state regression; rejected publish leaves log+views consistent; reopen replays Running state). 31/31 total, clippy -D warnings, fmt clean.

## Step 4 — lease TTL + heartbeat extension + expiry sweep

- src/entities.rs: `Lease.ttl_ms` added (`#[serde(default)]` so pre-v0.1 logs decode as 0 — legacy leases just expire as originally scheduled).
- src/graph.rs: `LeaseHeartbeat` now extends `expires_at_ms = max(existing, at_ms + ttl_ms)` (was only updating `last_heartbeat_ms`, which meant heartbeats never kept leases alive). `graph::claim` stores the given `ttl_ms` on the lease. `LeaseExpired` fold additionally resets `InReview` → `Ready` (older code only handled Claimed/InProgress — ownerless InReview tasks could have advanced to Done with no owner).
- src/coordinator.rs: `check_for_expired_leases(now_ms)` filter-maps leased, non-terminal tasks past `expires_at_ms` and publishes `LeaseExpired` for each via the single-writer publish path; second sweep is a no-op; reopening the log after a sweep replays to the same state.

## Step 6 — OpenCode adapter (e12eb08)

- src/opencode.rs: OpenCodeAdapter implements AgentAdapter over opencode's local server API (ureq blocking client). start_session{ _cwd tolerated; server runs in spawn_local cwd } → POST /session {title, permission?}; send_turn → POST /session/:id/message with text parts; map_message_parts projects OpenCode message.parts directly onto SessionEvent (text → Ok, tool → ToolUse+ToolResult, step-finish → Usage, unknown → Ok with raw json); interrupt → POST /session/:id/abort; resume → GET /session/:id; inject_context → POST .../message noReply=true; Drop kills the child server it spawned.
- req timeouts calibrated per call (3s health, 10s resume/inject, 60s turn). Health check polls every 200ms for at most 20s during spawn.
- tests/opencode.rs: capabilities contract smoke test (always) + live-session test gated on `which opencode` returning 0 (spawn_local+start_session+send_turn+interrupt+liveness+inject). Live session took ~57s for a real-model turn, fully green on 2026-10-05 device.

## Planner/contract loop (f54b035)

- src/planner.rs: deterministic plan validation runs fully before any append (empty goal, duplicate/unknown tasks and contracts, dep cycles via DFS, overlapping write scopes with glob-prefix normalization among parallel tasks including plan-vs-active, shared-interface contract coverage). Publishes GoalSubmitted → TaskCreated (topological) → ContractProposed with causal seq chaining and a PlanReceipt.
- Contract exchange: propose/accept/change with version-staleness checks; DecisionRecorded passthrough.
- Assignment: next_assignment picks first ready task (deterministic TaskId order) matching agent capabilities; assign claims through Coordinator (team gate enforced); goal_progress counts done/failed with is_complete.
- TaskGraph folds contracts now; ContractStatus is Copy like the other small enums.
- tests/planner.rs: 6 tests. 59/59 total green, clippy -D warnings clean, fmt clean.
