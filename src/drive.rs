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
    /// True when a cost cap stopped the turn (pre-turn skip or post-turn
    /// budget failure). The task ends `Failed` with reason
    /// [`BUDGET_EXCEEDED_REASON`], which the eval harness tags as
    /// runaway cost.
    pub budget_breached: bool,
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
    /// Hard cost kill-switch (durable, cross-run). Unlike
    /// `token_budget_per_agent` — a soft per-run skip — a breach fails the
    /// task with [`BUDGET_EXCEEDED_REASON`].
    pub cost_policy: CostPolicy,
}

impl Default for HeadConfig {
    fn default() -> Self {
        Self {
            max_rounds: 10,
            stuck_threshold: 3,
            token_budget_per_agent: None,
            cost_policy: CostPolicy::unlimited(),
        }
    }
}

/// Kill-switch reason recorded on tasks failed by [`CostPolicy`]
/// enforcement. The eval harness tags exactly this reason as runaway cost.
pub const BUDGET_EXCEEDED_REASON: &str = "budget-exceeded";

/// Reason recorded when the mid-turn watchdog interrupts a turn that ran
/// past `turn_wall_max_ms`. Counted as a plain task failure (no MAST tag).
pub const TURN_TIMEOUT_REASON: &str = "turn-timeout";

/// Hard token-spend caps enforced by [`drive_agent_turn`]. Any cap set to
/// `None` is not enforced. All spend is read from the folded log
/// (`TaskGraph::cost_by_agent` / `cost_by_task`), so caps hold across
/// restarts — unlike the in-memory per-run budget in [`HeadConfig`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CostPolicy {
    /// Max tokens for a single turn. A turn that succeeds but costs more is
    /// anomalous: its task fails so the work is re-done under review.
    pub per_turn_tokens_max: Option<u64>,
    /// Max cumulative tokens charged to one task.
    pub per_task_tokens_max: Option<u64>,
    /// Max cumulative tokens charged to one agent. An over-budget agent is
    /// skipped before claiming; a turn that pushes it over fails its task.
    pub per_agent_tokens_max: Option<u64>,
    /// Max cumulative tokens across all tasks of one goal. When unset, the
    /// goal's own `Budget::max_tokens` applies if it is non-zero. This is
    /// the brief §9.8 "runaway cost" per-goal leg.
    pub per_goal_tokens_max: Option<u64>,
    /// Mid-turn sample cadence in milliseconds. When set, the turn runs on
    /// a scoped worker thread while the drive loop samples
    /// [`AgentAdapter::sample_usage`] every tick, publishes a
    /// `UsageSampled` event per tick (the ADE usage stream — subscribe via
    /// `events_since`), and interrupts the turn when the sampled total
    /// breaches `per_turn_tokens_max`. `None` (default) keeps turns
    /// single-threaded with post-turn accounting only.
    pub sample_every_ms: Option<u64>,
    /// Mid-turn wall-clock cap in milliseconds. A turn running past it is
    /// interrupted and failed with [`TURN_TIMEOUT_REASON`]. Requires the
    /// sample loop, so it only applies when `sample_every_ms` is set.
    /// `interrupt()` is cooperative except for process adapters (which
    /// kill the child, D19): an adapter that ignores it fails only once
    /// `send_turn` returns.
    pub turn_wall_max_ms: Option<u64>,
}

impl CostPolicy {
    pub fn unlimited() -> Self {
        Self {
            per_turn_tokens_max: None,
            per_task_tokens_max: None,
            per_agent_tokens_max: None,
            per_goal_tokens_max: None,
            sample_every_ms: None,
            turn_wall_max_ms: None,
        }
    }

    fn over(&self, cap: Option<u64>, spent: u64) -> bool {
        cap.is_some_and(|max| spent > max)
    }

    fn monitored(&self) -> bool {
        self.sample_every_ms.is_some() || self.turn_wall_max_ms.is_some()
    }
}

