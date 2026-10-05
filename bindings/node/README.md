# mimae-node — Node.js binding for Mimae

First language binding (brief §15). Thin JSON-string surface over
`mimae-core`'s `Coordinator`, built with napi-rs. Driving turns still needs
CLI sessions (the host spawns those); this covers team lifecycle, planning,
claiming, gates, leases, and event polling.

## Build

```sh
cargo build -p mimae-node
cp ../../target/debug/libmimae_node.so ./mimae_node.node
```

(The `.node` file is a renamed cdylib; node-api symbols resolve at load.
`mimae_node.node` is git-ignored — always rebuild locally.)

## Test

```sh
npm test
```

Runs `test.js`: a full coordination loop from JS (team setup, two-task
goal, assignment, status flow to Done, progress, event polling, error
paths). 64-bit integers cross the boundary as `BigInt` — napi-rs refuses
lossy `Number` conversion, and the binding fails loudly on out-of-range
values.

## API

All functions take/return JSON strings (or an opaque numeric handle):

- `protocolVersion() -> u32` — must equal `mimae-core`'s protocol.
- `open(logPath) -> handle`, `close(handle)`.
- `publish(handle, actorJson, payloadJson, causesJson, atMs) -> Event`.
- `claim(handle, taskId, agentId, scopeJson, nowMs, ttlMs) -> Lease`.
- `planGoal(handle, goalJson, tasksJson, contractsJson, atMs) -> PlanReceipt`.
- `assign(handle, agentId, nowMs, ttlMs) -> {task_id, lease}`.
- `goalProgress(handle, goalId) -> {total, done, failed}`.
- `eventsSince(handle, seq) -> [Event]`.
- `sweepExpired(handle, nowMs) -> [[task_id, agent_id]]`.

Errors throw `Error` with the coordinator/validation message; rejected
events never touch the log.
