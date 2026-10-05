use crate::actor::Actor;
use crate::coordinator::{Coordinator, CoreError};
use crate::entities::{
    Capability, Contract, ContractStatus, Decision, Goal, Lease, Task, TaskStatus, Verification,
    WriteScope,
};
use crate::event::{Event, EventPayload, KnownPayload};
use crate::ids::{AgentId, ContractId, GoalId, TaskId};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct PlanTask {
    pub id: TaskId,
    pub title: String,
    pub brief: String,
    pub capability: Capability,
    pub write_scope: WriteScope,
    pub deps: Vec<TaskId>,
    pub verification: Verification,
    pub interfaces: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PlanContract {
    pub id: ContractId,
    pub parties: Vec<TaskId>,
    pub content: String,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanReceipt {
    pub goal_seq: u64,
    pub task_seqs: Vec<(TaskId, u64)>,
    pub contract_seqs: Vec<(ContractId, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalProgress {
    pub total: usize,
    pub done: usize,
    pub failed: usize,
}

impl GoalProgress {
    pub fn is_complete(&self) -> bool {
        self.total > 0 && self.done == self.total
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("goal text must not be empty")]
    EmptyGoal,
    #[error("duplicate task id in plan: {0}")]
    DuplicateTask(TaskId),
    #[error("task {0} already exists")]
    TaskExists(TaskId),
    #[error("task {task} depends on unknown task {dep}")]
    UnknownTask { task: TaskId, dep: TaskId },
    #[error("dependency cycle detected: {0:?}")]
    DependencyCycle(Vec<TaskId>),
    #[error("tasks {a} and {b} can run in parallel but both write {path}")]
    OverlappingScopes { a: TaskId, b: TaskId, path: String },
    #[error("parallel tasks share interface {interface} with no covering contract")]
    MissingContract { interface: String },
    #[error("duplicate contract id in plan: {0}")]
    DuplicateContract(ContractId),
    #[error("contract {0} already exists")]
    ContractExists(ContractId),
    #[error("contract {contract} references unknown task {party}")]
    UnknownContractParty { contract: ContractId, party: TaskId },
    #[error("contract {0} has invalid version {1}")]
    InvalidContractVersion(ContractId, u32),
    #[error("agent {0} not found")]
    NoSuchAgent(AgentId),
    #[error("no ready task matches agent {0}")]
    NoReadyTask(AgentId),
    #[error("core error: {0}")]
    Core(#[from] CoreError),
}

fn normalize_scope_path(path: &str) -> String {
    let mut out = path.trim().trim_end_matches('/').to_string();
    for suffix in ["/**", "/*"] {
        if let Some(stripped) = out.strip_suffix(suffix) {
            out = stripped.trim_end_matches('/').to_string();
        }
    }
    out
}

fn scopes_overlap(a: &WriteScope, b: &WriteScope) -> Option<String> {
    for pa in &a.paths {
        for pb in &b.paths {
            let na = normalize_scope_path(pa);
            let nb = normalize_scope_path(pb);
            if na == nb || na.starts_with(&format!("{nb}/")) || nb.starts_with(&format!("{na}/")) {
                return Some(pa.clone());
            }
        }
    }
    None
}

fn build_adjacency(
    plan_tasks: &[PlanTask],
    graph: &crate::graph::TaskGraph,
) -> HashMap<TaskId, Vec<TaskId>> {
    let mut adj: HashMap<TaskId, Vec<TaskId>> = HashMap::new();
    for task in &graph.tasks {
        adj.entry(*task.0)
            .or_default()
            .extend(task.1.deps.iter().copied());
    }
    for spec in plan_tasks {
        adj.insert(spec.id, spec.deps.clone());
    }
    let missing: Vec<TaskId> = adj
        .values()
        .flatten()
        .copied()
        .filter(|dep| !adj.contains_key(dep))
        .collect();
    for dep in missing {
        adj.entry(dep).or_default();
    }
    adj
}

fn find_cycle(adj: &HashMap<TaskId, Vec<TaskId>>) -> Option<Vec<TaskId>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }
    let mut color: HashMap<TaskId, Color> = adj.keys().map(|id| (*id, Color::White)).collect();
    let mut stack: Vec<TaskId> = Vec::new();

    fn visit(
        node: TaskId,
        adj: &HashMap<TaskId, Vec<TaskId>>,
        color: &mut HashMap<TaskId, Color>,
        stack: &mut Vec<TaskId>,
    ) -> Option<Vec<TaskId>> {
        color.insert(node, Color::Gray);
        stack.push(node);
        if let Some(deps) = adj.get(&node) {
            for dep in deps {
                match color.get(dep).copied().unwrap_or(Color::Black) {
                    Color::Gray => {
                        let start = stack.iter().position(|id| id == dep).unwrap_or(0);
                        let mut cycle: Vec<TaskId> = stack[start..].to_vec();
                        cycle.push(*dep);
                        return Some(cycle);
                    }
                    Color::White => {
                        if let Some(cycle) = visit(*dep, adj, color, stack) {
                            return Some(cycle);
                        }
                    }
                    Color::Black => {}
                }
            }
        }
        stack.pop();
        color.insert(node, Color::Black);
        None
    }

    let mut ids: Vec<TaskId> = adj.keys().copied().collect();
    ids.sort_by_key(|id| id.get());
    for id in ids {
        let needs_visit = color.get(&id).copied().unwrap_or(Color::Black) == Color::White;
        if needs_visit && let Some(cycle) = visit(id, adj, &mut color, &mut stack) {
            return Some(cycle);
        }
    }
    None
}

fn reaches(adj: &HashMap<TaskId, Vec<TaskId>>, from: TaskId, to: TaskId) -> bool {
    if from == to {
        return true;
    }
    let mut seen: HashSet<TaskId> = HashSet::new();
    let mut stack = vec![from];
    while let Some(node) = stack.pop() {
        if node == to {
            return true;
        }
        if seen.insert(node)
            && let Some(deps) = adj.get(&node)
        {
            stack.extend(deps.iter().copied());
        }
    }
    false
}

fn parallel(adj: &HashMap<TaskId, Vec<TaskId>>, a: TaskId, b: TaskId) -> bool {
    a != b && !reaches(adj, a, b) && !reaches(adj, b, a)
}

fn topo_order(specs: &[PlanTask]) -> Vec<usize> {
    let index: HashMap<TaskId, usize> = specs.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
    let mut indegree: Vec<usize> = vec![0; specs.len()];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); specs.len()];
    for (i, spec) in specs.iter().enumerate() {
        for dep in &spec.deps {
            if let Some(&j) = index.get(dep) {
                indegree[i] += 1;
                dependents[j].push(i);
            }
        }
    }
    let mut queue: Vec<usize> = indegree
        .iter()
        .enumerate()
        .filter_map(|(i, &d)| (d == 0).then_some(i))
        .collect();
    let mut order = Vec::with_capacity(specs.len());
    while let Some(i) = queue.pop() {
        order.push(i);
        for &next in &dependents[i] {
            indegree[next] -= 1;
            if indegree[next] == 0 {
                queue.push(next);
            }
        }
    }
    order
}

fn active_status(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Proposed
            | TaskStatus::Ready
            | TaskStatus::Claimed
            | TaskStatus::InProgress
            | TaskStatus::InReview
            | TaskStatus::Integrating
            | TaskStatus::Blocked
    )
}