/// Total logged spend across every task of `goal_id` (the per-goal cap
/// view). Sums the durable task ledger — no extra bookkeeping needed.
pub fn goal_spend(coordinator: &Coordinator, goal_id: GoalId) -> u64 {
    let goal_tasks: std::collections::HashSet<TaskId> = coordinator
        .graph()
        .tasks
        .values()
        .filter(|task| task.goal_id == goal_id)
        .map(|task| task.id)
        .collect();
    coordinator
        .graph()
        .cost_by_task
        .iter()
        .filter(|(task_id, _)| goal_tasks.contains(task_id))
        .map(|(_, tokens)| *tokens)
        .fold(0u64, u64::saturating_add)
}

/// A goal's own budget applies when it asks for a real cap: `0` (the
/// derived default) means "no per-goal cap", and `CostPolicy` overrides the
/// goal when set.
fn effective_goal_cap(
    coordinator: &Coordinator,
    goal_id: GoalId,
    policy: &CostPolicy,
) -> Option<u64> {
    policy.per_goal_tokens_max.or_else(|| {
        coordinator
            .graph()
            .goals
            .get(&goal_id)
            .map(|goal| goal.budget.max_tokens)
            .filter(|cap| *cap > 0)
    })
}

/// How a monitored turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnBreach {
    /// Sampled usage passed the per-turn cap; interrupted mid-flight.
    Budget,
    /// Wall clock passed `turn_wall_max_ms`; interrupted mid-flight.
    Timeout,
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

fn fail_budget(
    coordinator: &mut Coordinator,
    task_id: TaskId,
    at_ms: u64,
) -> Result<(), DriveError> {
    fail_task(coordinator, task_id, Some(BUDGET_EXCEEDED_REASON), at_ms)
}

fn fail_task(
    coordinator: &mut Coordinator,
    task_id: TaskId,
    reason: Option<&str>,
    at_ms: u64,
) -> Result<(), DriveError> {
    coordinator.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::TaskStatusChanged {
            task_id,
            from: TaskStatus::Claimed,
            to: TaskStatus::Failed,
            reason: reason.map(str::to_string),
        }),
        vec![],
        at_ms,
    )?;
    Ok(())
}

fn record_cost(
    coordinator: &mut Coordinator,
    agent_id: AgentId,
    task_id: TaskId,
    tokens_used: u64,
    at_ms: u64,
) -> Result<(), DriveError> {
    // Spend is durable: folded into cost_by_agent/cost_by_task on replay,
    // so caps hold across restarts and eval can audit every token.
    coordinator.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::CostRecorded {
            agent_id,
            task_id: Some(task_id),
            input_tokens: tokens_used,
            output_tokens: 0,
        }),
        vec![],
        at_ms,
    )?;
    Ok(())
}

