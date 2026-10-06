# AI Build Prompt — Mimae Coordination Layer

> Paste this whole document into your AI. It is a self-contained build spec.
> The timestamps/decision IDs (D1–D27) are reference labels; do not invent
> new ones — you are building a fresh implementation, and this document is
> the source of truth.

---

## What you are building

**Mimae** is a Rust backend coordination layer that lets multiple agentic
CLI tutoring/coding CLIs (OpenCode, Claude Code, Codex CLI, Antigravity CLI,
Qwen Code, MimoCode) self-organize on one project: they claim tasks, hold
leases, pass changes through contracts, run automated verification gates,
and hand work back to a head agent — with no human router and no MCP.
It is a **library first**: ADEs embed it directly; non-Rust hosts use a
local loopback daemon (newline JSON-RPC 2.0); a first language binding
(Node, napi-rs) covers direct embedding from JS.

Everything inside Mimae is **deterministic and non-LLM**: scheduling,
leasing, routing, budgets, and conflict prediction never call a model.
LLMs only *produce* work at the edges (planning/judging).

Repository layout (single crate `mimae-core`, edition 2024, rust-version 1.85):

```
src/
  lib.rs        crate root, pub re-exports, #![deny(unsafe_code)]
  ids.rs        GoalId/TaskId/AgentId/... (u64-backed, transparent serde)
  actor.rs      Actor enum (Coordinator/Host/Human/System/Agent)
  entities.rs   Goal/Budget, Task, Agent, Lease, Contract, Decision, Message,
                Team, Artifact, Gate, Verification, WriteScope, Capability, CliKind
  event.rs      Event {schema_version, seq, actor, payload, causes, at_ms},
                KnownPayload variants, EventPayload (Known|Unknown), PROTOCOL_VERSION
  log.rs        EventLog (append-only JSONL), EventStore trait, EventStream, torn-tail recovery
  graph.rs      TaskGraph fold (strict transition table), TaskGraph::claim, readiness
  team.rs       TeamRegistry lifecycle (initializing/ready/running)
  coordinator.rs Coordinator: the single ingest point (log + graph + team registry)
  planner.rs    plan_goal (validate-then-publish), assign, next_assignment, goal_progress
  drive.rs      drive_agent_turn, drive_goal, drive_head, CostPolicy, run_monitored_turn
  host.rs       Mimae facade + HostHooks (NoHooks default)
  adapter.rs    AgentAdapter trait, SessionEvent, run_bounded_turn, TurnControl/NoControl
  opencode.rs / claude.rs / antigravity.rs / qwen.rs / mimo.rs
  workspace.rs  WorkspaceManager, PathLeaseRegistry, IntegrationGate
  router.rs     Message router (tiers, anti-jam, digests, quarantine)
  context.rs    Curated briefs, review coherence check
  verify.rs     Gates, reviewer loop cap, artifact recording, verify_artifact
  daemon.rs     handle_request, dispatch, serve, handle_connection
  eval.rs       ScenarioMetrics, collect_metrics, compare_message_cost, PromotionGate
  failures.rs   FailureKind -> MastCategory, tag_failures, summarize
  bin/
    mimae-daemon.rs
tests/
  vocab.rs log.rs graph.rs coordinator.rs router.rs context.rs verify.rs
  eval.rs drive.rs host.rs daemon.rs workspace.rs failures.rs promotion.rs adapter.rs ...
bindings/
  node/   napi-rs workspace member, JSON-string surface over Coordinator
examples/
  e2e_opencode.rs  sample_ade.rs
```

---

## Rules of the road

1. **EventLog is the single source of truth.** All state mutations go
   through `Coordinator::publish` / `Coordinator::claim`, which do
   *validate-then-commit*: dry-run against cloned views, append to the log,
   swap. Rejected events never hit the log, so replays stay clean.
   `TaskGraph::apply` is the strict fold that derives views from the log;
   from-log replay must equal in-memory state (test this).
2. **No clock reads inside the library.** Every function takes `at_ms` /
   `now_ms` from the caller. Tests and runs are deterministic.