pub fn plan_goal(
    coordinator: &mut Coordinator,
    goal: Goal,
    tasks: Vec<PlanTask>,
    contracts: Vec<PlanContract>,
    at_ms: u64,
) -> Result<PlanReceipt, PlanError> {
    if goal.text.trim().is_empty() {
        return Err(PlanError::EmptyGoal);
    }
    let graph = coordinator.graph();
    let mut seen_tasks: HashSet<TaskId> = HashSet::new();
    for spec in &tasks {
        if !seen_tasks.insert(spec.id) {
            return Err(PlanError::DuplicateTask(spec.id));
        }
        if graph.tasks.contains_key(&spec.id) {
            return Err(PlanError::TaskExists(spec.id));
        }
    }
    let mut seen_contracts: HashSet<ContractId> = HashSet::new();
    for spec in &contracts {
        if !seen_contracts.insert(spec.id) {
            return Err(PlanError::DuplicateContract(spec.id));
        }
        if graph.contracts.contains_key(&spec.id) {
            return Err(PlanError::ContractExists(spec.id));
        }
        if spec.version < 1 {
            return Err(PlanError::InvalidContractVersion(spec.id, spec.version));
        }
    }
    let known: HashSet<TaskId> = graph
        .tasks
        .keys()
        .copied()
        .chain(tasks.iter().map(|spec| spec.id))
        .collect();
    for spec in &tasks {
        for dep in &spec.deps {
            if !known.contains(dep) {
                return Err(PlanError::UnknownTask {
                    task: spec.id,
                    dep: *dep,
                });
            }
        }
    }
    for spec in &contracts {
        for party in &spec.parties {
            if !known.contains(party) {
                return Err(PlanError::UnknownContractParty {
                    contract: spec.id,
                    party: *party,
                });
            }
        }
    }
    let adj = build_adjacency(&tasks, graph);
    if let Some(cycle) = find_cycle(&adj) {
        return Err(PlanError::DependencyCycle(cycle));
    }
    let active_graph_tasks: Vec<&Task> = graph
        .tasks
        .values()
        .filter(|task| active_status(&task.status))
        .collect();
    for (i, a) in tasks.iter().enumerate() {
        for b in &tasks[i + 1..] {
            if parallel(&adj, a.id, b.id)
                && let Some(path) = scopes_overlap(&a.write_scope, &b.write_scope)
            {
                return Err(PlanError::OverlappingScopes {
                    a: a.id,
                    b: b.id,
                    path,
                });
            }
        }
        for existing in &active_graph_tasks {
            if parallel(&adj, a.id, existing.id)
                && let Some(path) = scopes_overlap(&a.write_scope, &existing.write_scope)
            {
                return Err(PlanError::OverlappingScopes {
                    a: a.id,
                    b: existing.id,
                    path,
                });
            }
        }
    }
    let mut by_interface: HashMap<&str, Vec<TaskId>> = HashMap::new();
    for spec in &tasks {
        for interface in &spec.interfaces {
            by_interface
                .entry(interface.as_str())
                .or_default()
                .push(spec.id);
        }
    }
    for (interface, members) in &by_interface {
        let mut needs_contract = false;
        'pairs: for (i, a) in members.iter().enumerate() {
            for b in &members[i + 1..] {
                if parallel(&adj, *a, *b) {
                    needs_contract = true;
                    break 'pairs;
                }
            }
        }
        if !needs_contract {
            continue;
        }
        let covered = contracts
            .iter()
            .any(|spec| members.iter().all(|member| spec.parties.contains(member)));
        if !covered {
            return Err(PlanError::MissingContract {
                interface: (*interface).to_string(),
            });
        }
    }

    let mut causes: Vec<u64> = Vec::new();
    let goal_event = coordinator.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::GoalSubmitted { goal: goal.clone() }),
        causes.clone(),
        at_ms,
    )?;
    causes = vec![goal_event.seq];
    let mut receipt = PlanReceipt {
        goal_seq: goal_event.seq,
        task_seqs: Vec::with_capacity(tasks.len()),
        contract_seqs: Vec::with_capacity(contracts.len()),
    };
    for i in topo_order(&tasks) {
        let spec = &tasks[i];
        let event = coordinator.publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TaskCreated {
                task: Task {
                    id: spec.id,
                    goal_id: goal.id,
                    title: spec.title.clone(),
                    brief: spec.brief.clone(),
                    capability: spec.capability.clone(),
                    write_scope: spec.write_scope.clone(),
                    deps: spec.deps.clone(),
                    status: TaskStatus::Proposed,
                    owner: None,
                    lease: None,
                    verification: spec.verification,
                },
            }),
            causes.clone(),
            at_ms,
        )?;
        causes = vec![event.seq];
        receipt.task_seqs.push((spec.id, event.seq));
    }
    for spec in &contracts {
        let event = coordinator.publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::ContractProposed {
                contract: Contract {
                    id: spec.id,
                    parties: spec.parties.clone(),
                    content: spec.content.clone(),
                    version: spec.version,
                    status: ContractStatus::Proposed,
                },
            }),
            causes.clone(),
            at_ms,
        )?;
        causes = vec![event.seq];
        receipt.contract_seqs.push((spec.id, event.seq));
    }
    Ok(receipt)
}

