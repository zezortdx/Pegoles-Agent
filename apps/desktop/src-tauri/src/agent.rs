//! Agent runs for the desktop app: the Core bridge the orchestrator
//! drives, the single active run, and model settings + API key storage.
//!
//! The API key lives in the macOS Keychain (or `ANTHROPIC_API_KEY`). It
//! never reaches the webview (the UI only learns whether one is set), is
//! never logged, and never enters the guest: it is only an HTTPS header
//! on requests to the Anthropic API.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pegoles_agent::anthropic::{
    AnthropicConfig, AnthropicPlanner, DEFAULT_EFFORT, DEFAULT_MODEL, EFFORTS, SUPPORTED_MODELS,
};
use pegoles_agent::{run_task, CoreAccess, CoreComputer, RunLimits};
use pegoles_core::{CancellationToken, ComputerRegistry, EventBus, TaskManager};
use pegoles_protocol::{TaskId, TaskStatus};
use serde::{Deserialize, Serialize};

use crate::commands::{lock_state, SharedState};

/// Core access for the orchestrator: one short lock per call, history
/// synced after, poison-tolerant.
pub struct AppCore(pub SharedState);

impl CoreAccess for AppCore {
    fn with_core<R>(&self, f: impl FnOnce(&mut ComputerRegistry, &mut TaskManager) -> R) -> R {
        let mut guard = lock_state(&self.0);
        let state = &mut *guard;
        let result = f(&mut state.registry, &mut state.tasks);
        state.sync_history();
        result
    }
}

/// At most one agent run at a time (one computer). The cancel token is
/// held outside the app-state lock so Stop is always immediate.
#[derive(Clone, Default)]
pub struct AgentSupervisor(Arc<Mutex<Option<(TaskId, CancellationToken)>>>);

impl AgentSupervisor {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<(TaskId, CancellationToken)>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn claim(&self, task: TaskId) -> Result<CancellationToken, String> {
        let mut slot = self.slot();
        if let Some((running, _)) = slot.as_ref() {
            return Err(format!("task {running} is already running"));
        }
        let cancel = CancellationToken::new();
        *slot = Some((task, cancel.clone()));
        Ok(cancel)
    }

    fn release(&self, task: TaskId) {
        let mut slot = self.slot();
        if slot.as_ref().is_some_and(|(t, _)| *t == task) {
            *slot = None;
        }
    }

    /// Cancel the active run (all runs when `task` is None). Returns
    /// whether a run was signalled.
    pub fn cancel(&self, task: Option<TaskId>) -> bool {
        match self.slot().as_ref() {
            Some((running, cancel)) if task.is_none_or(|t| t == *running) => {
                cancel.cancel();
                true
            }
            _ => false,
        }
    }

    pub fn active(&self) -> Option<TaskId> {
        self.slot().as_ref().map(|(t, _)| *t)
    }
}

// --- settings --------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelSettings {
    pub model: String,
    pub effort: String,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            effort: DEFAULT_EFFORT.to_string(),
        }
    }
}

impl ModelSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !SUPPORTED_MODELS.contains(&self.model.as_str()) {
            return Err(format!("unsupported model {:?}", self.model));
        }
        if !EFFORTS.contains(&self.effort.as_str()) {
            return Err(format!("unsupported effort {:?}", self.effort));
        }
        Ok(())
    }
}

fn settings_path() -> PathBuf {
    pegoles_computer::pegoles_data_dir().join("settings.json")
}

pub fn load_settings() -> ModelSettings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<ModelSettings>(&raw).ok())
        .filter(|s| s.validate().is_ok())
        .unwrap_or_default()
}

pub fn save_settings(settings: &ModelSettings) -> Result<(), String> {
    settings.validate()?;
    let path = settings_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, raw).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

// --- API key ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    Keychain,
    Environment,
}

const KEYCHAIN_SERVICE: &str = "dev.pegoles.agent";
const KEYCHAIN_ACCOUNT: &str = "anthropic-api-key";

/// Shape check only (the API is the authority). Rejects anything that
/// could smuggle whitespace or control bytes into an HTTP header.
pub fn validate_api_key(raw: &str) -> Result<String, String> {
    let key = raw.trim();
    if !(20..=256).contains(&key.len()) {
        return Err("the key should be 20 to 256 characters".to_string());
    }
    if !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("the key contains spaces or invalid characters".to_string());
    }
    Ok(key.to_string())
}

