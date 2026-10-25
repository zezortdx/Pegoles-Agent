//! Agent runs for the desktop app: the Core bridge the orchestrator
//! drives, the single active run, which planner a run uses (Pegoles
//! Local by default, a cloud provider only if the user chose one), and
//! the cloud API key storage.
//!
//! Every provider is an untrusted planner behind the same `Planner`
//! trait: the runner, Core's executor and Pegoles Policy are identical
//! whichever one proposes the actions.
//!
//! The API key lives in the macOS Keychain (or `ANTHROPIC_API_KEY`). It
//! never reaches the webview (the UI only learns whether one is set), is
//! never logged, never enters the guest, and never reaches the local
//! model worker (which runs with a cleared environment): it is only an
//! HTTPS header on requests to the Anthropic API.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pegoles_agent::anthropic::{
    AnthropicConfig, AnthropicPlanner, DEFAULT_EFFORT, DEFAULT_MODEL, EFFORTS, SUPPORTED_MODELS,
};
use pegoles_agent::{
    run_task, CoreAccess, CoreComputer, InternetConfirm, InternetPlan, Planner, RunLimits,
};
use pegoles_core::{CancellationToken, ComputerRegistry, EgressKill, EventBus, TaskManager};
use pegoles_protocol::{TaskId, TaskStatus};
use serde::{Deserialize, Serialize};

use crate::commands::{lock_state, SharedState};
use crate::local::LocalModels;

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
pub struct AgentSupervisor {
    run: Arc<Mutex<Option<(TaskId, CancellationToken)>>>,
    /// Cuts the task's internet session without the app lock (docs/EGRESS.md
    /// kill switch): every Stop, pause, takeover and exit goes through
    /// `cancel`, so the internet ends with the run, not a planner turn later.
    internet: Arc<Mutex<Option<EgressKill>>>,
}

impl AgentSupervisor {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<(TaskId, CancellationToken)>> {
        self.run.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Wire the lock-free internet kill switch (once, at startup).
    pub fn set_internet_kill(&self, kill: EgressKill) {
        *self.internet.lock().unwrap_or_else(|e| e.into_inner()) = Some(kill);
    }

    /// Cut the internet session now, whatever the run is doing.
    pub fn cut_internet(&self, reason: &str) -> bool {
        let kill = self
            .internet
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        kill.is_some_and(|k| k.kill(reason))
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
        let signalled = match self.slot().as_ref() {
            Some((running, cancel)) if task.is_none_or(|t| t == *running) => {
                cancel.cancel();
                true
            }
            _ => false,
        };
        if signalled || task.is_none() {
            self.cut_internet("the task was stopped");
        }
        signalled
    }

    pub fn active(&self) -> Option<TaskId> {
        self.slot().as_ref().map(|(t, _)| *t)
    }
}

// --- settings --------------------------------------------------------------

/// Which planner proposes actions. Local is the default: the core product
/// never needs a cloud account.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Local,
    Anthropic,
}

/// Anthropic model settings (the cloud provider's own choices).
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

/// Everything about "who plans", persisted in `settings.json` (never a
/// secret).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntelligenceSettings {
    pub provider: Provider,
    pub local_model: String,
    pub anthropic: ModelSettings,
}

impl Default for IntelligenceSettings {
    fn default() -> Self {
        Self {
            provider: Provider::Local,
            local_model: pegoles_inference::Catalog::builtin().default_model,
            anthropic: ModelSettings::default(),
        }
    }
}

impl IntelligenceSettings {
    pub fn validate(&self) -> Result<(), String> {
        self.anthropic.validate()?;
        if pegoles_inference::Catalog::builtin()
            .get(&self.local_model)
            .is_none()
        {
            return Err(format!("unknown local model {:?}", self.local_model));
        }
        Ok(())
    }

    /// Read `settings.json`, accepting the pre-local format
    /// (`{"model","effort"}` = Anthropic settings) without switching the
    /// user away from the local default.
    fn parse(raw: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(raw).ok()?;
        let s = if v.get("provider").is_some() {
            serde_json::from_value::<Self>(v).ok()?
        } else {
            Self {
                anthropic: serde_json::from_value::<ModelSettings>(v).ok()?,
                ..Self::default()
            }
        };
        s.validate().is_ok().then_some(s)
    }
}

