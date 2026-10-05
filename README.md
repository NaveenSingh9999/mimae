# Mimae

Run the live orchestration example:

    cargo run --example e2e_opencode

It spins up a local opencode serve, registers one worker, creates one task, claims it, runs a turn via the OpenCode adapter, and appends the full lifecycle to a temp event log under `${TMPDIR}/mimae-e2e-*`.
