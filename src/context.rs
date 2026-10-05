use crate::entities::{ContractStatus, TaskStatus};
use crate::graph::TaskGraph;
use crate::ids::{AgentId, TaskId};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContextError {
    #[error("agent {0} not found")]
    UnknownAgent(AgentId),
    #[error("task {0} not found")]
    UnknownTask(TaskId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractBrief {
    pub id: String,
    pub version: u32,
    pub status: ContractStatus,
    pub content: String,
    pub parties: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionBrief {
    pub id: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerBrief {
    pub agent: String,
    pub task: String,
    pub capability: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brief {
    pub agent: String,
    pub task: String,
    pub brief_text: String,
    pub scope_paths: Vec<String>,
    pub acceptance: Vec<String>,
    pub contracts: Vec<ContractBrief>,
    pub decisions: Vec<DecisionBrief>,
    pub peers: Vec<PeerBrief>,
}

impl Brief {
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("agent {}\n", self.agent));
        out.push_str(&format!("task {}: {}\n", self.task, self.brief_text));
        out.push_str(&format!("scope: {}\n", self.scope_paths.join(", ")));
        out.push_str("acceptance:\n");
        for criterion in &self.acceptance {
            out.push_str(&format!("- {criterion}\n"));
        }
        out.push_str("contracts:\n");
        for contract in &self.contracts {
            out.push_str(&format!(
                "- {} v{} ({:?}): {}\n",
                contract.id, contract.version, contract.status, contract.content
            ));
        }
        out.push_str("decisions:\n");
        for decision in &self.decisions {
            out.push_str(&format!("- {}: {}\n", decision.id, decision.rationale));
        }
        out.push_str("peers:\n");
        for peer in &self.peers {
            out.push_str(&format!(
                "- agent {} owns task {} ({}, {:?})\n",
                peer.agent, peer.task, peer.capability, peer.status
            ));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewContext {
    pub task: String,
    pub spec: String,
    pub diff: String,
}

impl ReviewContext {
    pub fn render(&self) -> String {
        format!(
            "review task {}\nspec:\n{}\ndiff:\n{}\n",
            self.task, self.spec, self.diff
        )
    }
}

pub fn build_brief(
    graph: &TaskGraph,
    agent_id: AgentId,
    task_id: TaskId,
) -> Result<Brief, ContextError> {
    if !graph.agents.contains_key(&agent_id) {
        return Err(ContextError::UnknownAgent(agent_id));
    }
    let task = graph
        .tasks
        .get(&task_id)
        .ok_or(ContextError::UnknownTask(task_id))?;
    let acceptance = graph
        .goals
        .get(&task.goal_id)
        .map(|goal| goal.acceptance.clone())
        .unwrap_or_default();
    let mut contracts: Vec<ContractBrief> = graph
        .contracts
        .values()
        .filter(|contract| contract.parties.contains(&task_id))
        .map(|contract| ContractBrief {
            id: format!("contract({})", contract.id.get()),
            version: contract.version,
            status: contract.status,
            content: contract.content.clone(),
            parties: contract
                .parties
                .iter()
                .map(|party| format!("task({})", party.get()))
                .collect(),
        })
        .collect();
    contracts.sort_by(|a, b| a.id.cmp(&b.id));
    let mut decisions: Vec<DecisionBrief> = graph
        .decisions
        .iter()
        .map(|decision| DecisionBrief {
            id: format!("decision({})", decision.id.get()),
            rationale: decision.rationale.clone(),
        })
        .collect();
    decisions.sort_by(|a, b| a.id.cmp(&b.id));
    let mut peers: BTreeMap<String, PeerBrief> = BTreeMap::new();
    for peer_task in graph.tasks.values() {
        let Some(owner) = peer_task.owner else {
            continue;
        };
        if owner == agent_id {
            continue;
        }
        peers.insert(
            format!("agent({})-task({})", owner.get(), peer_task.id.get()),
            PeerBrief {
                agent: format!("agent({})", owner.get()),
                task: format!("task({})", peer_task.id.get()),
                capability: peer_task.capability.0.clone(),
                status: peer_task.status.clone(),
            },
        );
    }
    Ok(Brief {
        agent: format!("agent({})", agent_id.get()),
        task: format!("task({})", task_id.get()),
        brief_text: task.brief.clone(),
        scope_paths: task.write_scope.paths.clone(),
        acceptance,
        contracts,
        decisions,
        peers: peers.into_values().collect(),
    })
}

pub fn build_review_context(
    graph: &TaskGraph,
    task_id: TaskId,
    diff: &str,
) -> Result<ReviewContext, ContextError> {
    let task = graph
        .tasks
        .get(&task_id)
        .ok_or(ContextError::UnknownTask(task_id))?;
    let mut spec = task.brief.clone();
    if let Some(goal) = graph.goals.get(&task.goal_id) {
        spec.push_str("\nacceptance:\n");
        for criterion in &goal.acceptance {
            spec.push_str(&format!("- {criterion}\n"));
        }
    }
    Ok(ReviewContext {
        task: format!("task({})", task_id.get()),
        spec,
        diff: diff.to_string(),
    })
}
