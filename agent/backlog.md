# Backlog (ranked)

1. **Phase 0 — Decisions and research closure**: resolve Q1-Q12 (defaults available in Section 17), create DECISIONS.md, pick first 3-4 CLIs, license, prior-art strategy for gate4agent etc.
2. **Phase 1 — Adapter foundation**: normalized event model, adapter contract, one T1 (ACP) or T2 (headless JSON) adapter, session supervisor. Acceptance: start/prompt/stream/interrupt/resume/crash-recovery on one CLI with conformance tests.
3. **Phase 2 — Multi-CLI**: 3-4 CLI adapters, capability negotiation, PTY fallback + fixtures.
4. **Phase 3 — Shared state and coordinator**: append-only log, task graph, atomic claim, leases with heartbeat, resume after restart.
5. **Phase 4 — Workspace manager**: worktree lifecycle, path leases, conflict prediction, integration gate.
6. **Phase 5 — Messaging and context**: typed messages, router, anti-jam rules, context broker, contracts/decision log.
7. **Phase 6 — Planner and automatic assignment**.
8. **Phase 7 — Verification and security**.
9. **Phase 8 — Evaluation harness** (CooperBench, baselines).
10. **Phase 9 — Embedding**: stable library API, daemon, bindings, docs.