pub fn propose_contract(
    coordinator: &mut Coordinator,
    contract: Contract,
    at_ms: u64,
) -> Result<Event, PlanError> {
    if coordinator.graph().contracts.contains_key(&contract.id) {
        return Err(PlanError::ContractExists(contract.id));
    }
    if contract.version < 1 {
        return Err(PlanError::InvalidContractVersion(
            contract.id,
            contract.version,
        ));
    }
    for party in &contract.parties {
        if !coordinator.graph().tasks.contains_key(party) {
            return Err(PlanError::UnknownContractParty {
                contract: contract.id,
                party: *party,
            });
        }
    }
    Ok(coordinator.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::ContractProposed { contract }),
        vec![],
        at_ms,
    )?)
}

pub fn accept_contract(
    coordinator: &mut Coordinator,
    contract_id: ContractId,
    version: u32,
    at_ms: u64,
) -> Result<Event, PlanError> {
    Ok(coordinator.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::ContractAccepted {
            contract_id,
            version,
        }),
        vec![],
        at_ms,
    )?)
}

pub fn change_contract(
    coordinator: &mut Coordinator,
    contract: Contract,
    at_ms: u64,
) -> Result<Event, PlanError> {
    Ok(coordinator.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::ContractChanged { contract }),
        vec![],
        at_ms,
    )?)
}