fn settings_path() -> PathBuf {
    pegoles_computer::pegoles_data_dir().join("settings.json")
}

pub fn load_settings() -> IntelligenceSettings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| IntelligenceSettings::parse(&raw))
        .unwrap_or_default()
}

pub fn save_settings(settings: &IntelligenceSettings) -> Result<(), String> {
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

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "dev.pegoles.agent";
#[cfg(target_os = "macos")]
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
/// Without a way to ask the person (`start_run_with(.., None)`), a task
/// that wants internet runs offline.
pub fn start_run(
    shared: SharedState,
    bus: EventBus,
    supervisor: AgentSupervisor,
    local: LocalModels,
    task: TaskId,
) -> Result<(), String> {
    start_run_with(shared, bus, supervisor, local, task, None)
}

/// [`start_run`] with the native consent for the task's internet access.
pub fn start_run_with(
    shared: SharedState,
    bus: EventBus,
    supervisor: AgentSupervisor,
    local: LocalModels,
    task: TaskId,
    internet_consent: Option<InternetConfirm>,
) -> Result<(), String> {
    let settings = load_settings();
    let (objective, internet) = {
        let guard = lock_state(&shared);
        let t = guard.tasks.get(&task).map_err(|e| e.to_string())?;
        if t.status != TaskStatus::Pending {
            return Err(format!("task is {:?}, not pending", t.status));
        }
        (t.title.clone(), t.internet.clone())
    };
    let confirm: InternetConfirm = internet_consent
        .unwrap_or_else(|| Arc::new(|_| Err("there is no way to confirm it here".to_string())));
    // Build the planner before claiming the run slot: a missing model or
    // key is an immediate, explained refusal.
    let mut planner: Box<dyn Planner> = match settings.provider {
        Provider::Local => Box::new(local.planner(&settings.local_model)?),
        Provider::Anthropic => {
            let (key, _) = load_api_key()
                .ok_or("Connect a model in Settings first: add an Anthropic key or switch to Pegoles Local.")?;
            Box::new(AnthropicPlanner::new(AnthropicConfig::new(
                key,
                &settings.anthropic.model,
                &settings.anthropic.effort,
            )))
        }
    };
    let cancel = supervisor.claim(task)?;
    let slot = supervisor.clone();
    std::thread::Builder::new()
        .name(format!("pegoles-task-{task}"))
        .spawn(move || {
            // Frees the run slot even if the run panics.
            let _slot = SlotGuard(slot, task);
            let _use = local.in_use();
            let mut computer = CoreComputer::new(AppCore(shared), bus);
            if !internet.is_off() {
                computer = computer.with_internet(InternetPlan {
                    task,
                    access: internet,
                    confirm,
                });
            }
            let _report = run_task(
                task,
                &objective,
                planner.as_mut(),
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
        // Belt and braces: however the run ended (even a panic), its
        // internet ends with it before the slot frees for the next task.
        self.0.cut_internet("the task ended");
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
    fn local_is_the_default_and_old_settings_keep_it() {
        assert_eq!(IntelligenceSettings::default().provider, Provider::Local);
        let old = r#"{"model":"claude-sonnet-5","effort":"low"}"#;
        let s = IntelligenceSettings::parse(old).unwrap();
        assert_eq!(s.provider, Provider::Local);
        assert_eq!(s.anthropic.model, "claude-sonnet-5");
        let chosen = serde_json::to_string(&IntelligenceSettings {
            provider: Provider::Anthropic,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            IntelligenceSettings::parse(&chosen).unwrap().provider,
            Provider::Anthropic
        );
        assert!(IntelligenceSettings::parse(r#"{"provider":"openai"}"#).is_none());
        let bad_local = r#"{"provider":"local","local_model":"../../etc","anthropic":{"model":"claude-opus-5","effort":"high"}}"#;
        assert!(IntelligenceSettings::parse(bad_local).is_none());
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
