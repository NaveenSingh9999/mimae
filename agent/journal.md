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

## Phase 4 — workspace manager

- src/workspace.rs: worktree lifecycle via git CLI, porcelain parsing that preserves status columns, path leases keyed by exact path, pairwise overlap prediction, integration gate that aborts conflicted merges before reporting paths and only fast-forwards main on clean merges + passing checks.
- tests/workspace.rs: 6 tests using temp repos with local-only git identity config. 68/68 total green, clippy -D warnings clean, fmt clean.

## Review pass (post-Phase 4)

- `IntegrationGate::run_check` took no timeout and inherited stdio: a hung check would block the gate forever and a chatty one could deadlock on a full pipe. Now `run_check(dir, program, args, timeout)` polls `try_wait`, kills on deadline, returns `CheckOutcome::{Passed, Failed, TimedOut}`, stdio nulled. Covered by a real `sleep 30` kill test.
- Verified no other unbounded subprocess waits: adapter turns go through `run_bounded_turn`, opencode HTTP calls carry per-call timeouts.
- md conformance re-checked: task lifecycle, message taxonomy, dispatch gate, D16 late-join, lease semantics all match; Artifact/Gate entities intentionally deferred to Phase 7.

## Phase 5 — messaging/context

- src/router.rs: scope/contract-aware routing (explicit recipients + affected owners), priority-ordered inboxes, delivery tiers per §9.3 (Interrupt for blocker/escalation and in-flight contract changes, Hook for contracts, NextTurn, NewTurn for idle, Digested batching for status_milestone), anti-jam (per-sender/kind rate limits, fingerprint dedupe with cap, broadcast rejection except human/host).
- src/context.rs: curated briefs (task + scope + goal acceptance + relevant contracts + decision digest + peer ownership, never peer histories), clean-context reviews, digest rendering, byte-budget test.
- TaskGraph folds GoalSubmitted/DecisionRecorded; Router owned by Coordinator, fed on every MessageSent publish and rebuilt identically on reopen (tested).
- tests/router.rs (9) + tests/context.rs (4). 82/82 total green, clippy -D warnings clean, fmt clean.

## Phase 7 — verification/security

- Artifact/Gate entities + three events folded with strict version/duplicate/double-decide rules; artifacts support status-only updates.
- Gate lifecycle helpers with reviewer loop cap; verification_status labels Verified/Unverified/Failed per task requirement.
- Deterministic injection-marker quarantine at router ingest (logged, never delivered); replay preserves quarantine.
- tests/verify.rs (6). 75/75 total green, clippy -D warnings clean, fmt clean.

## Phase 8 — evaluation harness

- src/eval.rs: metrics collected purely by replaying the event log; baselines as coordination regimes over identical scripted tasks; cost comparison table.
- tests/eval.rs: 12 scenarios. Notable verification: naive chat costs 40 messages vs 3 for Mimae on the same pair task; crash recovery loses zero tasks; milestone floods collapse to the 64-item digest cap.
- During verification, found real flakiness: parallel tests sharing one log file collide on the single-writer flock. Fixed by per-test log files plus explicit drop-before-reopen; also fixed piped-exit-code masking in my own check commands.

## Phase 9 — embedding

- src/daemon.rs: JSON-RPC dispatcher with -32601/-32602/-32700/-32000 codes, Arc<Mutex<Coordinator>> threaded serve loop, poll-based events_since subscription, no new dependencies.
- src/bin/mimae-daemon.rs: --port/--log arg parsing, replays log on open, prints protocol info to stderr.
- README documents library/daemon/bindings order, full method table, and host responsibilities.
- tests/daemon.rs (7). 101/101 total green, clippy -D warnings clean, fmt clean.

## Head drive loop

- src/drive.rs: deterministic head cycle using planner assignment + adapter turns; failed turns mark tasks Failed; goal loop exits on completion or round cap.
- tests/drive.rs (4). 105/105 total green, clippy -D warnings clean, fmt clean.

## Eval driven-goal follow-up

- 3 new scenarios in tests/eval.rs compose drive_goal with the metrics harness: full two-agent completion, stall (direct claim, no turn) → sweep → reclaim with reclaims=1/lease_expiries=1, and a discovery message whose seq latency is measured through completion.
- 114/114 total green, clippy -D warnings clean, fmt clean.

## Framework facade

- src/host.rs: Mimae facade owning Coordinator + per-agent adapters; with_hooks transition; poll_approvals answers pending Human gates via hook; drain_notifications surfaces Escalation/Blocker to hook; drain_events replays to hook; send_message added after test exposed the gap.
- README + D21. tests/host.rs (2). 116/116 total green, clippy -D warnings clean, fmt clean.

## Head auto-assignment loop

- src/drive.rs: drive_head wraps drive_agent_turn with token budgets (Usage events accumulated per agent, over-budget agents skipped), per-agent idle streaks with stuck_threshold flagging, early exit on consecutive fully-idle rounds, failed-task collection. TurnReport now carries tokens_used.
- Design fix during verification: breaking on the first idle round made the stuck threshold unreachable; consecutive-idle-round counting keeps both mechanisms coherent.
- tests/drive.rs (4 new). 120/120 total green, clippy -D warnings clean, fmt clean.

