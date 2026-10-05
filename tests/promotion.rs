//! Promotion-gate unit tests: each threshold breaches independently, and a
//! clean report with no tags passes the strict gate.

use mimae_core::*;
use std::collections::HashMap;

fn metrics() -> ScenarioMetrics {
    ScenarioMetrics {
        tasks_total: 2,
        tasks_done: 2,
        tasks_failed: 0,
        message_count: 3,
        messages_by_kind: HashMap::new(),
        lease_expiries: 0,
        reclaims: 0,
        discovery_latency_avg_seqs: None,
        human_interventions: 0,
        wall_ms: 5,
    }
}

fn report_with(metrics: ScenarioMetrics) -> ScenarioReport {
    ScenarioReport {
        name: "unit".to_string(),
        success: true,
        metrics,
    }
}

fn tag(kind: FailureKind) -> FailureTag {
    FailureTag {
        seq: 1,
        task_id: None,
        kind,
        category: kind.category(),
        detail: "unit".to_string(),
    }
}

fn fields(violations: &[Violation]) -> Vec<&str> {
    violations.iter().map(|v| v.field).collect()
}

#[test]
fn strict_gate_passes_clean_report() {
    let report = report_with(metrics());
    assert!(PromotionGate::strict().check(&report, &[]).is_empty());
    assert!(PromotionGate::default().check(&report, &[]).is_empty());
}

#[test]
fn gate_flags_failed_run() {
    let mut report = report_with(metrics());
    report.success = false;
    let violations = PromotionGate::strict().check(&report, &[]);
    assert_eq!(fields(&violations), vec!["success"]);
}

#[test]
fn gate_flags_tasks_failed() {
    let mut m = metrics();
    m.tasks_failed = 2;
    let violations = PromotionGate::strict().check(&report_with(m), &[]);
    assert_eq!(fields(&violations), vec!["tasks_failed"]);
    assert_eq!(violations[0].actual, "2");
}

#[test]
fn gate_flags_human_interventions() {
    let mut m = metrics();
    m.human_interventions = 1;
    let violations = PromotionGate::strict().check(&report_with(m), &[]);
    assert_eq!(fields(&violations), vec!["human_interventions"]);
}

#[test]
fn gate_flags_lease_churn() {
    let mut m = metrics();
    m.lease_expiries = 2;
    m.reclaims = 2;
    let violations = PromotionGate::strict().check(&report_with(m.clone()), &[]);
    assert_eq!(fields(&violations), vec!["lease_expiries", "reclaims"]);
    // Allowances silence exactly what they cover.
    let gate = PromotionGate {
        max_lease_expiries: 2,
        max_reclaims: 2,
        ..PromotionGate::strict()
    };
    assert!(gate.check(&report_with(m), &[]).is_empty());
}

#[test]
fn gate_flags_message_cap_breach() {
    let m = metrics();
    let gate = PromotionGate {
        max_messages: Some(2),
        ..PromotionGate::strict()
    };
    let violations = gate.check(&report_with(m), &[]);
    assert_eq!(fields(&violations), vec!["message_count"]);
    // No cap configured: volume is not judged.
    assert!(
        PromotionGate::strict()
            .check(&report_with(metrics()), &[])
            .is_empty()
    );
}

#[test]
fn gate_flags_latency_breach_and_missing_measurement() {
    let gate = PromotionGate {
        max_discovery_latency_seqs: Some(10.0),
        ..PromotionGate::strict()
    };
    // No measurement at all is a violation when a cap is set.
    assert_eq!(
        fields(&gate.check(&report_with(metrics()), &[])),
        vec!["discovery_latency_avg_seqs"]
    );
    let mut slow = metrics();
    slow.discovery_latency_avg_seqs = Some(25.0);
    assert_eq!(
        fields(&gate.check(&report_with(slow), &[])),
        vec!["discovery_latency_avg_seqs"]
    );
    let mut fast = metrics();
    fast.discovery_latency_avg_seqs = Some(4.0);
    assert!(gate.check(&report_with(fast), &[]).is_empty());
}

#[test]
fn gate_flags_failure_tags_by_category() {
    let tags = vec![
        tag(FailureKind::LeaseExpired),
        tag(FailureKind::DecisionConflict),
        tag(FailureKind::GateFailed),
    ];
    let violations = PromotionGate::strict().check(&report_with(metrics()), &tags);
    assert_eq!(
        fields(&violations),
        vec![
            "specification_tags",
            "misalignment_tags",
            "verification_tags"
        ]
    );
    let gate = PromotionGate {
        max_specification_tags: 1,
        max_misalignment_tags: 1,
        max_verification_tags: 1,
        ..PromotionGate::strict()
    };
    assert!(gate.check(&report_with(metrics()), &tags).is_empty());
}