3. **No `unwrap`/`expect`/`panic!` in `src/`.** Use `thiserror` errors.
   Error paths: validation failures, fold rejections, unknown ids.
4. **No detached threads.** Long-running work (turn execution, watchdog)
   uses scoped threads joined before return (`std::thread::scope`,
   same discipline as `run_bounded_turn`). The daemon spawns per-connection
   accept threads that end on client EOF; that is the only detached thread,
   by design.
5. **One lock wrapper.** `#![deny(unsafe_code)]` crate-wide; the single
   `#[allow(unsafe_code)]` covers the flock wrapper on unix. On Windows the
   single-writer lock is currently a documented stub (see D27 note below).
6. **Events out, never UI.** Mimae renders nothing; hosts answer approval
   gates, escalations, and subscriptions.
7. **Forward-compatible wire format.** `EventPayload::Unknown` round-trips
   unknown payloads verbatim; coordinating binaries tolerate new event
   kinds. New event types must be additive.
8. **Deterministic scheduling.** `ready()` sorts deterministically;
   round-robin drive loops; no wall-clock-dependent ordering.

---

## Core entities (entities.rs)

- `Goal { id, text, acceptance, constraints, budget }`; `Budget { max_tokens, deadline_ms }`
- `Task { id, goal_id, title, brief, capability, write_scope, deps, status, owner, lease, verification }`
- `Agent { id, cli, capabilities, role, state, budget_used_tokens }`
- `Lease { owner, scope, expires_at_ms, last_heartbeat_ms, ttl_ms }`
- `Contract { id, parties: Vec<TaskId>, content, version, status }`
- `Message { id, kind, sender: Actor, recipients, refs, priority, provenance, body }` (body cap 2048)
- `Gate { id, task_id, kind: Tests|Oracle|Reviewer|Human, status }`
- `Team { id, head, mode, state: Initializing|Ready|Running, members }`
- `Verification::None | Tests | Oracle | Reviewer | Human`
- `CliKind::OpenCode | ClaudeCode | Codex | Antigravity | QwenCode | MimoCode | Gemini | Other(String)`

## Task lifecycle (graph.rs, strict 8.2 table)

`Proposed → Ready → Claimed → InProgress → InReview → Integrating → Done`,
plus failure edges: `→ Blocked` (→ Ready | Cancelled), `→ Failed`
(→ Cancelled), `→ Abandoned` (→ Cancelled), `→ Cancelled` from Proposed /
Ready / Claimed / InProgress / InReview / Integrating. `LeaseExpired`
resets a task (Claimed/InProgress/InReview) back to Ready. A task is
claimable only when its dependencies are all Done (`TaskGraph::ready`).
`Coordinator::claim` is atomic and returns `ClaimError` on any violation.
Claims hold a `Lease` over a `WriteScope`; heartbeats extend
`expires_at_ms` by `ttl_ms`.

## Team lifecycle (team.rs)

`TeamCreated` → `Initializing` → all members `AgentReady` → `Ready` →
head transitions it to `Running`. `NotDispatchable` error if assignment is
attempted before `Ready`. Late joiners are full members; the ready-gate
never re-engages (D16).

## Events (event.rs)

`PROTOCOL_VERSION = 1`. Known payloads (all serde-tagged `type`):
`GoalSubmitted, TaskCreated, TaskStatusChanged, TaskClaimed, LeaseHeartbeat,
LeaseExpired, AgentRegistered, AgentStateChanged, ContractProposed,
ContractAccepted, ContractChanged, DecisionRecorded, MessageSent,
MessageDelivered, TeamCreated, AgentReady, ArtifactRecorded, GateOpened,
GateDecided, CostRecorded, UsageSampled`.

Every event: `schema_version`, monotonic `seq`, `actor`, `payload`,
`causes: Vec<seq>`, `at_ms` (caller clock).

## Coordinator (coordinator.rs)

- `open(path, EventLogOptions) -> (Coordinator, OpenInfo)`: opens the log,
  flocks it, recovers torn tail, replays to build views.