#[cfg(target_os = "macos")]
mod keychain {
    use super::{KEYCHAIN_ACCOUNT, KEYCHAIN_SERVICE};
    use security_framework::passwords;

    pub fn get() -> Option<String> {
        passwords::get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }

    pub fn set(key: &str) -> Result<(), String> {
        passwords::set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, key.as_bytes())
            .map_err(|e| format!("could not store the key in the Keychain: {e}"))
    }

    pub fn delete() -> Result<(), String> {
        match passwords::delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) {
            Ok(()) => Ok(()),
            // errSecItemNotFound: nothing to delete.
            Err(e) if e.code() == -25300 => Ok(()),
            Err(e) => Err(format!("could not remove the key from the Keychain: {e}")),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod keychain {
    pub fn get() -> Option<String> {
        None
    }
    pub fn set(_key: &str) -> Result<(), String> {
        Err("secure key storage is only implemented on macOS; set ANTHROPIC_API_KEY".into())
    }
    pub fn delete() -> Result<(), String> {
        Ok(())
    }
}

pub fn load_api_key() -> Option<(String, KeySource)> {
    if let Some(key) = keychain::get().and_then(|k| validate_api_key(&k).ok()) {
        return Some((key, KeySource::Keychain));
    }
    std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .and_then(|k| validate_api_key(&k).ok())
        .map(|k| (k, KeySource::Environment))
}

pub fn store_api_key(raw: &str) -> Result<(), String> {
    keychain::set(&validate_api_key(raw)?)
}

pub fn delete_api_key() -> Result<(), String> {
    keychain::delete()
}

// --- runs ------------------------------------------------------------------

/// Start the model-driven run for a pending task on a dedicated thread.
pub fn start_run(
    shared: SharedState,
    bus: EventBus,
    supervisor: AgentSupervisor,
    task: TaskId,
) -> Result<(), String> {
    let (key, _) = load_api_key().ok_or("Connect a model in Settings first.")?;
    let settings = load_settings();
    let objective = {
        let guard = lock_state(&shared);
        let t = guard.tasks.get(&task).map_err(|e| e.to_string())?;
        if t.status != TaskStatus::Pending {
            return Err(format!("task is {:?}, not pending", t.status));
        }
        t.title.clone()
    };
    let cancel = supervisor.claim(task)?;
    let slot = supervisor.clone();
    std::thread::Builder::new()
        .name(format!("pegoles-task-{task}"))
        .spawn(move || {
            // Frees the run slot even if the run panics.
            let _slot = SlotGuard(slot, task);
            let mut planner =
                AnthropicPlanner::new(AnthropicConfig::new(key, &settings.model, &settings.effort));
            let computer = CoreComputer::new(AppCore(shared), bus);
            let _report = run_task(
                task,
                &objective,
                &mut planner,
                &computer,
                &RunLimits::default(),
                &cancel,
            );
        })
        .map(|_| ())
        .map_err(|e| {
            supervisor.release(task);
            format!("could not start the task thread: {e}")
        })
}

struct SlotGuard(AgentSupervisor, TaskId);

impl Drop for SlotGuard {
    fn drop(&mut self) {
        self.0.release(self.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_shape_is_checked_without_echoing_it() {
        assert!(validate_api_key("  sk-ant-api03-abcdefghijklmnopqrstuvwxyz  ").is_ok());
        assert!(validate_api_key("short").is_err());
        let err = validate_api_key("sk-ant-api03-abc def-ghijklmnopqrstu").unwrap_err();
        assert!(!err.contains("sk-ant"));
        assert!(validate_api_key("sk-ant-api03-abcdefghijk\r\nX-Evil: 1").is_err());
    }

    #[test]
    fn settings_reject_unknown_models_and_efforts() {
        assert!(ModelSettings::default().validate().is_ok());
        let bad = ModelSettings {
            model: "gpt-9".into(),
            effort: "high".into(),
        };
        assert!(bad.validate().is_err());
        let bad = ModelSettings {
            model: DEFAULT_MODEL.into(),
            effort: "ludicrous".into(),
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn supervisor_allows_one_run_and_cancels_it() {
        let sup = AgentSupervisor::default();
        let a = TaskId::new();
        let cancel = sup.claim(a).unwrap();
        assert!(sup.claim(TaskId::new()).is_err());
        assert!(!sup.cancel(Some(TaskId::new())));
        assert!(sup.cancel(Some(a)));
        assert!(cancel.is_cancelled());
        sup.release(a);
        assert!(sup.active().is_none());
        assert!(sup.claim(TaskId::new()).is_ok());
    }
}
