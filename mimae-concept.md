# Mimae: Concept and Research Brief

| Field | Value |
|---|---|
| Document version | 0.1 (draft) |
| Written | 2026-10-04 |
| Audience | AI coding agent(s) that will design and build Mimae |
| Project owner | Naveen |
| Scope of this file | Concept, requirements, research evidence, architecture, behavior specs, roadmap. **Contains no implementation code on purpose.** |
| Language decision | Rust (stated by owner) |

---

## 0. How to use this document (read first)

**Tag legend.** Every important statement carries one tag:

| Tag | Meaning | How to treat it |
|---|---|---|
| `[STATED]` | The owner said this directly. | Hard constraint. Do not change without asking. |
| `[EVIDENCE]` | Backed by a source in Section 20. | Reliable but may be stale. Re-verify CLI flags and versions before depending on them. |
| `[PROPOSED]` | A design suggestion, not yet confirmed by the owner. | Default to it, but you may change it with a written justification in the decision log. |
| `[ASSUMPTION]` | General engineering knowledge or inference, no source. | Validate with a prototype or measurement. |
| `[OPEN]` | Undecided. | Ask the owner. Do not guess silently. |

**Rules for the implementing agent**

1. Treat `[STATED]` items (Section 3.1) as non-negotiable.
2. Do not copy the structure of existing tools blindly. Section 5 explains which of their approaches failed and why.
3. Build in the phase order of Section 14. Each phase has acceptance criteria; do not start a phase before the previous one passes.
4. Prefer structured control channels (protocols, JSON streams, hooks) over terminal screen scraping. Screen scraping is the last-resort fallback.
5. Keep a decision log (a plain markdown file in the repo) for every `[PROPOSED]` item you change or `[OPEN]` item you resolve.
6. CLI behavior changes fast. Anything in Section 10 about a specific CLI must be re-checked against the installed version before coding against it.
7. Naming warning: "ACP" is ambiguous. In this document **Agent Client Protocol (Zed ACP)** always means the editor-to-agent protocol. IBM's "Agent Communication Protocol" was merged into A2A and is not used here.

---

## 1. Summary

Mimae is a **backend coordination layer** that other developers embed inside their Agent Development Environments (ADEs). It lets many agentic coding CLIs (Claude Code, Codex, Gemini CLI, OpenCode, and others) work on one project as a self-organizing team, **without a human manually assigning each agent a job and without relaying information between agents by hand**.

Mimae is not an app and not a platform `[STATED]`. It is a library plus an optional local daemon that ADE builders integrate.

The research in this document points to one central design thesis:

> Remove the human *router*, not the *structure*. Agents should be assigned and coordinated automatically, but through a deterministic coordination core with exclusive write ownership, typed sparse messages, shared state, and verification gates. Free-form peer chat between agents measurably hurts results.

---

## 2. Problem statement

`[STATED]` Existing ADEs (the owner names Bridgemind, Nexa, and Orca) orchestrate tens of agentic CLIs, but each CLI still needs a **human to assign tasks manually**: one agent on UI/UX, another on backend, and so on. The human is the router and the memory between agents.

The owner's fix: make **inter-agent communication** possible **without handoffs**, **without MCP**, **faster**, and **smarter**.

### What the research adds to this problem

- Some of this already exists in vendor-specific or app-specific form (Section 4): Claude Code has experimental Agent Teams with a shared task list and mailbox, and Orca has an experimental orchestration feature with inter-terminal messaging and task decomposition. So the open gap is **not** "nobody has agent-to-agent messaging." The gap is a **cross-vendor, embeddable, runtime-independent, MCP-free** layer. `[EVIDENCE]`
- Naive agent-to-agent communication performs badly in measurements (Section 5, findings F1, F2, F3). A Mimae that simply connects agents with chat will probably make results worse. The product value is in the **coordination discipline**, not the pipe.

---

## 3. Requirements

### 3.1 Stated requirements (hard constraints) `[STATED]`

| ID | Requirement |
|---|---|
| R1 | Mimae is a **backend layer**, not an app or platform. |
| R2 | Other developers embed it inside their own ADEs; integration must be easy ("without hassles"). |
| R3 | It enables **inter-agent communication** between many agentic CLIs working together on one project like a team. |
| R4 | **No manual task assignment** by a human. |
| R5 | **No handoffs** (see interpretation I1 below). |
| R6 | **No MCP dependency** for the communication mechanism (see interpretation I2). |
| R7 | **Faster** communication than existing approaches (see I3). |
| R8 | **Smarter** coordination than existing approaches (see I4). |
| R9 | Implemented in **Rust**. |

### 3.2 Interpretations the implementer must follow `[PROPOSED]`

These turn vague wording into testable meaning. Confirm with the owner if a different reading was intended.

- **I1 "No handoffs".** A handoff is any step where a human, or a lead agent acting as a relay, must carry information from one agent to another before work can continue. Mimae removes human-mediated and lead-mediated relays for routine coordination. It does **not** mean "no ownership boundaries": exactly one agent owns each write scope at a time (see F1, F4).
- **I2 "No MCP".** Mimae's own coordination must not *require* MCP servers to be configured in each CLI. It may use any CLI-native channel: Agent Client Protocol, structured headless streams, local server APIs, hooks, or as a last resort terminal input. Whether Mimae should also *optionally* expose an MCP shim for CLIs with no other channel is `[OPEN]` (Section 17, Q6).
- **I3 "Faster".** Lower wall-clock time from "agent A learns something B needs" to "B acts on it," and lower coordination overhead per task (tokens and seconds). Must be measured against baselines, not asserted. See Section 12.
- **I4 "Smarter".** Higher task success rate and fewer wasted tokens than (a) a single agent doing everything, (b) lead-mediated orchestration, and (c) naive peer chat. Measured with the plan in Section 13.

### 3.3 Proposed constraints (not yet confirmed) `[PROPOSED]`

- **P1** Local-first: no required central server or cloud dependency; all state lives on the user's machine or the ADE's host. A self-hostable, privacy-preserving default.
- **P2** Core as a Rust library with a stable, versioned event stream; a thin local daemon for non-Rust hosts; language bindings later.
- **P3** Agent-agnostic through an adapter interface; adapters are separable from the core.
- **P4** Deterministic (non-LLM) coordination logic wherever possible, so the layer is fast, cheap, and testable.

### 3.4 Non-goals

- Not a UI, editor, terminal emulator for end users, or IDE.
- Not a model provider or token reseller. Users bring their own CLI subscriptions or keys.
- Not a generic multi-agent framework for non-coding domains (research agents, customer support). Coding-agent CLIs are the target.
- Not a replacement for A2A or MCP. It may interoperate with them later.

---

## 4. Landscape: what already exists

### 4.1 ADEs and orchestrators `[EVIDENCE]`

