use crate::actor::Actor;
use crate::adapter::{AgentAdapter, SessionEvent};
use crate::coordinator::{Coordinator, CoreError};
use crate::entities::TaskStatus;
use crate::event::{EventPayload, KnownPayload};
use crate::ids::{AgentId, GoalId, TaskId};
use crate::planner::{GoalProgress, PlanError, assign, goal_progress, next_assignment};

#[derive(Debug, thiserror::Error)]
pub enum DriveError {
    #[error("core error: {0}")]
    Core(#[from] CoreError),
    #[error("plan error: {0}")]
    Plan(#[from] PlanError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnReport {
    pub agent: AgentId,
    pub assigned: Option<TaskId>,
    pub turn_ok: bool,
    pub completed: bool,
    pub tokens_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveReport {
    pub rounds: u32,
    pub progress: GoalProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadConfig {
    pub max_rounds: u32,
    pub stuck_threshold: u32,
    pub token_budget_per_agent: Option<u64>,
}

impl Default for HeadConfig {
    fn default() -> Self {
        Self {
            max_rounds: 10,
            stuck_threshold: 3,
            token_budget_per_agent: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadReport {
    pub rounds: u32,
    pub progress: GoalProgress,
    pub over_budget: Vec<AgentId>,
    pub stalled: Vec<AgentId>,
    pub failed_tasks: Vec<TaskId>,
}

fn advance(
    coordinator: &mut Coordinator,
    task_id: TaskId,
    from: TaskStatus,
    to: TaskStatus,
    at_ms: u64,
) -> Result<(), DriveError> {
    coordinator.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::TaskStatusChanged {
            task_id,
            from,
            to,
            reason: None,
        }),
        vec![],
        at_ms,
    )?;
    Ok(())
}

pub fn drive_agent_turn(
    coordinator: &mut Coordinator,
    agent_id: AgentId,
    adapter: &mut dyn AgentAdapter,
    now_ms: u64,
    ttl_ms: u64,
) -> Result<TurnReport, DriveError> {
    coordinator.check_for_expired_leases(now_ms)?;
    let Some(task_id) = next_assignment(coordinator, agent_id)? else {
        return Ok(TurnReport {
            agent: agent_id,
            assigned: None,
            turn_ok: false,
            completed: false,
            tokens_used: 0,
        });
    };
    let (claimed, _) = assign(coordinator, agent_id, now_ms, ttl_ms)?;
    debug_assert_eq!(claimed, task_id);
    let brief = coordinator
        .graph()
        .tasks
        .get(&task_id)
        .map(|task| task.brief.clone())
        .unwrap_or_default();
    let events = adapter.send_turn(&brief);
    let mut tokens_used = 0u64;
    let mut turn_ok = false;
    for event in &events {
        match event {
            SessionEvent::Usage {
                input_tokens,
                output_tokens,
            } => {
                tokens_used =
                    tokens_used.saturating_add(input_tokens.saturating_add(*output_tokens));
            }
            SessionEvent::TurnComplete { ok: true } => turn_ok = true,
            _ => {}
        }
    }
    if turn_ok {
        advance(
            coordinator,
            task_id,
            TaskStatus::Claimed,
            TaskStatus::InProgress,
            now_ms,
        )?;
        advance(
            coordinator,
            task_id,
            TaskStatus::InProgress,
            TaskStatus::InReview,
            now_ms,
        )?;
        advance(
            coordinator,
            task_id,
            TaskStatus::InReview,
            TaskStatus::Integrating,
            now_ms,
        )?;
        advance(
            coordinator,
            task_id,
            TaskStatus::Integrating,
            TaskStatus::Done,
            now_ms,
        )?;
    } else {
        advance(
            coordinator,
            task_id,
            TaskStatus::Claimed,
            TaskStatus::Failed,
            now_ms,
        )?;
    }
    Ok(TurnReport {
        agent: agent_id,
        assigned: Some(task_id),
        turn_ok,
        completed: turn_ok,
        tokens_used,
    })
}

pub fn drive_goal(
    coordinator: &mut Coordinator,
    agents: &mut [(AgentId, Box<dyn AgentAdapter>)],
    goal_id: GoalId,
    now_ms: u64,
    ttl_ms: u64,
    max_rounds: u32,
) -> Result<DriveReport, DriveError> {
    let mut rounds = 0;
    while rounds < max_rounds {
        rounds += 1;
        for (agent_id, adapter) in agents.iter_mut() {
            drive_agent_turn(coordinator, *agent_id, adapter.as_mut(), now_ms, ttl_ms)?;
        }
        if goal_progress(coordinator, goal_id).is_complete() {
            break;
        }
    }
    Ok(DriveReport {
        rounds,
        progress: goal_progress(coordinator, goal_id),
    })
}

pub fn drive_head(
    coordinator: &mut Coordinator,
    agents: &mut [(AgentId, Box<dyn AgentAdapter>)],
    goal_id: GoalId,
    now_ms: u64,
    ttl_ms: u64,
    config: HeadConfig,
) -> Result<HeadReport, DriveError> {
    use std::collections::HashMap;
    let mut spent: HashMap<AgentId, u64> = HashMap::new();
    let mut idle_streak: HashMap<AgentId, u32> = HashMap::new();
    let mut over_budget: Vec<AgentId> = Vec::new();
    let mut rounds = 0;
    let mut idle_rounds = 0u32;
    let idle_limit = config.stuck_threshold.max(1);
    while rounds < config.max_rounds {
        rounds += 1;
        let mut assigned_this_round = false;
        for (agent_id, adapter) in agents.iter_mut() {
            if over_budget.contains(agent_id) {
                continue;
            }
            let report =
                drive_agent_turn(coordinator, *agent_id, adapter.as_mut(), now_ms, ttl_ms)?;
            if let Some(budget) = config.token_budget_per_agent {
                let total = spent.entry(*agent_id).or_insert(0);
                *total = total.saturating_add(report.tokens_used);
                if *total > budget && !over_budget.contains(agent_id) {
                    over_budget.push(*agent_id);
                }
            }
            if report.assigned.is_some() {
                assigned_this_round = true;
                idle_streak.insert(*agent_id, 0);
            } else {
                *idle_streak.entry(*agent_id).or_insert(0) += 1;
            }
        }
        if goal_progress(coordinator, goal_id).is_complete() {
            break;
        }
        if assigned_this_round {
            idle_rounds = 0;
        } else {
            idle_rounds += 1;
            if idle_rounds >= idle_limit {
                break;
            }
        }
    }
    let threshold = config.stuck_threshold.max(1);
    let mut stalled: Vec<AgentId> = idle_streak
        .iter()
        .filter_map(|(id, streak)| (*streak >= threshold).then_some(*id))
        .collect();
    stalled.sort_by_key(|id| id.get());
    over_budget.sort_by_key(|id| id.get());
    let mut failed_tasks: Vec<TaskId> = coordinator
        .graph()
        .tasks
        .values()
        .filter(|task| task.goal_id == goal_id && task.status == TaskStatus::Failed)
        .map(|task| task.id)
        .collect();
    failed_tasks.sort_by_key(|id| id.get());
    Ok(HeadReport {
        rounds,
        progress: goal_progress(coordinator, goal_id),
        over_budget,
        stalled,
        failed_tasks,
    })
}
