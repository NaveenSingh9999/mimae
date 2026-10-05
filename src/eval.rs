use crate::event::KnownPayload;
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
