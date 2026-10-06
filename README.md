# Mimae

Mimae is a Rust library that lets several agent coding CLIs (OpenCode,
Claude Code, Codex, Antigravity, Qwen, Mimo) work on one repo as a team.
They claim tasks, hold leases, negotiate through contracts, run checks,
and report back to a head agent. No human router in the middle, no MCP.

## What you get

- Deterministic core: scheduling, leases, routing, and cost limits are
  pure logic. Nothing here calls a model.
- An append-only event log as the one source of truth. Reopen the same
  file and it replays.
- A Neumann-style coordination loop: goal in, task tree out, agents
  assigned by capability, drive loop runs turns, gates verify, head
  decides what is next.
- Three ways to integrate:
  1. **Rust crate** — `mimae-core`, embedded directly.
  2. **Local daemon** — `mimae-daemon`, loopback JSON-RPC for
     Electron/Tauri/editor plugins/scripts.
  3. **Node binding** — `bindings/node` (napi-rs).

## Quick start

```sh
cargo test --workspace                  # runs the whole suite
cargo run --example sample_ade          # offline walkthrough, no CLIs needed
cargo run --example e2e_opencode        # live run against a local opencode
cargo run --bin mimae-daemon -- --port 7777 --log /tmp/run.log
```

## Building your own integration

Pick one of the two paths that fits your workflow:

- **AI-driven build** — paste `BUILD_PROMPT.md` into your coding AI.
  It has the whole architecture, protocol, invariants, and acceptance
  criteria.
- **Manual build** — follow `SETUP.md` step by step.

Both land at the same place: a working crate, a passing test suite, a
daemon you can talk to, and a Node binding if you need JS embedding.

## Minimal Rust embedding

```rust
let mut host = Mimae::open("run.log")?.with_hooks(MyHooks);
host.register_agent(agent, adapter, at)?;  // one adapter per session
host.create_team(team, at)?;               // team comes first
host.mark_ready(agent_id, at)?;
host.submit_goal(goal, tasks, contracts, at)?;
let report = host.drive(goal_id, now, ttl, max_rounds)?;
let events    = host.drain_events(since_seq)?;
let approvals = host.poll_approvals(at)?;
let notes     = host.drain_notifications();
```

## Daemon protocol

`mimae.hello` → `{protocol, version}` → `mimae.negotiate{protocol,client}`
(mismatch ⇒ `-32001`; any other call before negotiate ⇒ `-32002`), then
any of:

| Method | Key params | Result |
|---|---|---|
| `mimae.publish` | `actor`, `payload`, `causes`, `at_ms` | appended `Event` |
| `mimae.claim` | `task_id`, `agent_id`, `scope`, `now_ms`, `ttl_ms` | `Lease` |
| `mimae.ready_tasks` | — | `[task_id]` sorted |
| `mimae.task` | `id` | `Task` or null |
| `mimae.team_state` | — | state or null |
| `mimae.sweep_expired` | `now_ms` | `[[task_id, agent_id]]` released |
| `mimae.events_since` | `seq` | `[Event]` after that seq |
| `mimae.plan_goal` | `goal`, `tasks`, `contracts`, `at_ms` | `PlanReceipt` |
| `mimae.assign` | `agent_id`, `now_ms`, `ttl_ms` | `{task_id, lease}` |
| `mimae.next_assignment` | `agent_id` | `task_id` or null |
| `mimae.goal_progress` | `goal_id` | `{total, done, failed}` |
| `mimae.open_gate` | `gate`, `at_ms` | appended `Event` |
| `mimae.decide_gate` | `gate_id`, `passed`, `detail`, `at_ms` | appended `Event` |
| `mimae.verification_status` | `task_id` | `verified|unverified|failed` |
| `mimae.record_artifact` | `artifact`, `at_ms` | appended `Event` |
| `mimae.propose_contract` | `contract`, `at_ms` | appended `Event` |

Errors: `-32601` unknown, `-32602` bad params, `-32700` malformed line,
`-32001` protocol mismatch, `-32002` call before negotiate,
`-32000` coordinator rejected the event (the log is unchanged on rejection).

Poll `mimae.events_since` with the last `seq` you saw; Mimae never pushes
UI — your host renders. New event variants must be additive; old readers
round-trip unknown payloads verbatim.

## What the host must provide

- Running CLI sessions (one `AgentAdapter` per session), the cwd, and the
  role brief.
- An answer for approval gates (`HostHooks::on_approval`) and an escalation
  sink (`on_escalation`).
- The command lines for `Tests`/`Oracle` gates, and which git worktree
  layout you want (see `WorkspaceManager`).
- Exactly one writer per log file on Windows today (the single-writer lock
  is a unix flock; Windows is a documented stub — see `DECISIONS.md` D27).

## Where everything is

| Topic | File |
|---|---|
| Concept / requirements | `mimae-concept.md` |
| Decisions D1–D27 | `DECISIONS.md` |
| Manual setup steps | `SETUP.md` |
| AI build spec | `BUILD_PROMPT.md` |
| Core crate | `src/` |
| Node binding | `bindings/node/` |
| Executable | `src/bin/mimae-daemon.rs` |
| Examples | `examples/` |

## License

MIT OR Apache-2.0.
