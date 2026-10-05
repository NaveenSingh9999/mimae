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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveReport {
    pub rounds: u32,
    pub progress: GoalProgress,
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
    let turn_ok = events
        .iter()
        .any(|event| matches!(event, SessionEvent::TurnComplete { ok: true }));
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