| Product | What it is | Coordination model | Relevance to Mimae |
|---|---|---|---|
| **Orca** (Stably AI, open source) | Electron desktop ADE. One git worktree per agent; fans one prompt across several agents and lets the human compare and merge. Works with Claude Code, Codex, OpenCode, Gemini and many others. | Fleet-manager scheduling plus an experimental orchestration feature (messages between agent terminals, spec decomposition into subtasks with dependencies, decision gates). Those commands are RPC calls to the **running Orca app**. | Closest peer. Its orchestration is tied to its own runtime, so it is not embeddable elsewhere. |
| **BridgeSpace / BridgeMind** | ADE with a "Bridge Agent" that orchestrates a swarm, built-in browser and IDE. BridgeMind also ships an MCP server. | Lead-agent orchestration; MCP for connecting assistants. | Shows the MCP-centric approach Mimae wants to avoid. |
| **ADE** (arul28, open source) | Local-first multi-agent workspace, tasks are "lanes" backed by git worktrees, SQLite state in the project. | Human assigns work per lane. | Confirms worktree-per-task as the standard isolation unit. |
| **Superset, Agentty, others** | More worktree-based ADE or terminal ADE tools. | Mostly human-assigned parallel tasks. | Market is crowded; embeddability is the differentiator. |
| **Nexa** | Named by the owner. | **Not identified in research.** `[OPEN]` | Ask owner for a link before comparing. |

Note: there is a separate open-source project also called "orca" (Go, MCP-based). Do not confuse it with Stably AI's Orca.

### 4.2 Vendor-native multi-agent features `[EVIDENCE]`

| Feature | Design | Notes |
|---|---|---|
| **Claude Code Agent Teams** (experimental, off by default) | A lead session plus teammate sessions, a **shared task list** (pending, in progress, completed) with dependency auto-unblocking, a **mailbox** for direct peer messages, self-claiming of tasks, file-lock based claim to prevent races. Team and task data stored locally. | Claude-only. Reported weak spots: teammates share one working directory by default, task state can go stale, a crash loses the team, token cost is high. |
| **Devin manager and child agents** | A manager agent splits a task, spawns child agents, and coordinates through an internal MCP. | Cognition reports models do not message siblings by default; that behavior needed dedicated work. |
| **Anthropic C-compiler experiment** | 16 parallel Claude instances, each in its own container, claiming tasks by writing lock files into a shared directory, syncing through a central git repo, resolving merge conflicts themselves. Roughly 2,000 sessions, about two weeks, about $20,000, ~100k lines of Rust. | Worked because tests and a reference compiler gave a verifiable success signal. See F6. |
| **Cursor long-running agents** | Hundreds of concurrent agents. Planners create tasks, workers execute, a judge decides whether to continue. | Replaced an earlier flat design that failed. See F1. |
| **Open Agent Teams skill** | A skill that runs team coordination across Claude Code, Codex, Grok and Pi. | Direct cross-CLI competitor in spirit, but delivered as a skill, not an embeddable engine. |

### 4.3 Protocols `[EVIDENCE]`

| Protocol | Solves | Status and fit for Mimae |
|---|---|---|
| **MCP** | Agent-to-**tool** access. | Universal in coding CLIs. Not designed for peer coordination. Mimae avoids requiring it (R6). |
| **A2A (Agent2Agent)** | Agent-to-**agent** task lifecycle, discovery, delegation across vendors. v1.0 reached, 150+ supporting organizations, JSON-RPC and gRPC bindings. | Network and enterprise oriented. Heavier than needed for CLIs on one machine. A candidate for a **later bridge** to remote agents, not for the local core. |
| **Agent Client Protocol (Zed ACP)** | Editor/host-to-**agent** control: sessions, prompts, streaming updates, permission requests, terminals, over stdio JSON-RPC. | 25+ agents implement it. Gemini CLI natively; Claude Code and Codex through adapters; OpenCode listed. **Highly relevant: it is a structured way for a host to drive a coding CLI.** |
| IBM Agent Communication Protocol | Agent messaging. | Merged into A2A under the Linux Foundation. Ignore. |

### 4.4 Rust prior art to study before building `[EVIDENCE]`

These came from crate documentation only. Verify maturity, licence, and activity before depending on any of them.

| Crate / project | What it does | Why it matters |
|---|---|---|
| `gate4agent` | "Universal transport library" for Claude Code, Codex, Gemini, OpenCode with pipe, PTY, ACP and daemon transports; per-CLI session history and resume. | Overlaps Mimae's lowest layer. Study its adapter shapes. Decide: depend on it, learn from it, or ignore. |
| `fno-agents` | PTY supervisor for persistent, attachable multi-CLI agents using `alacritty_terminal` for screen state. | Reference for supervising long-lived agent processes. |
| `fux` / `zor` | Wraps an agent in a PTY, keeps a `vt100` screen, classifies the agent as working, blocked, or idle using per-agent rules with fixture-based tests. | Reference for the PTY fallback tier. |
| `multistack` | Terminal app managing multiple agent PTYs with `vt100`. | Minor reference. |
| `harness-test` (Go) | Conformance suite that exercises many coding-agent CLIs across headless, ACP, SDK and hook control modes. | Reference for what each CLI actually supports; a model for Mimae's own adapter conformance tests. |
| Agent Client Protocol Rust crates (schema) | Official Rust types for the protocol. | Use rather than re-implementing the schema. |

---

## 5. Research findings and their implications

Each finding states what was found, then **what it means for Mimae**.

### F1. Flat self-coordination fails; light hierarchy works `[EVIDENCE]`
In Cursor's large-scale experiments, agents first coordinated through a shared file with locks. Agents held locks too long or forgot to release them, and with no hierarchy they became risk-averse, so work churned without progress. A planner/worker split fixed it: planners create tasks (and can spawn sub-planners), workers execute a task end to end without coordinating with other workers, and a judge decides whether to continue. Their summary: simpler structures beat complex ones, and prompting mattered a lot.
**Implication:** Mimae's "automatic assignment" should be planner-driven with worker isolation, not a free-for-all. Leases need **expiry and heartbeats**, never indefinite locks.

### F2. The "curse of coordination" is measured `[EVIDENCE]`
CooperBench (600+ collaborative coding tasks, 12 libraries, 4 languages) found agents average about **30% lower success** working as a pair than one agent doing both tasks, and in one setup two-agent cooperation reached only about 25%, roughly half the solo baseline. Even when each agent could solve its own part, the best models failed to coordinate about half the time. Human pairs solved about 9 of 10 tasks under the same constraints where agents managed about 3 of 10. Three failure modes were identified:
1. Communication channels become jammed with vague, badly timed, or inaccurate messages.
2. Agents deviate from commitments and hold wrong expectations about others.
3. Agents use version-control tools poorly for real-time coordination.
Rare positive behaviors included role division, resource division, and negotiation.
**Implication:** This is the benchmark Mimae must beat. Design against all three failure modes: typed, rate-limited messages (mode 1); explicit contracts with verification (mode 2); Mimae-managed git operations instead of agent-managed ones (mode 3). See Section 13.

