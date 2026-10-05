use mimae_core::*;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("mimae-ws-{name}-{nanos}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "git {args:?} failed in {}",
        repo.display()
    );
}

fn init_repo() -> PathBuf {
    let repo = unique_dir("repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    std::fs::write(repo.join("a.txt"), "base\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "add a",
        ],
    );
    repo
}

#[test]
fn worktree_create_list_remove_roundtrip() {
    let repo = init_repo();
    let mgr = WorkspaceManager::new(&repo, "main");
    let wt = unique_dir("wt1");
    let info = mgr.create_worktree(&wt, "agent/one").unwrap();
    assert_eq!(info.branch, "agent/one");
    assert_eq!(info.path, wt);
    let listed = mgr.list_worktrees().unwrap();
    assert!(
        listed
            .iter()
            .any(|w| w.path == wt && w.branch == "agent/one")
    );
    assert!(listed.iter().any(|w| w.branch == "main"));
    mgr.remove_worktree(&wt, true).unwrap();
    let listed = mgr.list_worktrees().unwrap();
    assert!(!listed.iter().any(|w| w.path == wt));
}

#[test]
fn dirty_paths_reports_modified_and_untracked() {
    let repo = init_repo();
    let mgr = WorkspaceManager::new(&repo, "main");
    std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
    std::fs::write(repo.join("new.txt"), "fresh\n").unwrap();
    let mut dirty = mgr.dirty_paths(&repo).unwrap();
    dirty.sort();
    assert_eq!(dirty, vec!["a.txt".to_string(), "new.txt".to_string()]);
}

#[test]
fn path_leases_conflict_and_release() {
    let mut reg = PathLeaseRegistry::new();
    assert!(reg.is_empty());
    let owner_a = PathOwner {
        agent: AgentId::new(1),
        task: TaskId::new(1),
    };
    let owner_b = PathOwner {
        agent: AgentId::new(2),
        task: TaskId::new(2),
    };
    reg.acquire(&["src/api".to_string()], owner_a.clone())
        .unwrap();
    assert_eq!(reg.owner_of("src/api"), Some(&owner_a));
    let err = reg
        .acquire(&["src/api".to_string()], owner_b.clone())
        .unwrap_err();
    assert_eq!(
        err,
        vec![PathConflict {
            path: "src/api".to_string(),
            owner: owner_a.clone(),
        }]
    );
    reg.acquire(&["src/ui".to_string()], owner_b.clone())
        .unwrap();
    reg.release_owner(AgentId::new(1));
    assert!(reg.owner_of("src/api").is_none());
    reg.acquire(&["src/api".to_string()], owner_b).unwrap();
}

#[test]
fn predict_overlaps_flags_shared_paths_only() {
    let overlaps = predict_overlaps(&[
        (
            "t1".to_string(),
            vec!["src/a.rs".to_string(), "x.txt".to_string()],
        ),
        (
            "t2".to_string(),
            vec!["src/b.rs".to_string(), "x.txt".to_string()],
        ),
        ("t3".to_string(), vec!["src/c.rs".to_string()]),
    ]);
    assert_eq!(overlaps.len(), 1);
    assert_eq!(overlaps[0].a, "t1");
    assert_eq!(overlaps[0].b, "t2");
    assert_eq!(overlaps[0].paths, vec!["x.txt".to_string()]);
}

#[test]
fn integration_gate_merges_clean_branch_and_advances() {
    let repo = init_repo();
    let gate = IntegrationGate::new(&repo, "main", "mimae-integration");
    let dir = unique_dir("integ");
    gate.prepare(&dir).unwrap();
    git(&repo, &["checkout", "-q", "-b", "feature/good"]);
    std::fs::write(repo.join("good.txt"), "fine\n").unwrap();
    git(&repo, &["add", "good.txt"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "good",
        ],
    );
    git(&repo, &["checkout", "-q", "main"]);
    assert_eq!(
        gate.merge_branch(&dir, "feature/good").unwrap(),
        MergeOutcome::Merged
    );
    assert!(gate.run_check(&dir, "true", &[]).unwrap());
    assert!(!gate.run_check(&dir, "false", &[]).unwrap());
    gate.advance().unwrap();
    let head = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["log", "--format=%s", "-1", "main"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&head.stdout).contains("good"));
    assert!(repo.join("good.txt").exists());
}

#[test]
fn integration_gate_reports_conflicts_not_breakage() {
    let repo = init_repo();
    let gate = IntegrationGate::new(&repo, "main", "mimae-integration");
    let dir = unique_dir("integ-bad");
    git(&repo, &["checkout", "-q", "-b", "feature/bad"]);
    std::fs::write(repo.join("a.txt"), "theirs\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "theirs",
        ],
    );
    git(&repo, &["checkout", "-q", "main"]);
    std::fs::write(repo.join("a.txt"), "ours\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "ours",
        ],
    );
    gate.prepare(&dir).unwrap();
    match gate.merge_branch(&dir, "feature/bad").unwrap() {
        MergeOutcome::Conflicts(paths) => assert_eq!(paths, vec!["a.txt".to_string()]),
        MergeOutcome::Merged => panic!("expected conflicts"),
    }
    let status = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&status.stdout).trim().is_empty());
}
