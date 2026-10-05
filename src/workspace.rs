use crate::ids::{AgentId, TaskId};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("git `{command}` failed with status {status}: {stderr}")]
    Git {
        command: String,
        status: String,
        stderr: String,
    },
    #[error("worktree already tracked at {0}")]
    Exists(PathBuf),
    #[error("unknown worktree at {0}")]
    Unknown(PathBuf),
}

fn git(repo: &Path, args: &[&str]) -> Result<String, WorkspaceError> {
    let command = format!("git {}", args.join(" "));
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(WorkspaceError::Io)?;
    if !output.status.success() {
        return Err(WorkspaceError::Git {
            command,
            status: format!("{}", output.status),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
    pub bare: bool,
}

pub struct WorkspaceManager {
    repo: PathBuf,
    main_branch: String,
}

impl WorkspaceManager {
    pub fn new(repo: impl AsRef<Path>, main_branch: &str) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            main_branch: main_branch.to_string(),
        }
    }

    pub fn repo(&self) -> &Path {
        &self.repo
    }

    pub fn main_branch(&self) -> &str {
        &self.main_branch
    }

    pub fn create_worktree(
        &self,
        path: impl AsRef<Path>,
        branch: &str,
    ) -> Result<WorktreeInfo, WorkspaceError> {
        let path = path.as_ref();
        git(
            &self.repo,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                &path.to_string_lossy(),
                &self.main_branch,
            ],
        )?;
        Ok(WorktreeInfo {
            path: path.to_path_buf(),
            branch: branch.to_string(),
            bare: false,
        })
    }

    pub fn remove_worktree(
        &self,
        path: impl AsRef<Path>,
        force: bool,
    ) -> Result<(), WorkspaceError> {
        let path = path.as_ref();
        if force {
            git(
                &self.repo,
                &["worktree", "remove", "--force", &path.to_string_lossy()],
            )?;
        } else {
            git(&self.repo, &["worktree", "remove", &path.to_string_lossy()])?;
        }
        Ok(())
    }

    pub fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>, WorkspaceError> {
        let out = git(&self.repo, &["worktree", "list", "--porcelain"])?;
        let mut infos = Vec::new();
        let mut path: Option<PathBuf> = None;
        let mut branch: Option<String> = None;
        let mut bare = false;
        let flush = |path: &mut Option<PathBuf>,
                     branch: &mut Option<String>,
                     bare: &mut bool,
                     infos: &mut Vec<WorktreeInfo>| {
            if let Some(p) = path.take() {
                infos.push(WorktreeInfo {
                    path: p,
                    branch: branch.take().unwrap_or_default(),
                    bare: *bare,
                });
                *bare = false;
            }
        };
        for line in out.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                flush(&mut path, &mut branch, &mut bare, &mut infos);
                path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                branch = Some(b.to_string());
            } else if line.trim() == "bare" {
                bare = true;
            }
        }
        flush(&mut path, &mut branch, &mut bare, &mut infos);
        Ok(infos)
    }

    pub fn changed_vs_base(&self, base: &str, branch: &str) -> Result<Vec<String>, WorkspaceError> {
        let out = git(
            &self.repo,
            &["diff", "--name-only", &format!("{base}...{branch}")],
        )?;
        Ok(out
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }

    pub fn dirty_paths(&self, worktree: impl AsRef<Path>) -> Result<Vec<String>, WorkspaceError> {
        let out = git(worktree.as_ref(), &["status", "--porcelain"])?;
        let mut paths = Vec::new();
        for line in out.lines() {
            if line.len() < 4 {
                continue;
            }
            let rest = line[3..].trim();
            if rest.is_empty() {
                continue;
            }
            let path = rest
                .split_once(" -> ")
                .map(|(_, to)| to)
                .unwrap_or(rest)
                .trim_matches('"')
                .to_string();
            if !path.is_empty() {
                paths.push(path);
            }
        }
        Ok(paths)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathOwner {
    pub agent: AgentId,
    pub task: TaskId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathConflict {
    pub path: String,
    pub owner: PathOwner,
}

#[derive(Debug, Default)]
pub struct PathLeaseRegistry {
    leases: HashMap<String, PathOwner>,
}

impl PathLeaseRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn acquire(&mut self, paths: &[String], owner: PathOwner) -> Result<(), Vec<PathConflict>> {
        let mut conflicts = Vec::new();
        for path in paths {
            if let Some(existing) = self.leases.get(path)
                && *existing != owner
            {
                conflicts.push(PathConflict {
                    path: path.clone(),
                    owner: existing.clone(),
                });
            }
        }
        if !conflicts.is_empty() {
            return Err(conflicts);
        }
        for path in paths {
            self.leases.insert(path.clone(), owner.clone());
        }
        Ok(())
    }

    pub fn release_owner(&mut self, agent: AgentId) {
        self.leases.retain(|_, owner| owner.agent != agent);
    }

    pub fn release_paths(&mut self, paths: &[String]) {
        for path in paths {
            self.leases.remove(path);
        }
    }

    pub fn owner_of(&self, path: &str) -> Option<&PathOwner> {
        self.leases.get(path)
    }

    pub fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlap {
    pub a: String,
    pub b: String,
    pub paths: Vec<String>,
}

pub fn predict_overlaps(branches: &[(String, Vec<String>)]) -> Vec<Overlap> {
    let mut out = Vec::new();
    for (i, (a_name, a_paths)) in branches.iter().enumerate() {
        let a_set: HashSet<&str> = a_paths.iter().map(|p| p.as_str()).collect();
        for (b_name, b_paths) in &branches[i + 1..] {
            let mut shared: Vec<String> = b_paths
                .iter()
                .filter(|p| a_set.contains(p.as_str()))
                .cloned()
                .collect();
            shared.sort();
            if !shared.is_empty() {
                out.push(Overlap {
                    a: a_name.clone(),
                    b: b_name.clone(),
                    paths: shared,
                });
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckOutcome {
    Passed,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    Merged,
    Conflicts(Vec<String>),
}

pub struct IntegrationGate {
    repo: PathBuf,
    main_branch: String,
    integration_branch: String,
}

impl IntegrationGate {
    pub fn new(repo: impl AsRef<Path>, main_branch: &str, integration_branch: &str) -> Self {
        Self {
            repo: repo.as_ref().to_path_buf(),
            main_branch: main_branch.to_string(),
            integration_branch: integration_branch.to_string(),
        }
    }

    pub fn prepare(&self, dir: impl AsRef<Path>) -> Result<(), WorkspaceError> {
        let dir = dir.as_ref();
        if dir.exists() {
            let removed = git(
                &self.repo,
                &["worktree", "remove", "--force", &dir.to_string_lossy()],
            );
            if removed.is_err() {
                std::fs::remove_dir_all(dir).map_err(WorkspaceError::Io)?;
            }
        }
        git(
            &self.repo,
            &[
                "worktree",
                "add",
                "-B",
                &self.integration_branch,
                &dir.to_string_lossy(),
                &self.main_branch,
            ],
        )?;
        Ok(())
    }

    pub fn merge_branch(
        &self,
        dir: impl AsRef<Path>,
        branch: &str,
    ) -> Result<MergeOutcome, WorkspaceError> {
        let dir = dir.as_ref();
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["merge", "--no-edit", "--no-ff", branch])
            .output()
            .map_err(WorkspaceError::Io)?;
        if output.status.success() {
            return Ok(MergeOutcome::Merged);
        }
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !combined.contains("CONFLICT") {
            return Err(WorkspaceError::Git {
                command: format!("git merge --no-edit --no-ff {branch}"),
                status: format!("{}", output.status),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        let unmerged = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["diff", "--name-only", "--diff-filter=U"])
            .output()
            .map_err(WorkspaceError::Io)?;
        let _ = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["merge", "--abort"])
            .output();
        let paths: Vec<String> = String::from_utf8_lossy(&unmerged.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        Ok(MergeOutcome::Conflicts(paths))
    }

    pub fn run_check(
        &self,
        dir: impl AsRef<Path>,
        program: &str,
        args: &[&str],
        timeout: std::time::Duration,
    ) -> Result<CheckOutcome, WorkspaceError> {
        use std::process::Stdio;
        let mut child = Command::new(program)
            .args(args)
            .current_dir(dir.as_ref())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(WorkspaceError::Io)?;
        let start = std::time::Instant::now();
        loop {
            match child.try_wait().map_err(WorkspaceError::Io)? {
                Some(status) => {
                    return Ok(if status.success() {
                        CheckOutcome::Passed
                    } else {
                        CheckOutcome::Failed
                    });
                }
                None => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Ok(CheckOutcome::TimedOut);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }

    pub fn advance(&self) -> Result<(), WorkspaceError> {
        git(&self.repo, &["checkout", "-q", &self.main_branch])?;
        git(
            &self.repo,
            &["merge", "-q", "--ff-only", &self.integration_branch],
        )?;
        Ok(())
    }

    pub fn integration_branch(&self) -> &str {
        &self.integration_branch
    }
}
