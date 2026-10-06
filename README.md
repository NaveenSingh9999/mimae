# Mimae

> [!TIP]
> **Just want it running?** Use the [AI build prompt](BUILD_PROMPT.md) —
> paste it into a coding assistant and it has the full spec, invariants,
> protocol, and acceptance criteria. Prefer to do it by hand?
> [`SETUP.md`](SETUP.md) walks through prerequisites, tests, examples,
> the daemon, the Node binding, and each embedding option step by step.

Mimae dispenses with the middleman between projects and the coding
agents working on them. You point it at a goal and a repo; it decomposes
the goal, assigns tasks, brokers leases and contracts, runs verification
gates, and reports progress. When something fails, the failure is
classified, tagged, and counted — never silently dropped.

Design centre: deterministic Rust core, thin async edges. The core owns
scheduling, leasing, routing, budgets, and verification. Agent CLIs only
execute; they never coordinate with each other directly. There is no MCP,
no UI inside Mimae, and no model call anywhere in the core loop.

## Highlights

- **Single source of truth.** Every mutation is appended to one JSONL
  event log. In-memory views are replays of that log; reopening a path
  reproduces identical state.
- **Safe claims.** Lease-based single ownership per task, releasing on
  heartbeat expiry. A parallel stress test claims 50 tasks from two
  threads with zero double-claims.
- **Deterministic planning.** Goals go in; a task graph with
  dependency/ordering validation and write-scope overlap checks comes out.
  No overlap, cycle, or missing contract ⇒ validation error before the
  event is logged.
- **Structured messaging.** Typed messages with delivery tiers (interrupt,
  hook, next/new turn, digest), anti-jam rate limits, and prompt-injection
  quarantine before routing.
- **Verification gates.** Per-task tests, oracle, reviewer, and human
  gates; reviewer loops are capped; artifacts are recorded with
  provenance.
- **Durable cost accounting.** Token spend is written to the log, so caps
  hold across restarts. Hard per-turn/per-task/per-agent/per-goal caps
  fail closed with `budget-exceeded`. `UsageSampled` events stream live
  usage to ADE subscribers; a scoped watchdog can interrupt a turn that
  blows the watchdog window.
- **Eval harness built in.** Scenarios replay the log into metrics,
  failure tagging maps failures to MAST categories, and promotion gates
  turn metrics into pass/fail thresholds.

## Feature tour

| Area | How to use it |
|---|---|
| Event vocabulary | `Event` + 21 `KnownPayload` variants in `src/event.rs`; unknown types round-trip verbatim for forward compatibility |
| Log | `EventLog::open` / `EventStore` trait in `src/log.rs`; append-only, fsync, torn-tail recovery, stream-replaying reader |
| Task graph | `TaskGraph` fold in `src/graph.rs`; strict transition table, dependency-gated readiness, atomic `claim` |
| Single writer | `Coordinator::publish` / `claim` with validate-then-commit; on fold/team/route error the views rebuild from the log |
| Teams | `TeamRegistry` in `src/team.rs`; one transition chain `Initializing → Ready → Running` |
| Planning | `planner` validates scope overlaps, cycles, duplicate ids, missing contracts before publishing; assigns by capability match; tracks completion |
| Turn driving | `drive_agent_turn` / `drive_goal` / `drive_head` in `src/drive.rs` over `AgentAdapter`; soft per-run token budget; hard cost policy with `budget-exceeded` / `turn-timeout` reasons |
| Host integration | `Mimae` facade in `src/host.rs` (`open`/`register_agent`/`create_team`/`mark_ready`/`submit_goal`/`drive`/`drive_with_policy`/`open_gate`/`record_artifact`/`propose_contract`/`record_decision`/`send_message`/`poll_approvals`/`drain_notifications`/`drain_events`) + `HostHooks` |
| Agent adapters | `AgentAdapter` in `src/adapter.rs` + five CLI impls; normalized `SessionEvent`s; bounded turn execution that kills hung children |
| Workspace | `WorkspaceManager` + `PathLeaseRegistry` + `IntegrationGate` in `src/workspace.rs`; one worktree per task, conflict prediction, bounded gate check |
| Messaging | `Router` in `src/router.rs`: typed boxes per agent, anti-jam caps, dedupe, digests, suspension after three rapid notifications; injection detector quarantines strings with instruction-override markers |
| Human context | `src/context.rs`: curated role briefs with token budgets and cross-role reasoning hidden |
| Verification | `src/verify.rs`; reviewer-loop cap of three; artifacts record file paths and producing task; status upgrade propagates through `verification_status` |
| Evaluation | `src/eval.rs` metrics over events; `Baseline` comparison across Solo/Mimae/NaiveChat; `PromotionGate` thresholds per MAST tag category |
| Failure tagging | `src/failures.rs`: `FailureKind` → `MastCategory`; each detected rule anchored to log sequence |
| Daemon | `src/daemon.rs`: newline JSON-RPC 2.0; `hello` + enforced `negotiate`; 16 state methods |
| Node binding | `bindings/node`: ping/open/close/publish/claim/plan/assign/goal-progress/events-since/sweep-expired/protocol-version; `BigInt` wire types |

## Where to find the docs

| Document | Purpose |
|---|---|
| `mimae-concept.md` | Full concept brief (requirements, adapters, risks, roadmap) |
| `DECISIONS.md` | Every decision D1–D27, with dates and justification |
| `BUILD_PROMPT.md` | Self-contained spec you can paste into a coding AI to build this |
| `SETUP.md` | Manual setup: prerequisites, tests, examples, daemon, Node build, embedding picks |
| `README.md` | This file |

## Building and running

The short version lives in `SETUP.md`. Day-to-day commands:

- `cargo test --workspace` — full verification
- `cargo clippy --workspace --all-targets -- -D warnings` — lint gate
- `cargo fmt --check` — formatting gate
- `cargo run --example sample_ade` — offline walkthrough of team + drive + gates + events
- `cargo run --example e2e_opencode` — live turn against OpenCode when `opencode` is on PATH
- `cargo run --bin mimae-daemon -- --port 7777 --log /tmp/run.log` — serve the JSON-RPC surface

## Interfaces at a glance

**Rust embed:** depend on `mimae-core`, open a log path, register agents
with their adapters, create the team first, mark ready, submit a goal,
drive, and let hooks deliver approvals/events out.

**Daemon:** same logic over loopback JSON-RPC 2.0. One TCP line is one
request; replies carry matching `id`. `hello` then `negotiate` before any
state-changing call, `-32002` otherwise.

**Node:** same surface is exported from `bindings/node`; loads as
`mimae_node.node`, test via `npm test` in that directory.

## Notes and constraints

- Windows single-writer lock is a documented stub today; plan for exactly
  one coordinator process per log file on Windows until LockFileEx-based
  locking is implemented.
- Daemon is loopback TCP and has no auth layer by design.
- OpenCode is the only live-verified CLI adapter at the moment; Claude,
  Antigravity, Qwen, and Mimo pass offline conformance/fixtures.
- Mid-turn token kill requires an adapter that supplies
  `TurnControl::sample_usage`; built-in adapters run timeout-bounded
  turns.

## License

**License:** MIT OR Apache-2.0
