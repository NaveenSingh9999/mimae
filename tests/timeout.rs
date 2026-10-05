use mimae_core::{TurnOutcome, run_bounded_turn};
use std::io::BufReader;
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn fast_child_exits_before_deadline() {
    let mut child = Command::new("echo")
        .arg("hello")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut lines = Vec::new();
    let outcome: TurnOutcome = run_bounded_turn(
        &mut child,
        BufReader::new(stdout),
        Duration::from_secs(10),
        |line| lines.push(line.to_string()),
    );
    assert!(!outcome.timed_out);
    assert!(outcome.exited);
    assert!(outcome.success);
    assert_eq!(lines, vec!["hello".to_string()]);
    assert!(child.try_wait().unwrap().is_some());
}

#[test]
fn hung_child_is_killed_at_deadline() {
    let mut child = Command::new("sleep")
        .arg("60")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut lines = Vec::new();
    let outcome: TurnOutcome = run_bounded_turn(
        &mut child,
        BufReader::new(stdout),
        Duration::from_millis(300),
        |line| lines.push(line.to_string()),
    );
    assert!(outcome.timed_out);
    assert!(!outcome.exited);
    assert!(!outcome.success);
    assert!(lines.is_empty());
    assert!(child.try_wait().unwrap().is_some());
}