## MAST failure tagging (D22)

- New `src/failures.rs`: `FailureKind` (AgentCrash, AgentStuck, LeaseExpired, PromptInjection, DecisionConflict, DuplicateWork, TaskFailed, GateFailed, IntegrationFailed) mapped onto the three MAST categories; `tag_failures` is a read-only scan over any `EventStore`; `summarize` rolls tags into `CategoryCounts`.
- Detectors: Dead agent, Blocked task, LeaseExpired event, Failed task, failed-by-2+-owners (duplicate work), failed gate (+integration-failed for Tests/Oracle), injection markers via the router detector, contract change under active parties. Runaway cost classified but undetectable (no cost events) — documented in-module.
- `tests/failures.rs` (12). 133/133 total green, clippy -D warnings clean, fmt clean. Committed `da28184`.

## Gate promotion thresholds (D23)

- `PromotionGate` in `src/eval.rs`: thresholds over `ScenarioReport` + `FailureTag`s (success, tasks_failed, human_interventions, lease_expiries, reclaims, message cap, discovery-latency cap, per-MAST-category tag caps); `check` returns `Vec<Violation>`, empty means promoted.
- Every eval scenario declares its gate via `assert_gate` (re-opens log, tags, checks): strict for clean runs; explicit allowances where the scenario exercises the failure (heartbeat/crash/driven-stall 1+1+1, convoy 3+3+3, reviewer-cap 3 verification, injection 1 spec + 1-message cap, flood 100-message cap, latency 100-seq tripwire). Baselines: solo/naive strict, mimae must beat naive volume.
- `tests/promotion.rs` (8 unit tests). 141/141 total green, clippy -D warnings clean, fmt clean. Committed `8dc6587`.

## Phase 9 (D24)

- Daemon negotiation: `mimae.negotiate{protocol, client}` exact-match (v1), `-32001` mismatch, `-32002` transport gate on pre-negotiation calls (`hello` stays open); `handle_request` ungated for in-process use. 3 new daemon tests (accept/reject/socket-gate), README protocol table updated.
- `examples/sample_ade.rs`: minimal host over `Mimae` facade — team-first ordering (register-before-team fails `NotInitialized`), drive 2/2, human gate auto-approved via hooks, 20 events subscribed. Runs offline.
- `bindings/node`: napi-rs crate (workspace member), JSON-string `Coordinator` surface (open/close/publish/claim/plan_goal/assign/goal_progress/events_since/sweep_expired); u64 crosses as BigInt (napi-rs refuses lossy Number), needs `napi6` feature; poisoned registry mutex recovers. `test.js` drives a full loop from JS: 20 events, goal 2/2, error paths verified.
- 144/144 green, clippy -D warnings + fmt clean workspace-wide. Committed `1f34ff3`.

## Runaway-cost kill-switch (D25)

- `CostRecorded{agent_id, task_id?, input_tokens, output_tokens}` event; fold accumulates `cost_by_agent`/`cost_by_task` (+ `Agent.budget_used_tokens`), lenient on unknown agents; spend verified durable across reopen.
- `CostPolicy{per_turn/per_task/per_agent_tokens_max}` (`unlimited()` default): pre-turn agent skip (no claim, `budget_breached`), pre-turn task fail without burning a turn, post-turn fail on any breach — always `Failed` with reason `budget-exceeded`, spend recorded first. Threaded through `drive_goal`, `drive_head` (`HeadConfig.cost_policy`), host `drive_with_policy` (`drive` unchanged). Soft `token_budget_per_agent` skip kept (different action).
- `FailureKind::RunawayCost` (Specification) tags exactly the kill-switch reason. New eval scenario `driven-budget` with gate (1 failed / 1 spec / 1 verification). Mid-turn token kill stays timeout-based (D19) — documented.
- 152/152 green (8 new), clippy -D warnings + fmt clean, node conformance + sample re-verified. Committed `e0d6166`.

## Usage streaming + mid-turn watchdog (D26)

- `UsageSampled{agent, task?, elapsed_ms, tokens_so_far}` events: the ADE usage stream, readable via `events_since` polling (daemon/binding/hosts alike).
- `TurnControl: Send + Sync` + `AgentAdapter::control_handle()` (default `NoControl`): out-of-band sample/interrupt sharing state with the running turn through atomics. `AgentAdapter` gained a `Send` supertrait (all impls already Send).
- `CostPolicy` gains `sample_every_ms` + `turn_wall_max_ms`: monitored turns run on a scoped worker thread (joined before return, `run_bounded_turn` discipline) with interrupt on sampled per-turn breach (`budget-exceeded`) or wall expiry (`turn-timeout`, plain failure). Unlimited policy stays single-threaded, zero new events.
- Key bug caught by tests: Mutex-around-adapter deadlocks the ticker for the whole turn — fixed via the split control plane before committing.
- Real CLI adapters keep `NoControl` (D19 timeouts still bound them); ADE-owned adapters implement real handles. 3 watchdog tests, ran 3x stable.
- 155/155 green, clippy -D warnings + fmt clean, node + sample re-verified (sample now streams 22 events: 20 + 2 CostRecorded). Committed `d206be2`.