### F3. Multi-agent systems fail in classifiable ways `[EVIDENCE]`
The MAST study (1,600+ annotated traces across 7 frameworks) defines 14 failure modes in 3 categories: system design / specification issues, inter-agent misalignment, and task verification.
**Implication:** Use MAST categories as the checklist for Mimae's failure handling and for tagging failures during evaluation. Verification (the third category) must be a first-class part of the design, not an afterthought.

### F4. Context sharing and implicit decisions (Cognition) `[EVIDENCE]`
Cognition's 2025 argument: agents must share context, and "actions carry implicit decisions," so parallel writers make conflicting choices about style, edge cases, and patterns. Their April 2026 follow-up found a narrower class that works: **multiple agents contribute intelligence while writes stay single-threaded.** Examples: a clean-context reviewer that works best when it does **not** share the coder's context beforehand; a stronger "smart friend" model consulted by a primary model (works across strong frontier models); a manager that splits work and child agents that execute. They call unstructured swarms (arbitrary networks negotiating) mostly a distraction, and note that cross-agent messaging (a child surfacing a discovery that should change siblings' work) does not happen by default because models were not trained for it.
**Implication:** Mimae should not rely on agents to volunteer coordination. The **coordination layer must do it for them**: detect discoveries and decisions, record them centrally, and deliver them to affected agents. Reviewers should get clean context by default. Exclusive write ownership is the safe default.

### F5. Orchestrator-worker is the proven pattern, and it is expensive `[EVIDENCE]`
Anthropic's multi-agent research system (lead plus parallel subagents) beat a single-agent baseline by 90.2% on an internal research eval but used about 15x the tokens of a chat. Token usage alone explained about 80% of performance variance. It worked best on breadth-first tasks with independent threads. Early failures came from vague delegation; fixes were explicit objectives, boundaries, and "do not research X, another agent owns it" instructions.
**Implication:** Coordination overhead must be minimized or the extra agents are a tax. Delegation briefs generated by Mimae must always state objective, boundaries, and what other agents own.

### F6. Large autonomous runs depend on verification and conflict handling `[EVIDENCE]`
In the C-compiler run, success relied on tests, a known-good reference compiler used as an oracle to split failing work across agents, containers for isolation, lock files for claiming, and frequent merge-conflict resolution. It was a near-ideal task: well-specified, with comprehensive tests.
**Implication:** Mimae's value is lower where no tests exist. Provide hooks for the host to supply verification (tests, oracles, reviewers). When all agents collapse onto the same bug, partitioning the search space is a coordinator job.

### F7. Shared task list plus mailbox is the converged primitive `[EVIDENCE]`
Claude Code Agent Teams, Orca's orchestration, and the C-compiler harness all use a task list or lock mechanism plus messages. Atomic claiming uses file locks.
**Implication:** Adopt the task graph and atomic claim, but fix the reported weaknesses: shared working directory (use isolated worktrees), stale state (heartbeat and lease expiry), crash loss (durable append-only state and resume).

### F8. Blackboard-style shared state has research support `[EVIDENCE]`
LLM blackboard systems, where agents post to and read from a shared board, outperformed a master-slave baseline by 13% to 57% relative on a data-discovery task, and another blackboard design matched state of the art with fewer tokens. A separate preprint argues for replacing opportunistic activation with a deterministic scheduler to reduce untraceable emergent behavior.
**Implication:** Make shared state the **primary channel** and messages secondary pointers into it. Use a deterministic scheduler. Caveat: these results are not from coding tasks, so treat as supporting, not conclusive.

### F9. Worktrees are the baseline; integration is where conflicts really surface `[EVIDENCE]`
Practitioner reports agree: one git worktree and branch per agent, then integration as a **gate, not a plain merge**: merge into a temporary integration branch, run the full build and tests before it reaches main. Semantic conflicts can exist even when git reports none (shared DTOs, API specs, build files). Many "conflicts" in dependency-graph parallel work are just stale bases and are fixed by rebasing. Conflict prediction by comparing changed paths and hunks across branches catches overlaps early. Credential isolation matters because agents run model-generated commands.
**Implication:** Mimae owns git operations (worktree creation, rebasing, integration branch, merge order). Agents should not be trusted to coordinate through git by themselves (CooperBench failure mode 3).

### F10. Coding CLIs now expose structured control surfaces `[EVIDENCE]`
This is the key enabling finding for R6 and R7.
- **Claude Code:** non-interactive mode with JSON and streaming-JSON output, and a streaming-JSON **input** mode for multi-turn control over stdin, session resume, tool-permission handling, and a hooks system.
- **Codex CLI:** non-interactive exec mode with a JSON event stream, resume support, and an app-server that speaks JSON-RPC over stdio; an ACP adapter exists.
- **Gemini CLI:** headless streaming JSON and a native experimental ACP mode.
- **OpenCode:** a headless HTTP server with an OpenAPI 3.1 spec, a server-sent-events stream, a JS SDK, and the ability to drive a running TUI.
- **Agent Client Protocol:** 25+ agents, with clients including Zed, JetBrains, and Microsoft's terminal agent pane.
- **Hooks:** lifecycle hooks (tool completed, prompt submitted, agent stopping) that can **inject additional context into the agent** and can force an agent to keep going. Claude-format hooks are read by other tools too. Behavior differs between CLIs and modes.
**Implication:** Mimae can talk to agents **without MCP and without screen scraping** in most cases. Hooks give a low-latency channel for delivering a peer message into a running agent at a safe moment. Terminal (PTY) control becomes the fallback tier only.

### F11. Terminal state detection is feasible but per-agent work `[EVIDENCE]`
Existing Rust tools combine a PTY library with a terminal-state parser to classify an agent as working, blocked, or idle using per-agent rules tested against captured screen fixtures.
**Implication:** Keep the PTY tier, but treat each CLI's screen rules as data (fixtures plus rules), with conformance tests.

### F12. Inter-agent boundaries are an attack surface `[EVIDENCE]`
A 2026 threat-model study enumerated 14 injection vectors across four categories (user input, tool outputs, inter-agent messages, orchestrator manipulation). Against a 6-agent system, 67% of agents were vulnerable to at least one scope violation despite system-prompt guardrails. Four architectural defenses cut aggregate injection success from 31.2% to 4.2%: message signing with provenance tracking, sanitization at agent boundaries, privilege-scoped tools per role, and anomaly detection on inter-agent traffic. This is one study of one system.
**Implication:** Mimae is a new trust boundary between agents. Peer messages must be treated as untrusted data, not instructions, with provenance on every message.

### F13. Protocol landscape has settled into layers `[EVIDENCE]`
MCP is for agent-to-tool, A2A for agent-to-agent, Zed ACP for editor-to-agent. Most production systems use more than one.
**Implication:** Mimae sits between the host and the local agent CLIs, which is the Agent Client Protocol layer plus coordination logic. A2A is a future bridge for remote agents.

---

## 6. Design thesis (synthesis)

| # | Principle | Why (finding) |
|---|---|---|
| T1 | Remove the human **router**, keep **structure**: planner assigns, workers own write scopes. | F1, F4 |
| T2 | Messages are **typed, sparse, and about contracts, blockers, and discoveries**. No chat. | F2 (jammed channels) |
| T3 | **Shared state is primary**; messages point into it. | F8, F7 |
| T4 | **Deterministic core, LLM at the edges.** Scheduling, leasing, routing, and conflict prediction are non-LLM. LLMs plan, review, and judge semantics. | F8, F5 (cost), R7 |
| T5 | **Structured channels first**, PTY last. | F10 |
| T6 | **Verification gates** are first-class. | F3, F6, F9 |
| T7 | **Mimae owns git and workspace operations.** | F2 mode 3, F9 |
| T8 | **Context is curated, not dumped.** Share decisions and contracts; give reviewers clean context. | F4 |
| T9 | **Every inter-agent message is untrusted input** with provenance. | F12 |
| T10 | **Measure against CooperBench-style tasks and baselines**, or the "smarter" claim is unproven. | F2 |

### Where the owner's expectation meets the evidence

| Owner expectation | Evidence | Resolution in Mimae |
|---|---|---|
| Agents talk directly with no handoffs. | Free peer coordination drops success (F2) and flat swarms fail (F1, F4). | Direct messaging exists, but only typed messages routed by the core, and only between agents whose scopes interact. Humans and lead agents are not relays. |
| No manual assignment. | Planner/worker hierarchy works (F1, F5). | Automatic assignment through a planner role plus capability-based claiming. Human can still steer and approve (Section 9.9). |
| No MCP. | Structured CLI channels exist (F10). | Tiered adapters: ACP, structured headless, server API, hooks, PTY. |
| Faster. | Tokens dominate cost (F5); coordination overhead can exceed benefit. | Deterministic routing, digests, hook-time delivery, low message volume. Measure it. |
| Smarter. | Verification and contracts matter (F3, F4, F6). | Contract-first coordination, clean-context review, integration gates. |

---

## 7. Conceptual architecture `[PROPOSED]`

```text
   Host ADE (any language)
        |  library API  /  local socket (daemon)  /  bindings
        v
  +---------------------------------------------------------------+
  |                          MIMAE CORE                           |
  |                                                               |
  |  Coordinator (deterministic)   Planner role (LLM agent)       |
  |   - scheduler, claiming         - goal -> task graph          |
  |   - routing, leases             - contracts, scopes           |
  |                                                               |
  |  Message Router & Delivery     Context Broker                 |
  |   - inboxes, priorities         - briefs, digests, decisions  |
  |   - hook-time injection                                       |
  |                                                               |
  |  Workspace Manager             Policy & Security              |
  |   - worktrees, path leases      - provenance, trust classes   |
  |   - integration gate            - scoped permissions, gates   |
  |                                                               |
  |  Shared State Store (append-only log + materialized views)    |
  |                                                               |
  |  Session Supervisor  <->  Adapter Layer (tiers T1..T4)        |
  +---------------------------------------------------------------+
        |            |             |              |
     Claude Code    Codex       Gemini CLI     OpenCode  ... others
```

### 7.1 Components

| Component | Responsibility |
|---|---|
| **Adapter layer** | Normalizes each CLI behind one capability model: start, send a turn, receive events, detect idle/blocked, interrupt, resume, answer permission prompts, inject context. Tiered transports (Section 10). |
| **Session supervisor** | Process lifecycle, restart, resume, budgets, rate limits, stuck detection. |
| **Shared state store** | Durable, local, append-only event log with materialized views (task graph, leases, contracts, decisions). Survives crashes. Replaceable behind an interface. |
| **Coordinator** | Deterministic scheduler: dependency resolution, capability matching, atomic claim, lease expiry, load balancing. |
| **Planner role** | An LLM agent (any CLI) that turns a goal into a task graph with scopes and contracts, and wakes when tasks complete to plan the next step (as Cursor found helpful). |
| **Message router and delivery** | Typed messages, inboxes, priority, dedupe, rate limits, digests, and delivery at safe moments. |
| **Context broker** | Builds each agent's brief: task, scope, relevant contracts, decision-log digest, what peers own. |
| **Workspace manager** | Worktree per agent, branch naming, path leases, conflict prediction, rebasing, integration branch and gate, cleanup. |
| **Policy and security** | Trust classes, message provenance and signing, scoped permissions per role, approval gates, anomaly detection on traffic. |
| **Observability and evaluation** | Event stream to the host, traces, replay, metrics, benchmark runner. |
| **Embedding surfaces** | Library core, local daemon, later bindings (Section 15). |

---

## 8. Core concepts and data model (conceptual) `[PROPOSED]`

### 8.1 Entities

| Entity | Meaning | Key fields (conceptual) |
|---|---|---|
| **Goal** | A human-provided objective for the project. | text, acceptance criteria, constraints, budget |
| **Task** | Unit of work in a dependency graph. | id, title, brief, role/capability needed, write scope, dependencies, status, owner, lease, verification requirement |
| **Agent** | A running CLI session under Mimae. | id, CLI type, capabilities, role, workspace, state (idle/working/blocked/dead), budget used |
| **Role / capability** | What an agent is good for or assigned to (frontend, backend, tests, review, integrator). | name, preferred models/CLIs, tool permissions |
| **Scope (lease)** | Exclusive right to write a set of paths or components for a time. | owner, paths/components, expiry, heartbeat |
| **Contract** | An agreed interface between tasks (API shape, data schema, file ownership boundary) that parallel work depends on. | parties, content, version, status (proposed/accepted/changed) |
| **Decision** | A recorded choice that affects others (naming, patterns, library choice). | author, rationale, affected scopes, timestamp |
| **Message** | Typed communication between agents or Mimae and an agent. | type, sender, recipients, refs (task/contract/decision), priority, provenance |
| **Event** | Immutable log entry for anything that happens. | type, actor, payload, time, causal refs |
| **Artifact** | Output of work: branch, diff, test report, review. | location, producer, verification status |
| **Gate** | A checkpoint requiring verification or human approval. | condition, evaluator (tests/agent/human), result |

### 8.2 Task states
`proposed` -> `ready` (dependencies met) -> `claimed` -> `in_progress` -> `in_review` -> `integrating` -> `done`. Side states: `blocked` (waiting on contract, message, or human), `failed`, `abandoned` (lease expired), `cancelled`.

### 8.3 Message taxonomy
Only these types exist in v1. Anything else is rejected, to avoid channel jamming (F2).

| Type | Purpose | Delivery priority |
|---|---|---|
| `contract_proposal` / `contract_accept` / `contract_change` | Agree interfaces before or during parallel work. | High, delivered at next safe point |
| `blocker` | "I cannot proceed because of X owned by Y." | High |
| `discovery` | A finding that may change others' work (bug found, assumption wrong). Routed only to affected scopes. | Medium |
| `info_request` / `info_response` | A bounded question with a deadline. | Medium |
| `status_milestone` | Coarse progress (not step-by-step chatter). Mostly consumed by the host UI and the coordinator. | Low, batched |
| `review_request` / `review_result` | Clean-context review exchange. | Medium |
| `escalation` | Needs human or planner decision. | High |

Every message carries references to the task, contract, or decision it concerns. Free text is allowed only inside these types and is length-capped.

---

## 9. Key mechanisms (behavior specifications)

### 9.1 Goal intake and decomposition `[PROPOSED]`
1. Host submits a goal (with optional constraints, budget, acceptance criteria).
2. Coordinator starts a **planner** agent with a clean brief: goal, repo map, constraints.
3. Planner outputs a task graph: tasks, dependencies, write scopes, required capabilities, and **contracts** for interfaces that parallel tasks share.
4. Mimae validates the plan deterministically: no overlapping write scopes among tasks that can run in parallel, no dependency cycles, every parallel interface has a contract.
5. Planner can spawn sub-planners for large areas. Planner wakes when a batch completes to plan the next step. `[EVIDENCE: Cursor]`
6. If the host supplies tests or an oracle, the plan includes verification gates tied to tasks.

### 9.2 Assignment and claiming `[PROPOSED]`
- Ready tasks sit in the shared task list. Agents with matching capability are offered tasks by the **coordinator** (push) or claim them (pull); the claim is atomic so two agents never own the same task.
- Claims are **leases** with a time limit and heartbeat. A lease that stops heartbeating expires, the task returns to `ready`, and the owner is flagged. This directly addresses the forgotten-lock failure reported by Cursor.
- Matching considers role, current load, CLI strengths, and cost tier (a cheap model for routine tasks, a strong model for hard ones). Cost-aware routing is `[ASSUMPTION]` until measured; Cognition's "smart friend" findings suggest capability routing across strong models is where gains show up.

### 9.3 Messaging and delivery `[PROPOSED]`
- **Who receives what:** the router sends a message only to agents whose scopes, contracts, or dependencies it touches. Broadcasts are forbidden except for human-originated notices.
- **When to deliver** (in order of preference): (1) at a hook point inside a running turn via injected context; (2) as a prefix to the agent's next turn; (3) by interrupting the agent only for `blocker` and `escalation` messages that invalidate current work; (4) for idle agents, by starting a new turn through the structured channel.
- **Anti-jam rules:** per-agent rate limits, dedupe of near-identical messages, batching of low-priority messages into digests, a cap on message length, and a requirement that questions be answerable (bounded, with deadline).
- **Silent-agent problem:** since agents do not volunteer discoveries on their own (F4), Mimae observes agent output and events and **creates `discovery` and `decision` records itself** when it detects them, subject to confirmation rules. Which detection method to use is `[OPEN]` (Q8).

### 9.4 Contract-first coordination `[PROPOSED]`
Before two tasks that touch the same interface run in parallel, the planner or the involved agents agree a **contract**. Contracts are versioned records, not chat. Changing a contract triggers delivery to every party and, if work is in flight, a controlled pause. This targets the "actions carry implicit decisions" and "deviation from commitments" failure modes (F2, F4).

### 9.5 Context sharing policy `[PROPOSED]`
| Situation | Policy |
|---|---|
| Worker starting a task | Brief: task, scope, acceptance criteria, relevant contracts, decision-log digest, list of what peers own. Not the full history of other agents. |
| Reviewer | **Clean context** by default: diff plus spec. Coder's reasoning withheld. `[EVIDENCE: Cognition]` |
| Escalation to a stronger agent | Share a fork of the primary agent's full context, and let the stronger agent ask for missing files rather than invent. `[EVIDENCE: Cognition]` |
| Long-running agent | Mimae refreshes the brief and recommends context resets to counter context rot. `[ASSUMPTION]` |

### 9.6 Workspace isolation and integration `[PROPOSED]`
1. **Worktree and branch per agent**, created and cleaned up by Mimae.
2. **Path leases** (Section 8.1) for shared files agents cannot avoid touching (lockfiles, shared types, build config).
3. **Conflict prediction:** compare changed paths and nearby hunks across active branches continuously; warn or reorder before the merge. `[EVIDENCE]`
4. **Single integrator:** one logical owner of integration. Merge into an **integration branch**, run build and tests, and only then advance the main line. Merge order follows task dependencies. Rebase stale branches automatically before treating a conflict as real. `[EVIDENCE]`
5. **Semantic-conflict gate:** a check (tests, type check, or reviewer) after integration, because clean textual merges can still break behavior.
6. **Credential isolation:** agents get only the credentials their role needs.

### 9.7 Verification `[PROPOSED]`
- Gate types: automated tests/build, differential check against an oracle if available, clean-context agent reviewer, human approval.
- Reviewer and coder can iterate, with a loop cap. Cognition reports this finds real bugs, but notes loops can take long. `[EVIDENCE]`
- If no verification exists, Mimae labels results **unverified** in its event stream rather than presenting them as done.

### 9.8 Failure handling `[PROPOSED]`
Map to MAST categories (F3).

| Failure | Detection | Response |
|---|---|---|
| Agent crash or exit | Process supervision | Release lease, restart or reassign, resume from session if the CLI supports it |
| Stuck or looping agent | No progress events, repeated identical actions, time-in-state limit | Interrupt, nudge with a targeted prompt, then reassign |
| Runaway cost | Per-agent and per-goal budget | Pause and escalate |
| Expired lease / stale task | Heartbeat timeout | Return task to `ready`, flag |
| Conflicting decisions discovered | Contract/decision overlap check | Pause affected agents, notify, require a contract change |
| Integration failure | Gate result | Rebase and retry; otherwise create a repair task owned by one agent |
| Prompt injection suspected | Provenance and anomaly rules (Section 11) | Quarantine the message, notify host |
| All agents attack the same failure | Duplicate-work detection | Partition the search space (oracle-style), as in the C-compiler run `[EVIDENCE]` |

### 9.9 Human in the loop `[PROPOSED]`
No *routine* human routing, but the human can: set the goal, approve plan or gates, answer escalations, override assignments, pause or kill agents, and inspect everything through the event stream. Decision gates exist in Orca's design too `[EVIDENCE]`; Mimae should expose them as host callbacks.

---

## 10. Transport and CLI adapter specification

### 10.1 Transport tiers `[PROPOSED]`
Prefer the highest tier an agent supports.

| Tier | Mechanism | Strengths | Weaknesses |
|---|---|---|---|
| **T1** | Agent Client Protocol (stdio JSON-RPC) | Standardized sessions, prompts, streaming updates, permission requests; one implementation covers many CLIs | Not every CLI is native; adapters add a dependency |
| **T2** | CLI structured headless mode (streaming JSON in and out) | Official, reliable, no screen parsing | Per-CLI event formats; multi-turn support varies |
| **T3** | Local server API (for CLIs that offer one, e.g. OpenCode HTTP + events) | Rich control, events, multiple clients | CLI-specific |
| **T3b** | Hooks as a **delivery side-channel** | Inject context into a running agent at safe points without typing into a terminal | Event coverage and behavior vary by CLI and mode |
| **T4** | PTY plus terminal-state parsing | Works with anything interactive | Fragile, per-CLI rules, slowest to maintain |

### 10.2 Per-CLI status (verify before coding) `[EVIDENCE]`

| CLI | Channels reported available | Notes |
|---|---|---|
| Claude Code | Headless JSON/stream-json (including streaming input), session resume, hooks, Agent SDK; ACP via a Zed adapter | Hooks can inject context and force continuation. Check hook availability per mode. |
| Codex CLI | Exec with JSON event stream, resume, app-server JSON-RPC, ACP via adapter | Codex runs terminal commands in its own process (differs from some agents in ACP behavior). |
| Gemini CLI | Headless stream-json, native experimental ACP | First external ACP integration. |
| OpenCode | `serve` HTTP/OpenAPI server with event stream, JS SDK, `run` headless, ACP listed | Optional server password; session API. |
| Others (Goose, Kimi, Cursor CLI, Copilot CLI, Droid, etc.) | Many list ACP or headless modes | Check the ACP agents list and a conformance suite. |

### 10.3 Adapter contract (what every adapter must provide) `[PROPOSED]`
In prose, not code:
1. Start a session in a given working directory with a given role brief and permission profile.
2. Send a turn (prompt) and receive a **normalized event stream**: text, tool use, tool result, permission request, usage and cost, turn-complete, error.
3. Report agent state: working, idle, blocked-on-permission, dead.
4. Interrupt a running turn.
5. Resume a previous session.
6. Answer or auto-resolve permission requests according to policy.
7. Provide a safe **context-injection path** (hook, prefix-on-next-turn, or new turn).
8. Declare its **capabilities** so the coordinator can degrade gracefully (for example, no interrupt available).

Adapters must ship with **conformance tests** (modeled on the existing coding-agent conformance suite) and, for the PTY tier, **captured-screen fixtures**.

### 10.4 Adding a new CLI
A third party should be able to add a CLI by implementing the adapter contract and its fixtures, without touching the core.

---

## 11. Security and trust model `[PROPOSED]` (grounded in F12)

1. **Peer messages are data, not instructions.** Delivered with a clear provenance header and framed so an agent treats them as information from a colleague, never as authority over its tools or permissions.
2. **Provenance and signing** on every message and event (who, which task, which scope).
3. **Boundary sanitization:** strip or flag content that looks like injected instructions when it crosses an agent boundary; never forward raw tool output between agents without framing.
4. **Least privilege per role:** reviewers read-only; workers write only inside their scope; only the integrator touches the main line; secrets scoped.
5. **Anomaly detection** on traffic: unusual volume, unexpected recipients, messages that request privileged actions.
6. **Approval gates** for dangerous actions (deleting, force operations, network access, dependency changes).
7. **Local-first storage:** no data leaves the machine unless the host configures it. `[PROPOSED, see P1]`
8. Open question: sandboxing level (containers vs worktrees only). Q9.

---

## 12. Performance: what "faster" means `[ASSUMPTION unless measured]`

Where time and tokens go in multi-agent coding:
- **Coordination latency:** time between a discovery and the affected agent acting on it. Main levers: hook-time delivery, event-driven routing instead of polling, no LLM in the routing path.
- **Token overhead:** every extra agent re-reads context. Main levers: curated briefs, digests, small typed messages, clean-context reviewers instead of long shared transcripts, avoiding agents sharing the same exploration.
- **Rework:** conflicts and wrong assumptions discovered late. Main levers: contracts, conflict prediction, early integration.
- **Idle time:** agents waiting on dependencies or on a human. Main levers: good graph partitioning, ready-task prefetching, planner that wakes on completion.

Hypotheses to test (not claims):
- H1: Hook-time delivery reduces discovery-to-action latency versus lead-mediated relay.
- H2: Typed sparse messaging reduces tokens spent on coordination versus free chat, with equal or better success.
- H3: Mimae-managed git and integration gates reduce conflict rework versus agent-managed git.

---

## 13. Evaluation plan `[PROPOSED]`

"Smarter" and "faster" are only real if measured.

### 13.1 Baselines
1. **Solo:** one agent does everything.
2. **Lead-mediated:** a lead agent relays between workers.
3. **Naive peer chat:** agents message freely with no typing or routing.
4. **Vendor-native:** Claude Code Agent Teams where applicable.
5. **Mimae.**

### 13.2 Benchmarks and scenarios
- **CooperBench** (open source): pairs of agents implement features that can conflict; measures the coordination deficit versus solo. Supports a no-messaging variant and a git-collaboration variant, so Mimae's messaging and git management can be ablated. `[EVIDENCE]`
- **Own scenarios:** feature spanning UI + API + data; refactor across modules; bug-hunt where many agents could collide; greenfield small app with tests.
- **Failure tagging:** classify failures using MAST categories. `[EVIDENCE]`

### 13.3 Metrics
| Metric | Why |
|---|---|
| Task success rate (tests pass) | Core outcome |
| Coordination deficit (pair vs solo) | Directly comparable with CooperBench |
| Wall-clock time to completion | "Faster" |
| Total tokens and cost | Overhead tax (F5) |
| Coordination tokens as a share of total | Chatter measure |
| Discovery-to-action latency | Delivery speed (H1) |
| Conflict count and rework | Isolation quality (H3) |
| Message count, types, and useful-message rate | Anti-jam (F2) |
| Lease expiries, stuck events, restarts | Robustness |
| Human interventions per goal | "No manual assignment" (R4) |

### 13.4 Success criteria (initial targets, to be set by the owner) `[OPEN]`
Suggested: no worse than solo on CooperBench-style pairs (closing the deficit), lower coordination tokens than naive peer chat, zero lost work on agent crashes, and zero human relays in the scenarios.

---

## 14. Roadmap with acceptance criteria `[PROPOSED]`

| Phase | Goal | Deliverables | Acceptance criteria |
|---|---|---|---|
| **0. Decisions and research closure** | Resolve blocking `[OPEN]` items. | Decision log; list of target CLIs; license choice; chosen prior-art strategy (use, learn from, or ignore each crate in 4.4). | Owner has answered Q1 to Q6 and Q10. |
| **1. Adapter foundation** | Drive one CLI reliably over a structured channel. | Normalized event model; adapter contract; one T1 or T2 adapter; session supervisor. | Start, prompt, stream, interrupt, resume, and crash recovery work on one CLI with conformance tests. |
| **2. Multi-CLI** | Support 3 to 4 CLIs. | Adapters for several CLIs; capability negotiation; PTY fallback with fixtures for at least one. | Same conformance suite passes on each CLI; capability degradation tested. |
| **3. Shared state and coordinator** | Durable state plus deterministic scheduling. | Append-only log, task graph, atomic claim, leases with heartbeat, resume after restart. | Kill the process mid-run and resume with no lost tasks; no double-claims under concurrent claim stress. |
| **4. Workspace manager** | Isolation and integration. | Worktree lifecycle, path leases, conflict prediction, integration branch gate. | Parallel agents on overlapping areas never corrupt each other; integration gate blocks a deliberately broken merge. |
| **5. Messaging and context** | Typed messages and delivery. | Message router, hook-time and next-turn delivery, anti-jam rules, context broker, contracts and decision log. | Discovery from agent A reaches affected agent B within the target latency; message volume stays under caps. |
| **6. Planner and automatic assignment** | Remove the human router. | Planner role, plan validation, capability matching, planner wake-on-completion. | A goal runs end to end with zero human assignment on the test scenarios. |
| **7. Verification and security** | Make it trustworthy. | Gates (tests, reviewer, human), provenance, scoped permissions, anomaly checks. | Reviewer loop works; injected-instruction test messages are quarantined. |
| **8. Evaluation harness** | Prove the claims. | CooperBench integration, baselines, metrics dashboard output. | Results published against baselines in Section 13. |
| **9. Embedding** | Make it easy for ADEs. | Stable library API, versioned event stream, local daemon, first language binding, docs and examples. | A sample ADE integrates in minimal steps; protocol version negotiation works. |

---

## 15. Embedding surfaces `[PROPOSED]`

ADE builders need a small, stable surface. The host does three things: **spawn agents**, **submit goals**, **subscribe to events**, plus answer gates and escalations.

| Surface | For | Notes |
|---|---|---|
| **Rust library** | Rust hosts, maximum speed | Primary artifact. |
| **Local daemon** with a local socket (JSON-RPC style) | Electron, Tauri, VS Code forks, any language | Most ADEs are not written in Rust, so this likely matters most. |
| **Language bindings** (Node, Python) | Direct embedding | After the daemon. |

Design rules:
- **Events out, never UI.** Mimae renders nothing; the host decides how to display.
- **Versioned protocol** from day one with negotiation.
- **No global state:** multiple Mimae instances can coexist in one process.
- **Pluggable storage and adapters** through interfaces.
- **Host callbacks** for approval gates, escalations, and verification (tests, oracles).
- **Licensing matters for adoption:** ADE builders embedding Mimae will care about license terms (note some ADEs are MIT, another is AGPL). `[OPEN]` Q4.

---

## 16. Risks and mitigations

| Risk | Likelihood / impact | Mitigation |
|---|---|---|
| Coordination makes results worse (CooperBench-style deficit) | High / critical | Contracts, typed sparse messaging, single-writer scopes, measure early (Phase 8 prototype benchmark even earlier if possible) |
| CLI behavior and flags change often | High / high | Capability-declaring adapters, conformance tests, version pinning, fixture-driven PTY rules |
| Overlap with existing tools (vendor features, Orca, gate4agent, Open Agent Teams) | Medium / high | Position on cross-vendor + embeddable + MCP-free; study prior art in 4.4 before building |
| Token cost balloons | High / medium | Budgets, deterministic routing, digests, clean-context reviewers, cost-aware assignment |
| Prompt injection through agent messages | Medium / high | Section 11 |
| Hooks and structured channels are not available for some CLIs | Medium / medium | Tiered fallback, PTY tier, explicit capability degradation |
| Tasks with no verifiable success criterion | High / medium | Label results unverified; support reviewer agents and human gates |
| Scope creep into building an ADE | Medium / medium | Non-goals in 3.4; events out, never UI |
| Semantic conflicts that merges cannot catch | Medium / high | Integration gate with tests and review |
| Solo-developer maintenance burden across many CLIs | High / medium | Keep adapters separable, share fixtures, prioritize 3 to 4 CLIs first |

---

## 17. Open questions for the owner

| # | Question | Recommendation |
|---|---|---|
| Q1 | Which CLIs must be supported first? | Claude Code, Codex, Gemini CLI, OpenCode; they all expose structured channels. |
| Q2 | Is **local-first with no central server** a hard requirement (P1)? | Yes: it fits the embeddable and privacy goals. |
| Q3 | Is a **planner/worker hierarchy** acceptable, given that evidence shows flat peer swarms fail? | Yes, as long as it is automatic. |
| Q4 | License for Mimae? | Permissive licence eases embedding in other ADEs. |
| Q5 | Should Mimae depend on existing Rust crates (such as `gate4agent`) or build its own adapters? | Study first; avoid duplicating the transport layer if one is solid. |
| Q6 | Should an **optional** MCP shim exist for CLIs with no other channel? | Optional and off by default. |
| Q7 | What are the target success criteria (Section 13.4)? | Match or beat solo on CooperBench-style pairs. |
| Q8 | How should Mimae detect discoveries and decisions in agent output: rules, a small classifier agent, or explicit agent reports? | Start with explicit structured reports plus lightweight rules; add a classifier later. |
| Q9 | Sandboxing depth: worktrees only, or containers too? | Worktrees first; containers optional per host. |
| Q10 | What is "Nexa"? | Provide a link so it can be compared. |
| Q11 | Should Mimae ever bridge to A2A for remote agents? | Later phase, not core. |
| Q12 | Windows support priority? | Decide early because PTY and process handling differ. |

---

## 18. Prior art checklist (study before writing adapters)

1. Agent Client Protocol: schema, session lifecycle, permission requests, terminal handling, known adapters (including how the Codex adapter handles terminal commands).
2. `gate4agent`, `fno-agents`, `fux/zor`: adapter structure, screen-state rules, resume handling.
3. Claude Code Agent Teams: task list format, claim locking, mailbox semantics, the weaknesses reported above.
4. Cursor planner/worker design and what failed first.
5. Anthropic C-compiler harness: lock-file claiming, oracle-based partitioning, test design.
6. CooperBench: task format, messaging setup, git setup, evaluation scripts.
7. MAST failure taxonomy: use as the failure checklist.
8. Orca's orchestration feature (experimental): message passing, decomposition, decision gates.

---

## 19. Glossary

| Term | Meaning |
|---|---|
| **ADE** | Agent (or Agentic) Development Environment: a workspace for running, isolating, monitoring, and reviewing coding agents. The term is still settling. |
| **Agentic CLI** | A terminal-based AI coding agent (Claude Code, Codex CLI, Gemini CLI, OpenCode, and so on). |
| **Handoff** | A relay of information between agents carried by a human or lead agent. |
| **Lease** | A time-limited exclusive claim on a task or write scope, kept alive by heartbeats. |
| **Contract** | An agreed, versioned interface between parallel tasks. |
| **Scope** | The set of files or components an agent may write. |
| **Integrator** | The single logical owner of merging work into the main line. |
| **Worktree** | A git feature giving each agent its own working directory and branch sharing one repository. |
| **Hook** | A CLI lifecycle callback that can observe or inject into an agent turn. |
| **Zed ACP** | Agent Client Protocol: standard for hosts to drive coding agents. |
| **A2A** | Agent2Agent protocol for cross-vendor agent collaboration. |
| **MCP** | Model Context Protocol for agent-to-tool access. |
| **MAST** | Multi-Agent System Failure Taxonomy. |
| **CooperBench** | Benchmark of collaborative coding tasks measuring the coordination deficit. |
| **Context rot** | Degraded model decisions as context grows long. |

---

## 20. Sources

Research gathered on 2026-10-04. Re-verify before relying on specifics; several are secondary summaries.

**ADEs and orchestration**
- Orca overview: https://openalternative.co/orca and https://landscape.jimmysong.io/projects/orca/
- Orca architecture write-up: https://pyshine.com/Orca-Agent-Development-Environment-Parallel-AI-Coding/
- Orca orchestration skill (inter-agent messaging, decomposition, gates): https://www.skills.sh/drolu/agent-skills/orchestration
- BridgeSpace 3: https://www.hunted.space/product/bridgespace-3
- BridgeMind profile (MCP server): https://www.everydev.ai/developers/bridgemind
- ADE (arul28): https://www.everydev.ai/tools/ade/llms.txt
- ADE category explainer: https://superset.sh/compare/what-is-an-agent-development-environment

**Vendor multi-agent features and experiments**
- Claude Code Agent Teams overviews: https://addyosmani.com/blog/claude-code-agent-teams/ and https://www.augmentcode.com/tools/claude-code-agent-teams-vs-intent
- Claude Code Agent Teams guide: https://www.aibuilderclub.com/blog/claude-code-agent-teams-guide
- Cursor, Scaling long-running autonomous coding: https://cursor.com/blog/scaling-agents
- Anthropic, Building a C compiler with a team of parallel Claudes (summarized): https://www.fanaticalfuturist.com/2026/01/16-claude-code-agents-built-a-new-c-compiler-for-20000-from-scratch-in-2-weeks/ and https://futurumgroup.com/insights/truth-or-dare-what-can-claude-agent-teams-and-developers-create-today/
- Anthropic, How we built our multi-agent research system: https://www.anthropic.com/engineering/multi-agent-research-system
- Cognition, Don't Build Multi-Agents: https://cognition.ai/blog/dont-build-multi-agents
- Cognition, Multi-Agents: What's Actually Working: https://cognition.ai/blog/multi-agents-working

**Research papers and benchmarks**
- MAST, Why Do Multi-Agent LLM Systems Fail?: https://arxiv.org/abs/2503.13657
- CooperBench: https://arxiv.org/abs/2601.13295 and https://www.mintlify.com/cooperbench/CooperBench
- LLM blackboard system for data discovery: https://arxiv.org/abs/2510.01285
- Blackboard-based LLM MAS (bMAS): https://arxiv.org/html/2507.01701v1
- Deterministic blackboard pipelines (preprint): https://zenodo.org/records/19068475
- Prompt injection in multi-agent systems (ICML 2026): https://icml.cc/virtual/2026/67971

**Protocols**
- Agent Client Protocol: https://agentclientprotocol.com/overview/agents and https://zed.dev/acp and https://zed.dev/blog/acp-progress-report
- Codex in Zed via ACP: https://zed.dev/blog/codex-is-live-in-zed
- ACP in Microsoft terminal: https://codex.danielvaughan.com/2026/06/10/agent-client-protocol-microsoft-intelligent-terminal-codex-cli-multi-agent-ide-ecosystem/
- A2A, MCP and ACP comparisons: https://about.you.com/resources/a2a-protocol-explained-what-agent-to-agent-communication-solves and https://www.morphllm.com/comparisons/acp-vs-mcp-vs-a2a and https://codex.danielvaughan.com/2026/05/29/agent-to-agent-communication-protocols-a2a-vs-acp-vs-mcp-compared/

**CLI control surfaces and hooks**
- Claude Code headless / Agent SDK: https://code.claude.com/docs/en/headless
- OpenCode server: https://opencode.ai/docs/server/
- Codex and Gemini structured modes via driver libraries: https://docs.rs/crate/gate4agent/latest and https://github.com/dortort/agentry
- CLI conformance suite: https://pkg.go.dev/github.com/belt-sh/harness-test
- Hooks (context injection, forced continuation): https://github.com/monks1975/opencode-agent-hooks and https://docs.devin.ai/cli/extensibility/hooks/overview.md

**Terminal control prior art (Rust)**
- `fux` / `zor` wrapper design: https://docs.rs/crate/fux/0.2.0/source/docs/wrapper-design.md
- `fno-agents`: https://docs.rs/crate/fno-agents/latest/source/src/screen.rs
- `multistack`: https://docs.rs/crate/multistack/1.0.0-rc6/source/ARCHITECTURE.md

**Workspace isolation and integration**
- Worktrees and integration gates: https://dzone.com/articles/two-ai-coding-agents
- Merge conflicts with parallel agents: https://braindetox.kr/en/posts/multiple_ai_agents_one_repo_2026.html
- Conflict prediction: https://codeongrass.com/blog/parallel-worktrees-conflict-prediction/
- Stale-base false conflicts: https://github.com/lwgray/marcus/pull/656

**Security**
- Threat model and defenses: https://arxiv.org/pdf/2609.22949
- Boundary guardrails: https://www.arthur.ai/column/prompt-injection-multi-agent-systems

---

## Appendix A. Confidence notes

- High confidence: the existence of coordination failures (F1 to F4), the orchestrator-worker cost profile (F5), worktree isolation practice (F9), and the existence of structured CLI channels (F10).
- Medium confidence: exact CLI feature behavior (hooks per mode, ACP maturity per CLI), because documentation is changing quickly and some sources are third-party summaries.
- Lower confidence: blackboard results (non-coding domains, F8), the security study numbers (one system, F12), and all performance hypotheses in Section 12.
- Not researched in depth: Windows-specific PTY issues, A2A bridging details, licensing landscape, and "Nexa."

## Appendix B. Suggested first questions for the implementing agent to ask the owner

1. Which four CLIs first, and can you provide a link for Nexa?
2. Confirm P1 (local-first, no central server) and the planner/worker hierarchy (Q2, Q3).
3. Preferred license.
4. Should it reuse existing Rust transport crates or own its adapters?
5. Which success criteria define "smarter" and "faster" for the first public claim?
