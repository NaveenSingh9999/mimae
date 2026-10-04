# Journal

## 2026-10-04 — Cold start, workspace bootstrap

- User provided concept brief at ~/storage/downloads/mimae-concept.md; copied to repo root.
- Read the full 681-line document: Mimae is a Rust backend coordination layer enabling multiple agentic coding CLIs to self-organize on one project. Hard constraints: backend library (not app), embeddable, no manual assignment, no handoffs, no MCP, faster/smarter, in Rust. Key thesis: remove the human router, keep structure — planner-driven assignment, typed sparse messages, shared state primary, verification gates, Mimae owns git ops.
- Initialized workspace: git repo, agent state, backlog.
- Next: Phase 0 (decisions) — doc has 12 open questions for the owner. Sensible default path: adopt Section 17 recommendations as defaults (documented in decision log), start Phase 1 adapter foundation with Claude Code as first target.

## Prior art to study before adapters (Section 18)

ACP schema, gate4agent/fno-agents/fux adapters, Claude Code Agent Teams, Cursor planner/worker, C-compiler harness, CooperBench, MAST, Orca orchestration.