- `publish(actor, payload, causes, at_ms) -> Result<Event, CoreError>`
- `claim(task_id, agent_id, scope, now_ms, ttl_ms) -> Result<Lease, ClaimError>`
- `check_for_expired_leases(now_ms) -> Result<Vec<(TaskId, AgentId)>, CoreError>`
  (query-time sweep: publishes `LeaseExpired` for expired leases; also
  resets `InReview` leases)
- `events_since(seq)`, `graph()`, `team()`, `router_mut()`, `router()`
- Tunable: every view is a pure function of the log — any new fold must be
  replay-safe.

## Planner (planner.rs)

`plan_goal(coord, goal, tasks, contracts, at_ms) -> PlanReceipt`:
- Validates: non-empty goal, duplicate/unknown tasks, dependency cycles,
  overlapping write scopes among parallel tasks (plan-vs-plan and
  plan-vs-active), shared-interface contract coverage.
- On success publishes `GoalSubmitted`、`TaskCreated`、`ContractProposed`
  with causal chaining. Failure ⇒ zero log side effects.
- `assign(coord, agent_id, now_ms, ttl_ms)` — capability match + claim.
- `next_assignment(coord, agent_id) -> Option<TaskId>`
- `goal_progress(coord, goal_id) -> {total, done, failed}` (+ `is_complete`)
- `propose_contract/accept/change_contract`, `record_decision`

## Drive loops (drive.rs)

- `drive_agent_turn(coord, agent_id, adapter, now_ms, ttl_ms, policy)
  -> TurnReport`: sweep → capability assign → scripted/real turn →
  Ok ⇒ Claimed→InProgress→InReview→Integrating→Done, Fail ⇒ Failed.
  Spend is recorded via `CostRecorded` before any failure decision.
- `drive_goal(coord, agents, goal_id, now_ms, ttl_ms, max_rounds, policy)`
- `drive_head(...)`: `HeadConfig { max_rounds, stuck_threshold,
  token_budget_per_agent (soft per-run skip), cost_policy }`;
  rounds round-robin until complete/idle-threshold; tracks idle streaks
  (stalled), over-budget agents, failed tasks.

## Cost kill-switch (drive.rs + failures.rs)

- Every successful-or-not turn appends `CostRecorded{agent_id, task_id?,
  input_tokens, output_tokens}`; `TaskGraph` folds it into
  `cost_by_agent`, `cost_by_task` (+ `Agent.budget_used_tokens`).
  Spend is **durable** — caps survive restarts.
- `CostPolicy { per_turn_tokens_max?, per_task_tokens_max?,
  per_agent_tokens_max?, per_goal_tokens_max?, sample_every_ms?,
  turn_wall_max_ms? }` — `unlimited()` default. Pre-turn: over-budget
  agent claims nothing (report.budget_breached=true); over-budget task or
  goal fails without burning another turn. Post-turn: any cap breach fails
  the task with reason `budget-exceeded` (eval tags this as
  `RunawayCost → Specification`).
- `CostPolicy.per_goal_tokens_max` falls back to the goal's own
  `Budget::max_tokens` when non-zero (`goal_spend`).
- **Watchdog**: when `sample_every_ms` is set, the turn runs on a scoped
  worker thread while the loop samples `AgentAdapter::control_handle()
  .sample_usage()` every tick, publishes `UsageSampled{...}` per tick
  (the streaming usage API), and calls `interrupt()` on per-turn breach.
  `turn_wall_max_ms` interrupts and fails with `turn-timeout`. Without
  monitoring (or with `NoControl`), turns are single-threaded and the cap
  is post-turn backstop only. Timeouts always come from
  `run_bounded_turn`/process kill (D19).

## Adapter contract (adapter.rs)

