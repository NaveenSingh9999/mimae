use crate::event::KnownPayload;
use crate::failures::{FailureTag, summarize};
use crate::ids::TaskId;
use crate::log::{EventLogOptions, EventStore};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioMetrics {
    pub tasks_total: usize,
    pub tasks_done: usize,
    pub tasks_failed: usize,
    pub message_count: usize,
    pub messages_by_kind: HashMap<String, u64>,
    pub lease_expiries: usize,
    pub reclaims: usize,
    pub discovery_latency_avg_seqs: Option<f64>,
    pub human_interventions: u32,
    pub wall_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioReport {
    pub name: String,
    pub success: bool,
    pub metrics: ScenarioMetrics,
}

pub fn collect_metrics(
    log_path: impl AsRef<Path>,
    wall_ms: u64,
) -> Result<ScenarioMetrics, crate::log::LogError> {
    let (log, _) = crate::log::EventLog::open(log_path, EventLogOptions::default())?;
    let mut metrics = ScenarioMetrics {
        tasks_total: 0,
        tasks_done: 0,
        tasks_failed: 0,
        message_count: 0,
        messages_by_kind: HashMap::new(),
        lease_expiries: 0,
        reclaims: 0,
        discovery_latency_avg_seqs: None,
        human_interventions: 0,
        wall_ms,
    };
    let mut claims_per_task: HashMap<TaskId, usize> = HashMap::new();
    let mut pending_discoveries: Vec<(u64, TaskId)> = Vec::new();
    let mut latencies: Vec<u64> = Vec::new();
    for event in log.stream()? {
        let event = event?;
        let crate::event::EventPayload::Known(payload) = &event.payload else {
            continue;
        };
        match payload {
            KnownPayload::TaskCreated { .. } => metrics.tasks_total += 1,
            KnownPayload::MessageSent { message } => {
                metrics.message_count += 1;
                *metrics
                    .messages_by_kind
                    .entry(format!("{:?}", message.kind))
                    .or_insert(0) += 1;
                if message.kind == crate::entities::MessageKind::Discovery
                    && let Some(task) = message.refs.task_id
                {
                    pending_discoveries.push((event.seq, task));
                }
            }
            KnownPayload::TaskStatusChanged { task_id, to, .. } => {
                if *to == crate::entities::TaskStatus::Done {
                    metrics.tasks_done += 1;
                }
                if *to == crate::entities::TaskStatus::Failed {
                    metrics.tasks_failed += 1;
                }
                let mut still_pending = Vec::new();
                for (seq, task) in pending_discoveries.drain(..) {
                    if task == *task_id {
                        latencies.push(event.seq.saturating_sub(seq));
                    } else {
                        still_pending.push((seq, task));
                    }
                }
                pending_discoveries = still_pending;
            }
            KnownPayload::LeaseExpired { .. } => metrics.lease_expiries += 1,
            KnownPayload::TaskClaimed { task_id, .. } => {
                *claims_per_task.entry(*task_id).or_insert(0) += 1;
            }
            _ => {}
        }
    }
    metrics.reclaims = claims_per_task
        .values()
        .map(|count| count.saturating_sub(1))
        .sum();
    if !latencies.is_empty() {
        metrics.discovery_latency_avg_seqs =
            Some(latencies.iter().sum::<u64>() as f64 / latencies.len() as f64);
    }
    Ok(metrics)
}

pub fn report(
    name: &str,
    log_path: impl AsRef<Path>,
    wall_ms: u64,
    success: bool,
) -> Result<ScenarioReport, crate::log::LogError> {
    Ok(ScenarioReport {
        name: name.to_string(),
        success,
        metrics: collect_metrics(log_path, wall_ms)?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Baseline {
    Solo,
    Mimae,
    NaiveChat,
}

pub fn compare_message_cost(reports: &[ScenarioReport]) -> String {
    let mut out = String::from("baseline | success | messages | coord_share\n");
    for report in reports {
        let total_events = report.metrics.message_count
            + report.metrics.tasks_total
            + report.metrics.lease_expiries;
        let share = if total_events == 0 {
            0.0
        } else {
            report.metrics.message_count as f64 / total_events as f64
        };
        out.push_str(&format!(
            "{} | {} | {} | {:.2}\n",
            report.name, report.success, report.metrics.message_count, share
        ));
    }
    out
}

/// One breached promotion threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Metric or tag category that breached, e.g. `"tasks_failed"`.
    pub field: &'static str,
    pub actual: String,
    pub allowed: String,
}

impl Violation {
    fn over(
        field: &'static str,
        actual: impl std::fmt::Display,
        allowed: impl std::fmt::Display,
    ) -> Self {
        Self {
            field,
            actual: actual.to_string(),
            allowed: allowed.to_string(),
        }
    }
}

/// Pass/fail thresholds a scenario must meet for promotion (adapter
/// graduation, CI gating). Each scenario declares its own gate: a clean run
/// uses [`PromotionGate::strict`], while a recovery scenario explicitly
/// allows the expiry/reclaim/tag it is designed to exercise. Allowances are
/// always deliberate, never silent.
#[derive(Debug, Clone, PartialEq)]
pub struct PromotionGate {
    pub require_success: bool,
    pub max_tasks_failed: usize,
    pub max_human_interventions: u32,
    pub max_lease_expiries: usize,
    pub max_reclaims: usize,
    /// Cap on total coordination messages, if the scenario pins volume.
    pub max_messages: Option<usize>,
    /// Cap on discovery-to-action latency in log seqs, if measured.
    pub max_discovery_latency_seqs: Option<f64>,
    pub max_specification_tags: usize,
    pub max_misalignment_tags: usize,
    pub max_verification_tags: usize,
}

impl Default for PromotionGate {
    /// Strictest gate: success with zero failures, zero human touch, zero
    /// lease churn, and zero failure tags of any MAST category.
    fn default() -> Self {
        Self::strict()
    }
}

impl PromotionGate {
    pub fn strict() -> Self {
        Self {
            require_success: true,
            max_tasks_failed: 0,
            max_human_interventions: 0,
            max_lease_expiries: 0,
            max_reclaims: 0,
            max_messages: None,
            max_discovery_latency_seqs: None,
            max_specification_tags: 0,
            max_misalignment_tags: 0,
            max_verification_tags: 0,
        }
    }

    /// Check a finished [`ScenarioReport`] plus its [`FailureTag`]s against
    /// the thresholds. Empty output means promoted.
    pub fn check(&self, report: &ScenarioReport, tags: &[FailureTag]) -> Vec<Violation> {
        let mut violations = Vec::new();
        let m = &report.metrics;
        if self.require_success && !report.success {
            violations.push(Violation::over("success", report.success, true));
        }
        if m.tasks_failed > self.max_tasks_failed {
            violations.push(Violation::over(
                "tasks_failed",
                m.tasks_failed,
                format!("<= {}", self.max_tasks_failed),
            ));
        }
        if m.human_interventions > self.max_human_interventions {
            violations.push(Violation::over(
                "human_interventions",
                m.human_interventions,
                format!("<= {}", self.max_human_interventions),
            ));
        }
        if m.lease_expiries > self.max_lease_expiries {
            violations.push(Violation::over(
                "lease_expiries",
                m.lease_expiries,
                format!("<= {}", self.max_lease_expiries),
            ));
        }
        if m.reclaims > self.max_reclaims {
            violations.push(Violation::over(
                "reclaims",
                m.reclaims,
                format!("<= {}", self.max_reclaims),
            ));
        }
        if let Some(cap) = self.max_messages
            && m.message_count > cap
        {
            violations.push(Violation::over(
                "message_count",
                m.message_count,
                format!("<= {cap}"),
            ));
        }
        if let Some(cap) = self.max_discovery_latency_seqs {
            match m.discovery_latency_avg_seqs {
                Some(latency) if latency > cap => violations.push(Violation::over(
                    "discovery_latency_avg_seqs",
                    latency,
                    format!("<= {cap}"),
                )),
                None => violations.push(Violation::over(
                    "discovery_latency_avg_seqs",
                    "unmeasured",
                    format!("measured and <= {cap}"),
                )),
                _ => {}
            }
        }
        let counts = summarize(tags);
        if counts.specification > self.max_specification_tags as u64 {
            violations.push(Violation::over(
                "specification_tags",
                counts.specification,
                format!("<= {}", self.max_specification_tags),
            ));
        }
        if counts.misalignment > self.max_misalignment_tags as u64 {
            violations.push(Violation::over(
                "misalignment_tags",
                counts.misalignment,
                format!("<= {}", self.max_misalignment_tags),
            ));
        }
        if counts.verification > self.max_verification_tags as u64 {
            violations.push(Violation::over(
                "verification_tags",
                counts.verification,
                format!("<= {}", self.max_verification_tags),
            ));
        }
        violations
    }
}
