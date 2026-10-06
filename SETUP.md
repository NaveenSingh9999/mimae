# Manual Setup

This guide takes you from a clean checkout to a running Mimae with all
surfaces verified. No AI needed; every step has commands you can paste.

## 1. Prerequisites

- Rust 1.85 or newer (`rustc --version`)
- `git` on PATH (workspace integration tests build temp repos)
- Node.js 18+ for the binding (optional)

You do **not** need any agent CLI for the core build or offline example.

## 2. Get the code and verify it

```sh
git clone <your-fork-url> mimae
cd mimae
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Expected: 150+ tests pass, no clippy output, no fmt diff.

## 3. Run the offline sample

```sh
cargo run --example sample_ade
```

This exercises the full flow with inline echo adapters: open log, team,
two agents, two-task goal, drive loop, human gate via hooks, event
subscription. No agent CLIs are installed or required.

## 4. Run a live orchestrator (OpenCode)

Only if `opencode` is on your PATH:

```sh
cargo run --example e2e_opencode
```

It spawns `opencode serve` on loopback, registers a worker with the
OpenCode adapter, creates one task, claims it, runs one turn, and appends
the lifecycle to a temp log. If `opencode` is missing, skip this step —
the smoke examples cover the logic.

Other adapter CLIs can be filled in the same way; only OpenCode is
live-verified on this machine, the rest pass offline parser/fixture
conformance.

## 5. Start the daemon

```sh
cargo run --bin mimae-daemon -- --port 7777 --log /tmp/mimae-run.log
```

Then from any shell:

```sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"mimae.hello","params":{}}' \
  | nc 127.0.0.1 7777
printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"mimae.negotiate","params":{"protocol":1,"client":"manual"}}' \
  | nc 127.0.0.1 7777
printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"mimae.ready_tasks","params":{}}' \
  | nc 127.0.0.1 7777
```

Each line is one JSON-RPC request; each line back is a response. Use any
TCP client; loopback only.

## 6. Build the Node binding

```sh
cargo build -p mimae-node
cp target/debug/libmimae_node.so bindings/node/mimae_node.node   # macOS: libmimae_node.dylib
cd bindings/node
npm test
```

On Windows the cdylib is `mimae_node.dll`; rename it to `mimae_node.node`
after building. If Electron cannot load it, rebuild against Electron's
headers (`npx node-gyp-rebuild` inside Electron's node headers, or use
`@electron/rebuild`). Driving from Python today means talking to the
daemon; a native Python module is a later item.

## 7. Embed into your product

Pick one:

- **Rust host or Tauri app** → depend on `mimae-core`, embed `Mimae` as
  shown in the README "Minimal Rust embedding" block.
- **Electron / VS Code fork / editor** → launch `mimae-daemon` as a child
  process, speak the JSON-RPC table in the README.
- **Node / Electron main** → `require('bindings/node')` after the build
  step above; `npm test` proves it end to end.

## 8. Verifying Windows builds

CI should run the same three commands (`cargo test`, clippy, fmt check) on
a Windows runner. Known gap: the single-writer lock around the event log
is a documented stub on non-unix; have exactly one coordinator process
per log file there until LockFileEx-based locking lands.