```rust
pub trait AgentAdapter: Send {
    fn capabilities(&self) -> AdapterCapabilities;
    fn start_session(&mut self, cwd, brief, permission_profile) -> SessionReply;
    fn send_turn(&mut self, prompt) -> Vec<SessionEvent>;
    fn interrupt(&mut self) -> bool;
    fn resume(&mut self, session_id) -> SessionReply;
    fn inject_context(&mut self, context) -> bool;
    fn liveness(&self) -> AgentState;
    fn timeout(&self) -> Option<Duration> { None }
    fn control_handle(&self) -> Box<dyn TurnControl> { Box::new(NoControl) }
}
pub trait TurnControl: Send + Sync {
    fn sample_usage(&self) -> Option<u64> { None }
    fn interrupt(&self) -> bool { false }
}
```

`SessionEvent` variants: `Ok|Error|ToolUse|ToolResult|Usage|PermissionRequest|TurnComplete|StateChanged`.
Each CLI adapter spawns the real binary (`opencode serve` / `claude -p` /
`agy -p` / `qwen` / `mimo run`), enforces the documented timeout, parses
stream-json, and exposes interrupt/resume. Only OpenCode is live-verified;
the others are fixture-tested — mark their tests `live` and skip by
default.

## Messaging & context (router.rs, context.rs)

- Closed `MessageKind` taxonomy (ContractProposal, ContractAccept,
  ContractChange, Blocker, Discovery, InfoRequest, InfoResponse,
  StatusMilestone, Escalation) with delivery tiers `Interrupt|Hook|NextTurn|NewTurn|Digested`,
  anti-jam (rate limits, dedupe, digest batching, broadcast rules).
- `detect_injection_markers` flags instruction-override / system-disclosure /
  role-reassignment / privilege-escalation; quarantined before routing.
- Context broker builds curated per-role briefs (no raw peer history).

## Workspace & integration (workspace.rs)

- `WorkspaceManager`: one git worktree per agent task, return-to-base diffs,
  dirty-path reporting.
- `PathLeaseRegistry`: acquire/release on write scopes.
- `predict_overlaps`: pairwise write-scope conflict check.
- `IntegrationGate`: prepare merge branch, merge-branch with conflict paths,
  run_check (bounded: `Passed|Failed|TimedOut`), advance on green.

## Verification & security (verify.rs)

- `open_gate/decide_gate/record_artifact/verify_artifact`; reviewer loop
  cap 3; labeling `Verified|Unverified|Failed`.
- Injected-instruction messages are quarantined and auditable.
- Human gates flow through `HostHooks::on_approval`.

## Eval harness (eval.rs + failures.rs)

- `collect_metrics(log_path, wall_ms)`: tasks done/failed, message counts
  by kind, lease expiries, reclaims, discovery-to-action latency, human
  interventions, wall time.
- `Baseline { Solo, Mimae, NaiveChat }`; `compare_message_cost` table.
- `PromotionGate { require_success, max_tasks_failed, max_human_interventions,
  max_lease_expiries, max_reclaims, max_messages?, max_discovery_latency_seqs?,
  max_specification_tags?, max_misalignment_tags?, max_verification_tags? }`;
  `check(report, tags) -> Vec<Violation>` (empty = promoted). Every
  scenario declares its gate; recovery scenarios explicitly allow the
  churn they exercise — allowances are deliberate constants, never silent.
- `FailureKind -> MastCategory` mapping: Specification (crashes, leases,
  trust boundary, runaway cost), Misalignment (stuck, conflicts, duplicate
  work), Verification (task/gate failures). `tag_failures(store)` is a
  read-only deterministic scan. Detectors: Dead agent, Blocked,
  LeaseExpired, Failed, failed-by-2+-owners (duplicate work), failed gate
  (+integration for Tests/Oracle), injection markers, contract-change
  under active parties, `budget-exceeded` failures.

## Daemon (daemon.rs)

- Newline-delimited JSON-RPC 2.0 on 127.0.0.1.
- `mimae.hello` -> `{protocol, version}` (open before negotiation).
- `mimae.negotiate{protocol, client}` -> exact-match v1; mismatched ⇒
  `-32001`. Transport gate: any non-hello/non-negotiate call before a
  successful negotiate gets `-32002`. In-process `handle_request` stays
  ungated (embedders own version discipline).