fn elapsed_ms(started: std::time::Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

/// Run one turn, optionally watched. With monitoring enabled the turn runs
/// on a scoped worker thread (joined before return — never detached, same
/// discipline as `run_bounded_turn`) while this thread samples usage every
/// tick, publishes `UsageSampled` events for ADE subscribers, and calls
/// `interrupt()` on budget breach or wall-clock expiry. A panicking adapter
/// fails the turn instead of the drive loop.
#[allow(clippy::too_many_arguments)]
fn run_monitored_turn(
    coordinator: &mut Coordinator,
    adapter: &mut (dyn AgentAdapter + Send),
    agent_id: AgentId,
    task_id: TaskId,
    brief: &str,
    now_ms: u64,
    policy: &CostPolicy,
) -> Result<(Vec<SessionEvent>, Option<TurnBreach>), DriveError> {
    if !policy.monitored() {
        return Ok((adapter.send_turn(brief), None));
    }
    let tick_ms = policy.sample_every_ms.unwrap_or(u64::MAX).max(1);
    let started = std::time::Instant::now();
    // The control plane is a separate object sharing state with the turn
    // through atomics: the worker owns `send_turn`, the ticker owns the
    // handle, and no mutex ever spans the blocking call (a Mutex around the
    // adapter would deadlock the ticker for the whole turn).
    let control = adapter.control_handle();
    let mut breach = None;
    let events = std::thread::scope(|s| {
        let worker = s.spawn(|| adapter.send_turn(brief));
        let mut next_sample_ms = tick_ms;
        loop {
            if worker.is_finished() {
                break;
            }
            // Wake for the next sample, the wall deadline, or a finish
            // check — whichever comes first (50ms slices at most).
            let mut wait_ms = 50u64;
            if policy.sample_every_ms.is_some() {
                wait_ms = wait_ms.min(next_sample_ms.saturating_sub(elapsed_ms(started)));
            }
            if let Some(max) = policy.turn_wall_max_ms {
                wait_ms = wait_ms.min(max.saturating_sub(elapsed_ms(started)));
            }
            std::thread::sleep(std::time::Duration::from_millis(wait_ms.max(1)));
            if worker.is_finished() {
                break;
            }
            let at_elapsed_ms = elapsed_ms(started);
            if policy.sample_every_ms.is_some() && at_elapsed_ms >= next_sample_ms {
                next_sample_ms = at_elapsed_ms.saturating_add(tick_ms);
                // A turn that finished between the two checks yields at most
                // one extra sample; interrupting it is a harmless no-op.
                let sampled = control.sample_usage();
                coordinator.publish(
                    Actor::Coordinator,
                    EventPayload::Known(KnownPayload::UsageSampled {
                        agent_id,
                        task_id: Some(task_id),
                        elapsed_ms: at_elapsed_ms,
                        tokens_so_far: sampled,
                    }),
                    vec![],
                    now_ms,
                )?;
                if sampled.is_some_and(|total| policy.over(policy.per_turn_tokens_max, total)) {
                    control.interrupt();
                    breach = Some(TurnBreach::Budget);
                    break;
                }
            }
            if policy
                .turn_wall_max_ms
                .is_some_and(|max| at_elapsed_ms > max)
            {
                control.interrupt();
                breach = Some(TurnBreach::Timeout);
                break;
            }
        }
        Ok::<_, DriveError>(worker.join().unwrap_or_else(|_| {
            vec![
                SessionEvent::Error {
                    message: "adapter turn panicked".into(),
                },
                SessionEvent::TurnComplete { ok: false },
            ]
        }))
    })?;
    Ok((events, breach))
}

pub fn drive_agent_turn(
    coordinator: &mut Coordinator,
    agent_id: AgentId,
    adapter: &mut (dyn AgentAdapter + Send),
    now_ms: u64,
    ttl_ms: u64,
    policy: &CostPolicy,
) -> Result<TurnReport, DriveError> {
    coordinator.check_for_expired_leases(now_ms)?;
    // Pre-turn kill-switch: an agent already over its lifetime cap claims
    // nothing. Reported as a breach with no assignment (not idle work).
    if policy.over(
        policy.per_agent_tokens_max,
        coordinator
            .graph()
            .cost_by_agent
            .get(&agent_id)
            .copied()
            .unwrap_or(0),
    ) {
        return Ok(TurnReport {
            agent: agent_id,
            assigned: None,
            turn_ok: false,
            completed: false,
            tokens_used: 0,
            budget_breached: true,
        });
    }
    let Some(task_id) = next_assignment(coordinator, agent_id)? else {
        return Ok(TurnReport {
            agent: agent_id,
            assigned: None,
            turn_ok: false,
            completed: false,
            tokens_used: 0,
            budget_breached: false,
        });
    };
    let (claimed, _) = assign(coordinator, agent_id, now_ms, ttl_ms)?;
    debug_assert_eq!(claimed, task_id);
    // A task already over its cap from prior turns fails now — running
    // another turn would only burn more tokens.
    if policy.over(
        policy.per_task_tokens_max,
        coordinator
            .graph()
            .cost_by_task
            .get(&task_id)
            .copied()
            .unwrap_or(0),
    ) {
        fail_budget(coordinator, task_id, now_ms)?;
        return Ok(TurnReport {
            agent: agent_id,
            assigned: Some(task_id),
            turn_ok: false,
            completed: false,
            tokens_used: 0,
            budget_breached: true,
        });
    }
    // Per-goal leg (§9.8 runaway cost): over-budget goals do not burn more
    // turns. Uses the goal's own Budget unless the policy overrides it.
    let goal_id = coordinator
        .graph()
        .tasks
        .get(&task_id)
        .map(|task| task.goal_id);
    if let Some(goal_id) = goal_id
        && policy.over(
            effective_goal_cap(coordinator, goal_id, policy),
            goal_spend(coordinator, goal_id),
        )
    {
        fail_budget(coordinator, task_id, now_ms)?;
        return Ok(TurnReport {
            agent: agent_id,
            assigned: Some(task_id),
            turn_ok: false,
            completed: false,
            tokens_used: 0,
            budget_breached: true,
        });
    }
    let brief = coordinator
        .graph()
        .tasks
        .get(&task_id)
        .map(|task| task.brief.clone())
        .unwrap_or_default();
    let (events, breach) = run_monitored_turn(
        coordinator,
        adapter,
        agent_id,
        task_id,
        &brief,
        now_ms,
        policy,
    )?;
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
    // NOTE: token-based mid-turn kill rides the sample loop above; the
    // per-turn cap below is the post-turn backstop for adapters without a
    // control handle.
    record_cost(coordinator, agent_id, task_id, tokens_used, now_ms)?;
    if let Some(breach) = breach {
        // The watchdog already interrupted the turn; the partial spend
        // above stays recorded and the task fails with the matching reason.
        match breach {
            TurnBreach::Budget => fail_budget(coordinator, task_id, now_ms)?,
            TurnBreach::Timeout => {
                fail_task(coordinator, task_id, Some(TURN_TIMEOUT_REASON), now_ms)?
            }
        }
        return Ok(TurnReport {
            agent: agent_id,
            assigned: Some(task_id),
            turn_ok: false,
            completed: false,
            tokens_used,
            budget_breached: breach == TurnBreach::Budget,
        });
    }
    let task_total = coordinator
        .graph()
        .cost_by_task
        .get(&task_id)
        .copied()
        .unwrap_or(0);
    let agent_total = coordinator
        .graph()
        .cost_by_agent
        .get(&agent_id)
        .copied()
        .unwrap_or(0);
    let over = policy.over(policy.per_turn_tokens_max, tokens_used)
        || policy.over(policy.per_task_tokens_max, task_total)
        || policy.over(policy.per_agent_tokens_max, agent_total)
        || goal_id
            .map(|goal_id| {
                policy.over(
                    effective_goal_cap(coordinator, goal_id, policy),
                    goal_spend(coordinator, goal_id),
                )
            })
            .unwrap_or(false);
    if over {
        fail_budget(coordinator, task_id, now_ms)?;
        return Ok(TurnReport {
            agent: agent_id,
            assigned: Some(task_id),
            turn_ok: false,
            completed: false,
            tokens_used,
            budget_breached: true,
        });
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
        budget_breached: false,
    })
}

pub fn drive_goal(
    coordinator: &mut Coordinator,
    agents: &mut [(AgentId, Box<dyn AgentAdapter>)],
    goal_id: GoalId,
    now_ms: u64,
    ttl_ms: u64,
    max_rounds: u32,
    policy: &CostPolicy,
) -> Result<DriveReport, DriveError> {
    let mut rounds = 0;
    while rounds < max_rounds {
        rounds += 1;
        for (agent_id, adapter) in agents.iter_mut() {
            drive_agent_turn(
                coordinator,
                *agent_id,
                adapter.as_mut(),
                now_ms,
                ttl_ms,
                policy,
            )?;
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
            let report = drive_agent_turn(
                coordinator,
                *agent_id,
                adapter.as_mut(),
                now_ms,
                ttl_ms,
                &config.cost_policy,
            )?;
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
