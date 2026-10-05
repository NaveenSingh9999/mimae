use crate::actor::Actor;
use crate::event::{Event, EventPayload};
use std::fs::{File, OpenOptions as FsOpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("log is locked by another writer: {0}")]
    Locked(PathBuf),
    #[error("corrupt event at line {line}: {source}")]
    Corrupt {
        line: usize,
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Durability {
    #[default]
    SyncData,
    FlushOnly,
}

#[derive(Debug, Clone, Default)]
pub struct EventLogOptions {
    pub durability: Durability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpenInfo {
    pub events: u64,
    pub high_water_seq: u64,
    pub torn_bytes_removed: u64,
}

pub trait EventStore {
    fn append(
        &mut self,
        actor: Actor,
        payload: EventPayload,
        causes: Vec<u64>,
        at_ms: u64,
    ) -> Result<Event, LogError>;
    fn events(&self) -> Result<Vec<Event>, LogError>;
    fn stream(&self) -> Result<EventStream, LogError>;
    fn high_water_seq(&self) -> u64;
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct EventStream {
    lines: std::io::Lines<BufReader<File>>,
    line_no: usize,
    failed: bool,
}

impl Iterator for EventStream {
    type Item = Result<Event, LogError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.failed {
                return None;
            }
            match self.lines.next() {
                Some(Ok(line)) => {
                    self.line_no += 1;
                    if line.trim().is_empty() {
                        continue;
                    }
                    return Some(serde_json::from_str::<Event>(&line).map_err(|source| {
                        LogError::Corrupt {
                            line: self.line_no,
                            source,
                        }
                    }));
                }
                Some(Err(err)) => {
                    self.failed = true;
                    return Some(Err(LogError::Io(err)));
                }
                None => return None,
            }
        }
    }
}

pub struct EventLog {
    path: PathBuf,
    file: File,
    durability: Durability,
    high_water_seq: u64,
    len: u64,
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn try_lock(file: &File) -> Result<bool, io::Error> {
    use std::os::unix::io::AsRawFd;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Ok(true)
    } else {
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(code) if code == libc::EWOULDBLOCK => Ok(false),
            _ => Err(err),
        }
    }
}

#[cfg(not(unix))]
fn try_lock(_file: &File) -> Result<bool, io::Error> {
    Ok(true)
}

fn scan_log(bytes: &[u8]) -> Result<(u64, u64, usize), LogError> {
    let mut events = 0u64;
    let mut high_water = 0u64;
    let mut complete_len = 0usize;
    let mut parts = bytes.split(|b| *b == b'\n').peekable();
    let mut line_no = 0usize;
    while let Some(chunk) = parts.next() {
        line_no += 1;
        if parts.peek().is_none() {
            break;
        }
        complete_len += chunk.len() + 1;
        if chunk.is_empty() {
            continue;
        }
        match serde_json::from_slice::<Event>(chunk) {
            Ok(event) => {
                high_water = high_water.max(event.seq);
                events += 1;
            }
            Err(source) => {
                return Err(LogError::Corrupt {
                    line: line_no,
                    source,
                });
            }
        }
    }
    Ok((events, high_water, complete_len))
}

impl EventLog {
    pub fn open(
        path: impl AsRef<Path>,
        options: EventLogOptions,
    ) -> Result<(Self, OpenInfo), LogError> {
        let path = path.as_ref().to_path_buf();
        let file = FsOpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)?;
        if !try_lock(&file)? {
            return Err(LogError::Locked(path));
        }
        let mut raw = Vec::new();
        File::open(&path)?.read_to_end(&mut raw)?;
        let (events, high_water_seq, complete_len) = scan_log(&raw)?;
        let torn_bytes_removed = (raw.len() - complete_len) as u64;
        if torn_bytes_removed > 0 {
            file.set_len(complete_len as u64)?;
        }
        Ok((
            Self {
                path,
                file,
                durability: options.durability,
                high_water_seq,
                len: events,
            },
            OpenInfo {
                events,
                high_water_seq,
                torn_bytes_removed,
            },
        ))
    }
}

impl EventStore for EventLog {
    fn append(
        &mut self,
        actor: Actor,
        payload: EventPayload,
        causes: Vec<u64>,
        at_ms: u64,
    ) -> Result<Event, LogError> {
        let seq = self.high_water_seq + 1;
        let event = Event::new(seq, actor, payload, causes, at_ms);
        let mut line = serde_json::to_vec(&event)?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.file.flush()?;
        if self.durability == Durability::SyncData {
            self.file.sync_data()?;
        }
        self.high_water_seq = seq;
        self.len += 1;
        Ok(event)
    }

    fn events(&self) -> Result<Vec<Event>, LogError> {
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut out = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let event: Event = serde_json::from_str(&line).map_err(|source| LogError::Corrupt {
                line: i + 1,
                source,
            })?;
            out.push(event);
        }
        Ok(out)
    }

    fn stream(&self) -> Result<EventStream, LogError> {
        Ok(EventStream {
            lines: BufReader::new(File::open(&self.path)?).lines(),
            line_no: 0,
            failed: false,
        })
    }

    fn high_water_seq(&self) -> u64 {
        self.high_water_seq
    }

    fn len(&self) -> u64 {
        self.len
    }
}
