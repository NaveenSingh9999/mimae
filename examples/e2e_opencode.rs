use mimae_core::*;
use std::error::Error;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn main() -> Result<(), Box<dyn Error>> {
    let ts_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("mimae-e2e-{ts_nanos}"));
    std::fs::create_dir_all(&tmp)?;
    let log_path = tmp.join("run.log");

    let port = OpenCodeAdapter::free_port().ok_or("no free port")?;
    println!(
        "[example] spawning opencode on 127.0.0.1:{port} in {}",
        tmp.display()
    );
    let mut adapter = OpenCodeAdapter::spawn_local(&tmp.to_string_lossy(), port)
        .map_err(|e| format!("spawn opencode: {e}"))?;
    match adapter.start_session(
        &tmp.to_string_lossy(),
        "You reply with only the word returned.",
        None,
    ) {
        SessionReply::Started => println!("[example] session started"),
        SessionReply::Failed { error } => return Err(error.into()),
    }

    let (mut coord, _info) = Coordinator::open(&log_path, EventLogOptions::default())?;
    let head = AgentId::new(1);
    let _ = coord.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::TeamCreated {
            team: Team {
                id: TeamId::new(1),
                head,
                mode: TeamMode::Auto,
                state: TeamState::Initializing,
                members: vec![head],
            },
        }),
        vec![],
        now_ms(),
    )?;
    let _ = coord.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::AgentRegistered {
            agent: Agent {
                id: head,
                cli: CliKind::OpenCode,
                capabilities: vec![Capability::new("code_edit")],
                role: Some("worker".into()),
                state: AgentState::Idle,
                budget_used_tokens: 0,
            },
        }),
        vec![],
        now_ms(),
    )?;
    let _ = coord.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::AgentReady { agent_id: head }),
        vec![],
        now_ms(),
    )?;
    println!("[example] team state: {:?}", coord.team().map(|t| t.state));

    let _ = coord.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::TaskCreated {
            task: Task {
                id: TaskId::new(1),
                goal_id: GoalId::new(10),
                title: "echo".into(),
                brief: "Reply with only the word pong".into(),
                capability: Capability::new("echo"),
                write_scope: WriteScope::default(),
                deps: vec![],
                status: TaskStatus::Proposed,
                owner: None,
                lease: None,
                verification: Verification::None,
            },
        }),
        vec![],
        now_ms(),
    )?;
    //dep milestones: claim-of owned for task here
    let _ = coord.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::TaskStatusChanged {
            task_id: TaskId::new(1),
            from: TaskStatus::Proposed,
            to: TaskStatus::Ready,
            reason: None,
        }),
        vec![],
        now_ms(),
    )?;
    let _ = coord.claim(
        TaskId::new(1),
        head,
        WriteScope::default(),
        now_ms(),
        120_000,
    )?;
    println!("[example] claimed t1 by agent1");
    let _ = coord.check_for_expired_leases(now_ms())?;
    let _ = coord.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::TaskStatusChanged {
            task_id: TaskId::new(1),
            from: TaskStatus::Claimed,
            to: TaskStatus::InProgress,
            reason: None,
        }),
        vec![],
        now_ms(),
    )?;

    let events = adapter.send_turn("Reply with only the word pong.");
    let ok = events
        .iter()
        .any(|e| matches!(e, SessionEvent::TurnComplete { ok: true }));
    println!("[example] turn completed ok={}", ok);
    if ok {
        for (from, to) in [
            (TaskStatus::InProgress, TaskStatus::InReview),
            (TaskStatus::InReview, TaskStatus::Integrating),
            (TaskStatus::Integrating, TaskStatus::Done),
        ] {
            let _ = coord.publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::TaskStatusChanged {
                    task_id: TaskId::new(1),
                    from,
                    to,
                    reason: None,
                }),
                vec![],
                now_ms(),
            )?;
        }
    } else {
        let _ = coord.publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TaskStatusChanged {
                task_id: TaskId::new(1),
                from: TaskStatus::InProgress,
                to: TaskStatus::Failed,
                reason: Some("adapter turn failed".into()),
            }),
            vec![],
            now_ms(),
        )?;
    }
    println!(
        "[example] final task status: {:?}",
        coord.graph().tasks.get(&TaskId::new(1)).map(|t| &t.status)
    );
    println!("[example] log: {}", log_path.display());
    Ok(())
}
