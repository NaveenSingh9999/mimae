<h1 align="center">Mimae</h1>

<p align="center">
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License">
  <img src="https://img.shields.io/badge/rust-1.85%2B-orange" alt="Rust version">
  <img src="https://img.shields.io/badge/status-alpha-yellow" alt="Status">
</p>

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

---

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

---

## Feature tour

| Area | What lives there |
|---|---|
| [`src/event.rs`](src/event.rs) | 21 typed `KnownPayload` variants; unknown payloads round-trip verbatim |
| [`src/log.rs`](src/log.rs) | Append-only JSONL, fsync per write, torn-tail recovery, stream reads |
| [`src/graph.rs`](src/graph.rs) | Strict fold, dependency-gated readiness, atomic claims, cost-by-agent projection |
| [`src/coordinator.rs`](src/coordinator.rs) | `publish`/`claim` with validate-then-commit; rebuild on fold error |
| [`src/team.rs`](src/team.rs) | One lifecycle chain: `Initializing → Ready → Running` |
| [`src/planner.rs`](src/planner.rs) | Scope-overlap and cycle validation before publish; capability assignment; goal progress |
| [`src/drive.rs`](src/drive.rs) | `drive_agent_turn`/`drive_goal`/`drive_head`; `CostPolicy`; scoped watchdog |
| [`src/host.rs`](src/host.rs) | `Mimae` facade + `HostHooks` for approvals, escalations, event subscription |
| [`src/adapter.rs`](src/adapter.rs) | Normalized `SessionEvent`s; bounded turn execution that kills hung children |
| [`src/workspace.rs`](src/workspace.rs) | Worktree lifecycle, path leases, overlap prediction, integration gate |
| [`src/router.rs`](src/router.rs) | Typed boxes, tiers, anti-jam, dedupe, digests, injection quarantine |
| [`src/context.rs`](src/context.rs) | Curated role briefs with token budgets; cross-role reasoning stays out |
| [`src/verify.rs`](src/verify.rs) | Gates, reviewer-loop cap of three, artifact recording, verification labels |
| [`src/eval.rs`](src/eval.rs) | Log-driven metrics, Solo/Mimae/NaiveChat baselines, `PromotionGate` thresholds |
| [`src/failures.rs`](src/failures.rs) | `FailureKind → MastCategory` tagging; anchored to log sequence |
| [`src/daemon.rs`](src/daemon.rs) | Newline JSON-RPC 2.0; `hello`/`negotiate`; 16 state methods |
| [`bindings/node`](bindings/node) | napi-rs binding: open/publish/claim/plan/assign/goal-progress/events-since/sweep/protocol-version |

---

## Where to find the docs

| Document | Purpose |
|---|---|
| [`mimae-concept.md`](mimae-concept.md) | Full concept brief (requirements, adapters, risks, roadmap) |
| [`DECISIONS.md`](DECISIONS.md) | Every decision D1–D27, with dates and justification |
| [`BUILD_PROMPT.md`](BUILD_PROMPT.md) | Self-contained spec you can paste into a coding AI to build this |
| [`SETUP.md`](SETUP.md) | Manual setup: prerequisites, tests, examples, daemon, Node build, embedding picks |
| `README.md` | This file |

---

## Building and running

The full walkthrough lives in [`SETUP.md`](SETUP.md). Day-to-day commands:

```sh
cargo test --workspace                  # full verification
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo run --example sample_ade          # offline team + drive + gates demo
cargo run --example e2e_opencode        # live turn when `opencode` is on PATH
cargo run --bin mimae-daemon -- --port 7777 --log /tmp/run.log
```

---

## Interfaces at a glance

*Rust embed:* depend on `mimae-core`, open a log path, register agents
with their adapters, create the team first, mark ready, submit a goal,
drive, and let hooks deliver approvals/events out.

*Daemon:* same logic over loopback JSON-RPC 2.0. One TCP line is one
request; replies carry matching `id`. `hello`, then `negotiate` before any
state-changing call — any other call there first gets `-32002`.

*Node:* same surface exported from [`bindings/node`](bindings/node); load
as `mimae_node.node`, verify via `npm test` in that directory.

---

## Constraints and current gaps

> [!NOTE]
> - Windows single-writer lock is a documented stub today; plan for
>   exactly one coordinator process per log file on Windows until
>   LockFileEx-based locking is implemented.
> - Daemon is loopback TCP and has no auth layer by design.
> - OpenCode is the only live-verified CLI adapter at the moment; Claude,
>   Antigravity, Qwen, and Mimo pass offline conformance/fixtures.
> - Mid-turn token kill requires an adapter-provided
>   `TurnControl::sample_usage`; built-in adapters run timeout-bounded
>   turns.

---

## License

**License:** MIT OR Apache-2.0
