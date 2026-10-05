# Mimae

Mimae is a backend coordination layer that ADE builders embed to let many
agentic coding CLIs work on one project as a self-organizing team. See
`mimae-concept.md` for the full concept brief and `DECISIONS.md` for the
decision log.

Run the live orchestration example:

    cargo run --example e2e_opencode

It spins up a local opencode serve, registers one worker, creates one task,
claims it, runs a turn via the OpenCode adapter, and appends the full
lifecycle to a temp event log under `${TMPDIR}/mimae-e2e-*`.

## Embedding

Three surfaces, in order of preference:

1. **Rust library / agentic framework** (primary). Add `mimae-core` as a
   dependency and embed the `Mimae` facade: `Mimae::open(path)` owns the
   `Coordinator`, `Mimae::register_agent` attaches one `AgentAdapter` per
   session, `submit_goal` runs deterministic planning, `drive` runs the
   head loop, and `poll_approvals` / `drain_notifications` / `drain_events`
   surface gates, escalations, and the event stream to your UI. Implement
   `HostHooks` (`on_approval`, `on_escalation`, `on_event`) for the human
   callbacks in §9.9; the default `NoHooks` approves everything. Raw
   `Coordinator` access remains available for custom orchestration. All
   coordination state lives in the append-only log at `path`; reopening the
   same path replays it exactly. No global state: multiple `Mimae`
   instances coexist in one process.
2. **Local daemon.** `cargo run --bin mimae-daemon -- --port <port> --log <path>`
   serves newline-delimited JSON-RPC 2.0 on loopback TCP for non-Rust hosts
   (Electron, Tauri, editor plugins, scripts). Every request is one JSON
   object per line; every reply carries the same `id`.
3. **Language bindings**: `bindings/node` ships the first one (napi-rs,
   JSON-string surface over `Coordinator`; `npm test` runs a full
   coordination loop from JS). Python is next, after the daemon.
   `examples/sample_ade.rs` (`cargo run --example sample_ade`) is the
   minimal Rust host: open, register agents, team lifecycle, submit goal,
   drive, human-gate approval via hooks, event subscription.

### Daemon protocol

`mimae.hello` returns `{"protocol": 1, "version": "..."}` — check `protocol`
before anything else; the number only moves on breaking wire changes.
Every connection must then call `mimae.negotiate` before any other method:
the server enforces exact-match negotiation (v1; tolerance for newer minor
versions may come later) and answers `-32002` until negotiation succeeds.
`EventPayload` values unknown to the reader are preserved verbatim, so old
readers replay newer logs without data loss.

| Method | Params | Result |
|---|---|---|
| `mimae.hello` | — | `{protocol, version}` |
| `mimae.negotiate` | `protocol`, `client` | `{protocol, compatible, server, client}` or `-32001` |
| `mimae.publish` | `actor`, `payload`, `causes`, `at_ms` | appended `Event` |
| `mimae.claim` | `task_id`, `agent_id`, `scope`, `now_ms`, `ttl_ms` | `Lease` |
| `mimae.ready_tasks` | — | `[task_id]` in deterministic order |
| `mimae.task` | `id` | `Task` or `null` |
| `mimae.team_state` | — | `"initializing"` \| `"ready"` \| … or `null` |
| `mimae.sweep_expired` | `now_ms` | `[[task_id, agent_id]]` released |
| `mimae.events_since` | `seq` | `[Event]` with seq greater than given |
| `mimae.plan_goal` | `goal`, `tasks`, `contracts`, `at_ms` | `PlanReceipt` |
| `mimae.assign` | `agent_id`, `now_ms`, `ttl_ms` | `{task_id, lease}` |
| `mimae.next_assignment` | `agent_id` | `task_id` or `null` |
| `mimae.goal_progress` | `goal_id` | `{total, done, failed}` |
| `mimae.open_gate` | `gate`, `at_ms` | appended `Event` |
| `mimae.decide_gate` | `gate_id`, `passed`, `detail`, `at_ms` | appended `Event` |
| `mimae.verification_status` | `task_id` | `"verified"` \| `"unverified"` \| `"failed"` |
| `mimae.record_artifact` | `artifact`, `at_ms` | appended `Event` |
| `mimae.propose_contract` | `contract`, `at_ms` | appended `Event` |

Errors follow JSON-RPC 2.0: `-32601` unknown method, `-32602` bad params,
`-32700` malformed line, `-32001` protocol mismatch, `-32002` call before
negotiation, `-32000` coordinator rejection (the log is never
mutated on rejection). Poll `mimae.events_since` with the last seen `seq`
to subscribe; Mimae never pushes UI — the host renders.

### Host responsibilities

The host spawns CLI sessions (one adapter per session), supplies the working
directory and role brief, answers approval gates and escalations, and provides
any test/oracle commands for verification gates. Mimae owns scheduling,
leases, routing, contracts, and git workspace operations.
