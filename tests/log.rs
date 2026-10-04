use mimae_core::*;
use std::io::Write;

fn tmp_log(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("mimae-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn discovery(seq: u64) -> EventPayload {
    EventPayload::Known(KnownPayload::MessageSent {
        message: Message {
            id: MessageId::new(seq),
            kind: MessageKind::Discovery,
            sender: Actor::Agent {
                agent_id: AgentId::new(1),
            },
            recipients: vec![AgentId::new(2)],
            refs: MessageRefs {
                task_id: Some(TaskId::new(1)),
                contract_id: None,
                decision_id: None,
            },
            priority: MessagePriority::Medium,
            provenance: Actor::Agent {
                agent_id: AgentId::new(1),
            },
            body: format!("finding {seq}"),
        },
    })
}

#[test]
fn append_and_read_back_roundtrips() {
    let path = tmp_log("roundtrip.jsonl");
    let (mut log, info) = EventLog::open(&path, EventLogOptions::default()).unwrap();
    assert_eq!(info, OpenInfo::default());
    for i in 1..=3u64 {
        log.append(Actor::Coordinator, discovery(i), vec![], i * 100)
            .unwrap();
    }
    let events = log.events().unwrap();
    assert_eq!(events.len(), 3);
    assert!(
        events
            .iter()
            .enumerate()
            .all(|(i, e)| e.seq == (i as u64 + 1))
    );
    assert_eq!(log.high_water_seq(), 3);
    assert_eq!(log.len(), 3);
}

#[test]
fn reopen_preserves_history_and_continues_seq() {
    let path = tmp_log("reopen.jsonl");
    {
        let (mut log, _) = EventLog::open(&path, EventLogOptions::default()).unwrap();
        log.append(Actor::Coordinator, discovery(1), vec![], 100)
            .unwrap();
        log.append(Actor::Coordinator, discovery(2), vec![], 200)
            .unwrap();
    }
    let (mut log, info) = EventLog::open(&path, EventLogOptions::default()).unwrap();
    assert_eq!(info.events, 2);
    assert_eq!(info.high_water_seq, 2);
    assert_eq!(info.torn_bytes_removed, 0);
    let e = log.append(Actor::Host, discovery(3), vec![2], 300).unwrap();
    assert_eq!(e.seq, 3);
    assert_eq!(log.events().unwrap().len(), 3);
}

#[test]
fn torn_tail_is_truncated_and_recovered() {
    let path = tmp_log("torn.jsonl");
    {
        let (mut log, _) = EventLog::open(&path, EventLogOptions::default()).unwrap();
        log.append(Actor::Coordinator, discovery(1), vec![], 100)
            .unwrap();
        log.append(Actor::Coordinator, discovery(2), vec![], 200)
            .unwrap();
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(b"{\"schema_version\":1,\"seq\":3,\"actor\":")
        .unwrap();
    file.flush().unwrap();
    drop(file);

    let (mut log, info) = EventLog::open(&path, EventLogOptions::default()).unwrap();
    assert_eq!(info.events, 2);
    assert!(info.torn_bytes_removed > 0);
    let e = log.append(Actor::Host, discovery(3), vec![], 300).unwrap();
    assert_eq!(e.seq, 3);
    let events = log.events().unwrap();
    assert_eq!(events.len(), 3);
}

#[test]
fn corrupt_middle_line_is_a_hard_error() {
    let path = tmp_log("corrupt.jsonl");
    {
        let (mut log, _) = EventLog::open(&path, EventLogOptions::default()).unwrap();
        log.append(Actor::Coordinator, discovery(1), vec![], 100)
            .unwrap();
        log.append(Actor::Coordinator, discovery(2), vec![], 200)
            .unwrap();
    }
    let mut raw = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<&str> = raw.lines().collect();
    lines.insert(1, "not json at all");
    raw = lines.join("\n") + "\n";
    std::fs::write(&path, raw).unwrap();

    let result = EventLog::open(&path, EventLogOptions::default());
    assert!(matches!(result, Err(LogError::Corrupt { .. })));
}

#[cfg(unix)]
#[test]
fn second_writer_is_rejected_until_first_drops() {
    let path = tmp_log("locked.jsonl");
    let (log, _) = EventLog::open(&path, EventLogOptions::default()).unwrap();
    let second = EventLog::open(&path, EventLogOptions::default());
    assert!(matches!(second, Err(LogError::Locked(_))));
    drop(log);
    let third = EventLog::open(&path, EventLogOptions::default());
    assert!(third.is_ok());
}

#[test]
fn seq_and_causes_are_preserved_in_file_order() {
    let path = tmp_log("order.jsonl");
    let (mut log, _) = EventLog::open(&path, EventLogOptions::default()).unwrap();
    let first = log
        .append(Actor::Coordinator, discovery(1), vec![], 100)
        .unwrap();
    let second = log
        .append(
            Actor::Agent {
                agent_id: AgentId::new(1),
            },
            discovery(2),
            vec![first.seq],
            200,
        )
        .unwrap();
    assert_eq!(second.causes, vec![first.seq]);
    let events = log.events().unwrap();
    assert_eq!(events[0].seq, 1);
    assert_eq!(events[1].causes, vec![1]);
}

#[test]
fn usable_through_eventstore_trait_object() {
    let path = tmp_log("trait.jsonl");
    let (log, _) = EventLog::open(&path, EventLogOptions::default()).unwrap();
    let mut store: Box<dyn EventStore> = Box::new(log);
    store
        .append(Actor::System, discovery(1), vec![], 1)
        .unwrap();
    assert_eq!(store.high_water_seq(), 1);
    assert_eq!(store.len(), 1);
}
