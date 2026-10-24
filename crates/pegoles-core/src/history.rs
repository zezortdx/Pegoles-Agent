//! Bounded content-minimized history; never serialize raw events or actions.
use chrono::{DateTime, Utc};
use pegoles_protocol::{AgentEvent, AgentMessageKind, AgentTask, TaskId, TaskStatus};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_TASKS: usize = 100;
pub const MAX_AGE_DAYS: i64 = 30;
pub const MAX_HISTORY_BYTES: usize = 512 * 1024;
pub const MAX_RESULT_CHARS: usize = 4_000;
pub const MAX_AUDIT: usize = 32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtifactData {
    Text {
        text: String,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskArtifact {
    pub task_id: TaskId,
    pub data: ArtifactData,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEntry {
    pub status: TaskStatus,
    pub at: DateTime<Utc>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskHistory {
    pub task: AgentTask,
    pub audit: Vec<HistoryEntry>,
    pub artifact: Option<TaskArtifact>,
    pub recovered: bool,
    output_blocked: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    schema_version: u8,
    pub records: Vec<TaskHistory>,
    // Retention may evict a still-running task. Keep only its privacy marker
    // in memory so a later summary cannot regain persistence in this session.
    #[serde(skip)]
    session_blocked: std::collections::HashSet<TaskId>,
}
impl Default for History {
    fn default() -> Self {
        Self {
            schema_version: 1,
            records: Vec::new(),
            session_blocked: std::collections::HashSet::new(),
        }
    }
}
impl History {
    pub fn tasks(&self) -> Vec<AgentTask> {
        self.records.iter().map(|r| r.task.clone()).collect()
    }
    pub fn detail(&self, id: TaskId) -> Option<&TaskHistory> {
        self.records.iter().find(|r| r.task.id == id)
    }
    pub fn update_tasks(&mut self, tasks: &[AgentTask]) {
        for task in tasks {
            let mut task = task.clone();
            let sensitive_title = secret_pattern(&task.title);
            if sensitive_title {
                self.session_blocked.insert(task.id);
            }
            let output_blocked = self.session_blocked.contains(&task.id);
            if sensitive_title {
                task.title = "Private task".into();
            }
            if let Some(record) = self.records.iter_mut().find(|r| r.task.id == task.id) {
                record.task = task;
                if output_blocked {
                    record.output_blocked = true;
                    record.artifact = None;
                }
            } else {
                self.records.push(TaskHistory {
                    task,
                    audit: Vec::new(),
                    artifact: None,
                    recovered: false,
                    output_blocked,
                });
            }
        }
    }
    pub fn record(&mut self, event: &AgentEvent) {
        // Explicit projections only. No request, diagnostic, key or pixel is serialized.
        match event {
            AgentEvent::TaskStatusChanged {
                task_id, to, at, ..
            } => {
                if let Some(record) = self.records.iter_mut().find(|r| r.task.id == *task_id) {
                    record.audit.push(HistoryEntry {
                        status: *to,
                        at: *at,
                    });
                    if record.audit.len() > MAX_AUDIT {
                        record.audit.remove(0);
                    }
                }
            }
            AgentEvent::ActionRequested { request }
            | AgentEvent::ActionEvaluated { request, .. }
            | AgentEvent::ActionStarted { request, .. }
            | AgentEvent::ActionCompleted { request, .. }
            | AgentEvent::ActionDenied { request, .. }
            | AgentEvent::ActionFailed { request, .. } => {
                if let pegoles_protocol::ComputerAction::TypeText { text, sensitive } =
                    &request.action
                {
                    if *sensitive || secret_pattern(text) {
                        self.session_blocked.insert(request.task_id);
                        if let Some(record) = self
                            .records
                            .iter_mut()
                            .find(|r| r.task.id == request.task_id)
                        {
                            record.output_blocked = true;
                            record.artifact = None;
                        }
                    }
                }
            }
            AgentEvent::AgentMessage {
                task_id,
                kind: AgentMessageKind::Summary,
                text,
                ..
            } => {
                if let Some(record) = self.records.iter_mut().find(|r| r.task.id == *task_id) {
                    if secret_pattern(text) {
                        self.session_blocked.insert(*task_id);
                    }
                    record.output_blocked |= self.session_blocked.contains(task_id);
                    // A tainted task can never regain a persistent output, including after restart.
                    record.artifact = if record.output_blocked || text.trim().is_empty() {
                        None
                    } else {
                        let text: String = text.chars().take(MAX_RESULT_CHARS).collect();
                        Some(TaskArtifact {
                            task_id: *task_id,
                            data: table(&text).unwrap_or(ArtifactData::Text { text }),
                        })
                    };
                }
            }
            _ => {}
        }
    }
    /// Missing bus events might contain sensitive input; fail closed for outputs.
    pub fn block_outputs(&mut self) {
        for r in &mut self.records {
            self.session_blocked.insert(r.task.id);
            r.output_blocked = true;
            r.artifact = None;
        }
    }
    pub fn retain(&mut self, now: DateTime<Utc>) -> Result<(), String> {
        self.records
            .retain(|r| r.task.updated_at >= now - chrono::Duration::days(MAX_AGE_DAYS));
        self.records
            .sort_by_key(|r| (r.task.updated_at, r.task.id.to_string()));
        while self.records.len() > MAX_TASKS
            || serde_json::to_vec(self)
                .map_err(|_| "Could not encode history.")?
                .len()
                > MAX_HISTORY_BYTES
        {
            if self.records.is_empty() {
                return Err("History is too large.".into());
            }
            self.records.remove(0);
        }
        Ok(())
    }
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.records.len() > MAX_TASKS {
            return Err("Unsupported history.".into());
        }
        let mut ids = std::collections::HashSet::new();
        for r in &self.records {
            if crate::tasks::normalize_task_title(&r.task.title)
                .ok()
                .as_ref()
                != Some(&r.task.title)
                || r.task.updated_at < r.task.created_at
                || !ids.insert(r.task.id)
                || r.audit.len() > MAX_AUDIT
                || secret_pattern(&r.task.title)
            {
                return Err("Invalid task history.".into());
            }
            if let Some(a) = &r.artifact {
                if a.task_id != r.task.id || r.output_blocked {
                    return Err("Invalid task result.".into());
                }
                let valid = match &a.data {
                    ArtifactData::Text { text } => {
                        !text.trim().is_empty()
                            && text.chars().count() <= MAX_RESULT_CHARS
                            && !secret_pattern(text)
                    }
                    ArtifactData::Table { columns, rows } => {
                        let cells: Vec<_> = columns.iter().chain(rows.iter().flatten()).collect();
                        !columns.is_empty()
                            && columns.len() <= 8
                            && !rows.is_empty()
                            && rows.len() <= 32
                            && rows.iter().all(|r| r.len() == columns.len())
                            && cells
                                .iter()
                                .all(|c| c.chars().count() <= 256 && !secret_pattern(c))
                            && cells.iter().map(|c| c.chars().count()).sum::<usize>()
                                <= MAX_RESULT_CHARS
                    }
                };
                if !valid {
                    return Err("Invalid task result.".into());
                }
            }
        }
        Ok(())
    }
}
// Deliberately conservative tripwire, not a guarantee of secret detection.
// All output after explicitly sensitive typing is discarded regardless of content.
fn secret_pattern(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "-----begin",
        "sk-",
        "api_key",
        "apikey",
        "api key",
        "password",
        "passwd",
        "secret",
        "bearer ",
        "token=",
        "token:",
        "private key",
    ]
    .iter()
    .any(|p| text.contains(p))
}
fn table(text: &str) -> Option<ArtifactData> {
    let lines: Vec<_> = text.lines().filter(|s| !s.trim().is_empty()).collect();
    if !(2..=34).contains(&lines.len()) || !lines.iter().all(|s| s.contains('|')) {
        return None;
    }
    let mut lines = lines.into_iter().map(|s| {
        s.trim()
            .trim_matches('|')
            .split('|')
            .map(|v| v.trim().to_string())
            .collect::<Vec<_>>()
    });
    let columns = lines.next()?;
    let rows: Vec<_> = lines
        .filter(|r| {
            !r.iter()
                .all(|c| !c.is_empty() && c.chars().all(|c| c == '-' || c == ':'))
        })
        .collect();
    let data = ArtifactData::Table { columns, rows };
    if let ArtifactData::Table { columns, rows } = &data {
        if columns.is_empty()
            || columns.len() > 8
            || rows.is_empty()
            || rows.len() > 32
            || rows.iter().any(|r| r.len() != columns.len())
            || columns
                .iter()
                .chain(rows.iter().flatten())
                .any(|c| c.chars().count() > 256)
        {
            return None;
        }
    }
    Some(data)
}

pub struct HistoryStore {
    directory: PathBuf,
}
impl HistoryStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
    pub fn path(&self) -> PathBuf {
        self.directory.join("tasks.json")
    }
    pub fn read(&self, now: DateTime<Utc>) -> Result<History, String> {
        use std::io::Read;
        check_directories(&self.directory)?;
        let path = self.path();
        match std::fs::symlink_metadata(&path) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                return Err("Invalid history file.".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(History::default()),
            Err(_) => return Err("Could not inspect history.".into()),
        }
        let file = private_options()
            .read(true)
            .open(path)
            .map_err(|_| "Could not read history.")?;
        let meta = file.metadata().map_err(|_| "Could not inspect history.")?;
        if !meta.is_file() || meta.len() > MAX_HISTORY_BYTES as u64 {
            return Err("Invalid history file size.".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_HISTORY_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read history.")?;
        if bytes.len() > MAX_HISTORY_BYTES {
            return Err("History is too large.".into());
        }
        let mut h: History = serde_json::from_slice(&bytes).map_err(|_| "Invalid history file.")?;
        h.validate()?;
        h.retain(now)?;
        h.session_blocked = h
            .records
            .iter()
            .filter(|r| r.output_blocked)
            .map(|r| r.task.id)
            .collect();
        for r in &mut h.records {
            if matches!(
                r.task.status,
                TaskStatus::Pending | TaskStatus::Running | TaskStatus::WaitingForApproval
            ) {
                r.task.status = TaskStatus::Cancelled;
                r.recovered = true;
                // Preserve timestamps: restarting cannot extend retention.
            }
        }
        Ok(h)
    }
    pub fn write(&self, history: &History, now: DateTime<Utc>) -> Result<(), String> {
        use std::io::Write;
        // Validate the existing file before replacement; never erase corruption.
        self.read(now)?;
        let mut h = history.clone();
        h.retain(now)?;
        h.validate()?;
        let bytes = serde_json::to_vec(&h).map_err(|_| "Could not encode history.")?;
        check_directories(&self.directory)?;
        std::fs::create_dir_all(&self.directory)
            .map_err(|_| "Could not create history directory.")?;
        check_directories(&self.directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "Could not secure history.")?;
        }
        let temporary = self
            .directory
            .join(format!(".tasks-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = private_options()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|_| "Could not create history file.")?;
            file.write_all(&bytes)
                .map_err(|_| "Could not write history.")?;
            file.sync_all().map_err(|_| "Could not sync history.")?;
            std::fs::rename(&temporary, self.path()).map_err(|_| "Could not replace history.")?;
            #[cfg(unix)]
            private_options()
                .read(true)
                .open(&self.directory)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| "Could not sync history directory.")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

fn check_directories(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
                return Err("Invalid history directory.".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not inspect history directory.".into()),
        }
    }
    Ok(())
}
fn private_options() -> std::fs::OpenOptions {
    #[allow(unused_mut)]
    let mut options = std::fs::OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "macos")]
        const FLAGS: i32 = 0x100 | 0x4;
        #[cfg(not(target_os = "macos"))]
        const FLAGS: i32 = 0x20000 | 0x800;
        options.mode(0o600).custom_flags(FLAGS);
    }
    options
}
#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