- Methods: publish, claim, ready_tasks, task, team_state, sweep_expired,
  events_since, plan_goal, assign, next_assignment, goal_progress,
  open_gate, decide_gate, verification_status, record_artifact,
  propose_contract.
- Errors: `-32601` unknown, `-32602` bad params, `-32700` malformed line,
  `-32001` protocol mismatch, `-32002` pre-negotiation call, `-32000`
  coordinator rejection (never mutates the log on rejection).
- `Coordinator` is per-log-path singleton; no global state.

## Host embedding (host.rs)

```rust
let mut host = Mimae::open("run.log")?.with_hooks(MyHooks);
host.register_agent(agent, adapter, at)?;       // one adapter per CLI session
host.create_team(team, at)?;                    // team before agents
host.mark_ready(agent_id, at)?;
host.submit_goal(goal, tasks, contracts, at)?;
host.drive(goal_id, now, ttl, max_rounds)?;     // unlimited-cost
host.drive_with_policy(goal_id, now, ttl, max_rounds, &policy)?;
host.open_gate(gate, at)?; host.poll_approvals(at)?;
host.send_message(message, at)?;
host.drain_notifications(); host.drain_events(since_seq)?;
host.coordinator() // read-only view
```

Node binding mirrors the same Coordinator surface over JSON strings
(`bindings/node`, registered workspace member); 64-bit ids cross as
`BigInt` (napi-rs refuses lossy Number conversion; out-of-range ⇒ loud
error). Python binding is next — implement the daemon client in Python and
skip native rebuilds.

## Run it

```sh
cargo test --workspace           # full suite + conformance
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo run --example sample_ade   # offline sample host (EchoAdapter)
cargo run --bin mimae-daemon -- --port 7777 --log /tmp/run.log
```

## Acceptance criteria (from the brief, §14)

- Phase 0: decisions recorded; license MIT OR Apache-2.0.
- Phase 1: one CLI (OpenCode) driven over a structured channel with live
  conformance (start/prompt/stream/interrupt/resume/crash recovery).
- Phase 2: same conformance suite passing on each of 3–4 CLIs + capability
  degradation tested. **(Open — needs those binaries present.)**
- Phase 3: kill-and-resume with zero lost tasks; atomic claim stress.
- Phase 4: overlapping parallel agents never corrupt each other;
  integration gate blocks a deliberately broken merge.
- Phase 5: discovery latency measurable through the log; message volume
  caps gate CI.
- Phase 6: a goal runs end-to-end with zero human assignment.
- Phase 7: reviewer loop works; injected instruction quarantined.
- Phase 8: CooperBench integration; results published vs Solo / Mimae /
  NaiveChat baselines. **(Open.)**
- Phase 9: stable library API, versioned protocol with negotiation, local
  daemon, first language binding (Node), docs + examples. **(Done for
  Node; Python next.)**

## Deliberate divergences / open debt (documented, do not "fix" silently)

- Phase 2 live conformance and Phase 8 CooperBench are blocked on
  binaries/benchmarks not available in the dev environment.
- Windows single-writer lock is a documented stub (`log.rs` TODO); the
  running assumption is one process per log file.
- §9.8 suggests "pause and escalate" for runaway cost; the core fails
  closed (`budget-exceeded`) — escalation is host policy, not core.
- Mid-turn token kill requires `TurnControl` handles; the five built-in
  CLI adapters run timeout-bounded single-threaded turns meanwhile.
- Daemon is loopback-only, unauthenticated.

## Decision index (reference labels)

D1 one crate; D2b single `mimae-core`; D3 learn-from gate4agent;
D4 MIT OR Apache-2.0; D5 local-first; D15 head-driven startup;
D16 late joiners full members; D17 hierarchical heads (typed contracts);
D18 depend-on vs learn (learn); D19 turn timeouts; D20 embedding order;
D21 framework-first embedding; D22 MAST tagging; D23 promotion gates;
D24 negotiation/sample/Node; D25 runaway kill-switch; D26 usage streaming
+watchdog; D27 review fixes.