pub fn record_decision(
    coordinator: &mut Coordinator,
    decision: Decision,
    at_ms: u64,
) -> Result<Event, PlanError> {
    Ok(coordinator.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::DecisionRecorded { decision }),
        vec![],
        at_ms,
    )?)
}

pub fn next_assignment(
    coordinator: &Coordinator,
    agent_id: AgentId,
) -> Result<Option<TaskId>, PlanError> {
    let graph = coordinator.graph();
    let agent = graph
        .agents
        .get(&agent_id)
        .ok_or(PlanError::NoSuchAgent(agent_id))?;
    for task_id in graph.ready() {
        let Some(task) = graph.tasks.get(&task_id) else {
            continue;
        };
        if agent.capabilities.contains(&task.capability) {
            return Ok(Some(task_id));
        }
    }
    Ok(None)
}

pub fn assign(
    coordinator: &mut Coordinator,
    agent_id: AgentId,
    now_ms: u64,
    ttl_ms: u64,
) -> Result<(TaskId, Lease), PlanError> {
    let Some(task_id) = next_assignment(coordinator, agent_id)? else {
        return Err(PlanError::NoReadyTask(agent_id));
    };
    let scope = coordinator
        .graph()
        .tasks
        .get(&task_id)
        .map(|task| task.write_scope.clone())
        .unwrap_or_default();
    Ok((
        task_id,
        coordinator.claim(task_id, agent_id, scope, now_ms, ttl_ms)?,
    ))
}

pub fn goal_progress(coordinator: &Coordinator, goal_id: GoalId) -> GoalProgress {
    let mut progress = GoalProgress {
        total: 0,
        done: 0,
        failed: 0,
    };
    for task in coordinator.graph().tasks.values() {
        if task.goal_id != goal_id {
            continue;
        }
        progress.total += 1;
        match task.status {
            TaskStatus::Done => progress.done += 1,
            TaskStatus::Failed => progress.failed += 1,
            _ => {}
        }
    }
    progress
}
