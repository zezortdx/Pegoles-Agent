//! Tauri commands: thin wrappers over Pegoles Core.
//! Every state change goes through the registry; the frontend only renders.
//!
//! THREADING (Phase 4, critical): the macOS VM runs in-process and
//! Virtualization.framework needs the MAIN queue, while Tauri 2 runs sync
//! commands ON the main thread. So every command that locks `AppState`
//! (and can therefore reach the ComputerBackend or the display) is an
//! `async fn` that does its work inside `spawn_blocking` via `with_state`:
//! the main thread never blocks on a VM reply or on the AppState lock, and
//! the lock is never held across an `.await`. Only lock-free commands stay
//! sync (`get_host_capabilities`).

use crate::agent::AgentSupervisor;
use crate::state::AppState;
use pegoles_computer::{DisplayGeometry, EffectsRecommendation, ImageStatus, PerformanceProfile};
use pegoles_core::{ComputerView, CoreError, DisplayBounds, GeometryOutcome};
use pegoles_protocol::{AgentEvent, AgentTask, ComputerInfo, ComputerState, GuestRuntimeState};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::Manager;

pub type SharedState = Arc<Mutex<AppState>>;

/// Cancel handle of the running input script, kept OUTSIDE the AppState
/// lock: a script holds that lock for its whole run, so Take Control,
/// Pause and Stop trip this first; the script then stops at its next
/// primitive or wait slice and releases the lock (and pressed state).
#[derive(Clone, Default)]
pub struct ScriptCancel(Arc<Mutex<Option<pegoles_core::CancellationToken>>>);

impl ScriptCancel {
    #[cfg(debug_assertions)]
    fn arm(&self) -> pegoles_core::CancellationToken {
        let token = pegoles_core::CancellationToken::new();
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(token.clone());
        }
        token
    }

    #[cfg(debug_assertions)]
    fn disarm(&self) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = None;
        }
    }

    /// Cancel the running script, if any. Lock-free w.r.t. AppState.
    pub fn trip(&self) {
        if let Ok(slot) = self.0.lock() {
            if let Some(token) = slot.as_ref() {
                token.cancel();
            }
        }
    }
}

/// Lock the app state, recovering from poisoning: a panic in one
/// command must not turn every later command (including Stop) into a
/// failure. Core re-validates its own invariants on the next call.
pub fn lock_state(shared: &SharedState) -> std::sync::MutexGuard<'_, AppState> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// Run `f` with the AppState lock held, on Tauri's blocking pool (never
/// the main thread). History is synced after every command so
/// `list_events` mirrors the live stream.
async fn with_state<T, F>(shared: SharedState, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut AppState) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = lock_state(&shared);
        let result = f(&mut guard);
        guard.sync_history();
        result
    })
    .await
    .map_err(|e| format!("background task failed: {e}"))?
}

/// Last computed status, served while a long operation (VM start, an
/// agent action) holds the app state. Status reads never queue behind
/// the VM.
#[derive(Clone, Default)]
pub struct StatusCache(Arc<Mutex<Option<StatusPayload>>>);

impl StatusCache {
    pub fn store(&self, status: StatusPayload) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(status);
    }

    fn get(&self) -> Option<StatusPayload> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Pump Core (guest session, display, lifecycle) and refresh the status
/// cache if the state is free right now; never waits for it.
pub fn pump_if_idle(shared: &SharedState, cache: &StatusCache, agent: &AgentSupervisor) -> bool {
    let mut guard = match shared.try_lock() {
        Ok(g) => g,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return false,
    };
    guard.registry.pump();
    guard.sync_history();
    cache.store(status_of(&guard, agent));
    true
}

fn err(e: CoreError) -> String {
    e.to_string()
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct StatusPayload {
    pub core: &'static str,
    /// `configured` when an API key is available, else `not_configured`.
    pub model: &'static str,
    /// The task an agent run is working on right now, if any.
    pub active_task: Option<String>,
    pub backend: &'static str,
    pub computer_created: bool,
    pub computer_state: Option<ComputerState>,
    pub computer_id: Option<String>,
    pub image_status: &'static str,
    pub spec_os: &'static str,
    pub spec_arch: &'static str,
    pub spec_vcpus: u8,
    pub spec_ram_mb: u32,
    pub guest_state: &'static str,
    pub guest_ready_ms: Option<u64>,
    /// Why the platform display adapter failed to install (startup).
    pub display_setup_error: Option<String>,
    /// Phase 5 agent input plane: guest advertised input support and the
    /// session is Ready (UI gates agent affordances on this).
    pub input_available: bool,
    /// A deterministic agent sequence currently owns control.
    pub agent_busy: bool,
    /// Phase 4 computer surface: viewport_state, viewport_issue,
    /// display_available, display_config, display_backend,
    /// display_attached, graphical_session, control_owner,
    /// display_ready_ms, display_error (flattened).
    #[serde(flatten)]
    pub view: ComputerView,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct ComputerPayload {
    pub info: Option<ComputerInfo>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct ImageStatusPayload {
    pub status: &'static str,
    pub preparing: bool,
    pub stage: Option<String>,
    pub downloaded: u64,
    pub total: u64,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct BootLogPayload {
    pub available: bool,
    pub total_lines: usize,
    pub tail: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct GuestInfoPayload {
    pub available: bool,
    pub info: Option<pegoles_guest_proto::SystemInfo>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct GuestPingPayload {
    pub latency_ms: u64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct PumpPayload {
    /// Events published by THIS pump (also on `pegoles://event`).
    pub events: Vec<AgentEvent>,
    pub status: StatusPayload,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct DisplayGeometryPayload {
    pub outcome: GeometryOutcome,
}

fn parse_profile(profile: Option<&str>) -> PerformanceProfile {
    match profile {
        Some("eco") => PerformanceProfile::Eco,
        Some("performance") => PerformanceProfile::Performance,
        Some("custom") => PerformanceProfile::Custom,
        _ => PerformanceProfile::Balanced,
    }
}

/// Governor recommendation for this host (advisory; create() still takes
/// explicit configs). Lets the UI/bench avoid host-starving defaults.
#[tauri::command]
pub async fn suggested_config(
    state: tauri::State<'_, SharedState>,
    profile: Option<String>,
) -> Result<pegoles_computer::ComputerRecommendation, String> {
    with_state(state.inner().clone(), move |s| {
        Ok(s.registry
            .suggested_config(parse_profile(profile.as_deref())))
    })
    .await
}

/// UI effects tier for this host (Full / Reduced / Minimal + reasons).
/// Advisory: Settings may override it. `prefers_reduced_motion` is
/// reported back as a reason and never changes the tier.
#[tauri::command(async)]
pub fn suggested_effects(
    prefers_reduced_motion: Option<bool>,
    profile: Option<String>,
) -> EffectsRecommendation {
    let host = pegoles_computer::detect_host_resources();
    pegoles_computer::recommend_effects(
        &host,
        parse_profile(profile.as_deref()),
        prefers_reduced_motion,
    )
}

/// System display accessibility options the web view can't read itself:
/// WKWebView ignores `prefers-reduced-transparency`, so glass surfaces ask
/// here and turn solid when Reduce Transparency is on.
#[derive(serde::Serialize)]
pub struct AccessibilityDisplay {
    pub reduce_transparency: bool,
    pub increase_contrast: bool,
}

#[tauri::command]
pub fn accessibility_display() -> AccessibilityDisplay {
    #[cfg(target_os = "macos")]
    {
        let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
        AccessibilityDisplay {
            reduce_transparency: workspace.accessibilityDisplayShouldReduceTransparency(),
            increase_contrast: workspace.accessibilityDisplayShouldIncreaseContrast(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        AccessibilityDisplay {
            reduce_transparency: false,
            increase_contrast: false,
        }
    }
}

/// Platform and virtualization capabilities of THIS host, reported by
/// Rust. The frontend never detects the platform itself (no user-agent).
/// Lock-free (never touches AppState), so it may stay sync.
#[tauri::command]
pub fn get_host_capabilities() -> pegoles_computer::HostCapabilities {
    pegoles_computer::host_capabilities()
}

fn guest_state_str(s: GuestRuntimeState) -> &'static str {
    match s {
        GuestRuntimeState::Unavailable => "unavailable",
        GuestRuntimeState::Waiting => "waiting",
        GuestRuntimeState::Connecting => "connecting",
        GuestRuntimeState::Ready => "ready",
        GuestRuntimeState::Disconnected => "disconnected",
        GuestRuntimeState::Incompatible => "incompatible",
        GuestRuntimeState::Error => "error",
    }
}

fn image_status_str(s: ImageStatus) -> &'static str {
    match s {
        ImageStatus::Missing => "missing",
        ImageStatus::Downloading => "downloading",
        ImageStatus::Ready => "ready",
        ImageStatus::Invalid => "invalid",
    }
}

fn status_of(state: &AppState, agent: &AgentSupervisor) -> StatusPayload {
    let info = state.registry.info();
    let cfg = info.as_ref().map(|i| i.config.clone());
    let computer_state = info.as_ref().map(|i| i.state);
    StatusPayload {
        core: "running",
        model: if state.model_configured {
            "configured"
        } else {
            "not_configured"
        },
        active_task: agent.active().map(|t| t.to_string()),
        backend: state.registry.backend_kind().as_str(),
        computer_created: info.is_some(),
        computer_state,
        computer_id: info.map(|i| i.id.to_string()),
        image_status: image_status_str(state.registry.image_status()),
        spec_os: "Debian 13",
        spec_arch: state.registry.guest_arch().as_str(),
        spec_vcpus: cfg.as_ref().map(|c| c.vcpus).unwrap_or(2),
        spec_ram_mb: cfg.as_ref().map(|c| c.memory_mb).unwrap_or(1536),
        guest_state: guest_state_str(state.registry.guest_state()),
        guest_ready_ms: state.registry.guest_ready_ms(),
        display_setup_error: state.display_error.clone(),
        input_available: state.registry.input_available(),
        agent_busy: state.registry.input_status().agent_busy,
        view: state
            .registry
            .computer_view_for(computer_state, state.preparing_image),
    }
}

/// Status after pumping guest + display + lifecycle reconciliation.
/// While a long operation holds the state, returns the cached status
/// (refreshed by the background pump) instead of queueing behind it.
#[tauri::command]
pub async fn get_status(
    state: tauri::State<'_, SharedState>,
    cache: tauri::State<'_, StatusCache>,
    agent: tauri::State<'_, AgentSupervisor>,
) -> Result<StatusPayload, String> {
    let shared = state.inner().clone();
    let cache = cache.inner().clone();
    let agent = agent.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if pump_if_idle(&shared, &cache, &agent) {
            if let Some(status) = cache.get() {
                return Ok(status);
            }
        }
        if let Some(status) = cache.get() {
            return Ok(status);
        }
        let mut guard = lock_state(&shared);
        guard.registry.pump();
        guard.sync_history();
        let status = status_of(&guard, &agent);
        cache.store(status.clone());
        Ok(status)
    })
    .await
    .map_err(|e| format!("background task failed: {e}"))?
}

/// Event-driven refresh: call on `pegoles://display-activity` (native
/// display wake) or any other nudge. Same work as `get_status`, plus the
/// events this pump published.
#[tauri::command]
pub async fn pump(
    state: tauri::State<'_, SharedState>,
    agent: tauri::State<'_, AgentSupervisor>,
) -> Result<PumpPayload, String> {
    let agent = agent.inner().clone();
    with_state(state.inner().clone(), move |s| {
        let events = s.registry.pump();
        Ok(PumpPayload {
            events,
            status: status_of(s, &agent),
        })
    })
    .await
}

/// Create the computer with the product default config: headless, or a
/// DesktopLarge framebuffer when the ready image is graphical and this
/// host can show it (decided by Core).
#[tauri::command]
pub async fn create_computer(
    state: tauri::State<'_, SharedState>,
) -> Result<ComputerPayload, String> {
    with_state(state.inner().clone(), |s| {
        s.registry.create_default().map_err(err)?;
        Ok(ComputerPayload {
            info: s.registry.info(),
        })
    })
    .await
}

fn lifecycle(
    s: &mut AppState,
    op: impl FnOnce(&mut pegoles_core::ComputerRegistry) -> pegoles_core::Result<ComputerState>,
) -> Result<ComputerPayload, String> {
    op(&mut s.registry).map_err(err)?;
    s.registry.pump();
    Ok(ComputerPayload {
        info: s.registry.info(),
    })
}

#[tauri::command]
pub async fn start_computer(
    state: tauri::State<'_, SharedState>,
) -> Result<ComputerPayload, String> {
    with_state(state.inner().clone(), |s| lifecycle(s, |r| r.start())).await
}

#[tauri::command]
pub async fn pause_computer(
    state: tauri::State<'_, SharedState>,
    script: tauri::State<'_, ScriptCancel>,
    agent: tauri::State<'_, AgentSupervisor>,
) -> Result<ComputerPayload, String> {
    script.trip();
    // The agent stops first, before this command waits for the state.
    agent.cancel(None);
    with_state(state.inner().clone(), |s| lifecycle(s, |r| r.pause())).await
}

#[tauri::command]
pub async fn resume_computer(
    state: tauri::State<'_, SharedState>,
) -> Result<ComputerPayload, String> {
    with_state(state.inner().clone(), |s| lifecycle(s, |r| r.resume())).await
}

#[tauri::command]
pub async fn stop_computer(
    state: tauri::State<'_, SharedState>,
    script: tauri::State<'_, ScriptCancel>,
    agent: tauri::State<'_, AgentSupervisor>,
) -> Result<ComputerPayload, String> {
    script.trip();
    // The agent stops first, before this command waits for the state.
    agent.cancel(None);
    with_state(state.inner().clone(), |s| lifecycle(s, |r| r.stop())).await
}

#[tauri::command]
pub async fn list_events(state: tauri::State<'_, SharedState>) -> Result<Vec<AgentEvent>, String> {
    with_state(state.inner().clone(), |s| {
        s.registry.pump();
        s.sync_history();
        Ok(s.event_log.iter().cloned().collect())
    })
    .await
}

#[tauri::command]
pub async fn get_image_status(
    state: tauri::State<'_, SharedState>,
) -> Result<ImageStatusPayload, String> {
    with_state(state.inner().clone(), |s| {
        Ok(ImageStatusPayload {
            status: image_status_str(s.registry.image_status()),
            preparing: s.preparing_image,
            stage: s.image_stage.clone(),
            downloaded: s.image_downloaded,
            total: s.image_total,
            error: s.image_error.clone(),
        })
    })
    .await
}

/// Start base-image preparation on a background thread (download +
/// verify + extract with real byte progress). Returns immediately.
/// Debug builds only: this fetches the official Debian image that image
/// builders start from; it does not install the sealed Pegoles image a
/// computer boots, so the product UI never offers it.
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn prepare_image(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<ImageStatusPayload, String> {
    let shared: SharedState = state.inner().clone();
    // Decide + claim the slot under one lock (no double start).
    let images_dir = with_state(shared.clone(), |s| {
        if matches!(s.registry.image_status(), ImageStatus::Ready) {
            return Ok(None);
        }
        if s.preparing_image {
            return Err("image preparation already in progress".to_string());
        }
        s.preparing_image = true;
        s.image_error = None;
        Ok(Some(s.registry.images_dir()))
    })
    .await?;
    let Some(images_dir) = images_dir else {
        return Ok(ImageStatusPayload {
            status: "ready",
            preparing: false,
            stage: None,
            downloaded: 0,
            total: 0,
            error: None,
        });
    };
    let worker_state = shared.clone();
    std::thread::spawn(move || run_image_preparation(app, worker_state, images_dir));
    with_state(shared, |s| {
        Ok(ImageStatusPayload {
            status: image_status_str(s.registry.image_status()),
            preparing: true,
            stage: s.image_stage.clone(),
            downloaded: s.image_downloaded,
            total: s.image_total,
            error: None,
        })
    })
    .await
}

/// Background worker for `prepare_image` (no lock held during the
/// multi-minute download; progress updates re-lock briefly).
#[cfg(debug_assertions)]
fn run_image_preparation(
    app: tauri::AppHandle,
    shared: SharedState,
    images_dir: std::path::PathBuf,
) {
    use tauri::Emitter;
    let manager = pegoles_computer::ComputerImageManager::new(images_dir);
    let result = manager.prepare(&mut |stage, downloaded, total| {
        {
            let mut s = lock_state(&shared);
            s.image_stage = Some(stage.as_str().to_string());
            s.image_downloaded = downloaded;
            s.image_total = total;
        }
        let _ = app.emit(
            "pegoles://image-progress",
            serde_json::json!({
                "stage": stage.as_str(),
                "downloaded": downloaded,
                "total": total,
            }),
        );
    });
    let mut s = lock_state(&shared);
    s.preparing_image = false;
    match result {
        Ok(_) => {
            s.image_stage = None;
            let _ = app.emit(
                "pegoles://image-progress",
                serde_json::json!({"stage": "ready"}),
            );
        }
        Err(e) => {
            s.image_error = Some(e.to_string());
            let _ = app.emit(
                "pegoles://image-progress",
                serde_json::json!({"stage": "error", "error": e.to_string()}),
            );
        }
    }
}

#[tauri::command]
pub async fn read_boot_log(state: tauri::State<'_, SharedState>) -> Result<BootLogPayload, String> {
    with_state(state.inner().clone(), |s| {
        match s.registry.read_boot_log(50) {
            Ok(log) => Ok(BootLogPayload {
                available: log.available,
                total_lines: log.total_lines,
                tail: log.tail,
            }),
            Err(CoreError::NoBootLog) => Ok(BootLogPayload {
                available: false,
                total_lines: 0,
                tail: vec![],
            }),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
}

pub fn shared_state(app: &tauri::AppHandle) -> SharedState {
    app.state::<SharedState>().inner().clone()
}

/// Last SystemInfo reported by the guest (no UI event; queried on demand).
#[tauri::command]
pub async fn guest_info(state: tauri::State<'_, SharedState>) -> Result<GuestInfoPayload, String> {
    with_state(state.inner().clone(), |s| {
        s.registry.pump();
        let info = s.registry.guest_info();
        Ok(GuestInfoPayload {
            available: info.is_some(),
            info,
        })
    })
    .await
}

/// Blocking Ping -> Pong round-trip against the guest (latency in ms).
/// Fails fast when the guest is not Ready.
#[tauri::command]
pub async fn guest_ping(state: tauri::State<'_, SharedState>) -> Result<GuestPingPayload, String> {
    with_state(state.inner().clone(), |s| {
        let latency_ms = s
            .registry
            .guest_ping(std::time::Duration::from_secs(15))
            .map_err(err)?;
        s.registry.pump();
        Ok(GuestPingPayload { latency_ms })
    })
    .await
}

// --- Phase 4: display + control ---

/// Webview bounds in CSS px + scale, read from the HOST window (never
/// trusted from React). Called outside the AppState lock: the getter
/// round-trips through the main thread, which must never wait on us.
fn webview_bounds(window: &tauri::WebviewWindow) -> Result<DisplayBounds, String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    Ok(DisplayBounds {
        width: f64::from(size.width) / scale,
        height: f64::from(size.height) / scale,
        scale,
    })
}

/// Place (or attach) the native framebuffer at the React-measured slot.
/// Send on layout change / resize / mode change only — never per frame.
#[tauri::command]
pub async fn display_set_geometry(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, SharedState>,
    geometry: DisplayGeometry,
) -> Result<DisplayGeometryPayload, String> {
    let bounds = {
        let window = window.clone();
        tauri::async_runtime::spawn_blocking(move || webview_bounds(&window))
            .await
            .map_err(|e| format!("background task failed: {e}"))??
    };
    with_state(state.inner().clone(), move |s| {
        let outcome = s
            .registry
            .set_display_geometry(geometry, bounds)
            .map_err(err)?;
        Ok(DisplayGeometryPayload { outcome })
    })
    .await
}

/// Remove the framebuffer view (control returns to Pegoles first). It
/// comes back on the next `display_set_geometry`.
#[tauri::command]
pub async fn display_detach(state: tauri::State<'_, SharedState>) -> Result<ComputerView, String> {
    with_state(state.inner().clone(), |s| {
        s.registry.detach_display().map_err(err)?;
        Ok(s.registry.computer_view(s.preparing_image))
    })
    .await
}

/// Take Control: route the HUMAN's keyboard/pointer into the isolated
/// computer. Fails unless the display is ready (viewport `ready`).
#[tauri::command]
pub async fn take_control(
    state: tauri::State<'_, SharedState>,
    script: tauri::State<'_, ScriptCancel>,
    agent: tauri::State<'_, AgentSupervisor>,
) -> Result<ComputerView, String> {
    script.trip();
    // The agent stops first, before this command waits for the state.
    agent.cancel(None);
    with_state(state.inner().clone(), |s| {
        s.registry.take_control().map_err(err)?;
        Ok(s.registry.computer_view(s.preparing_image))
    })
    .await
}

/// Return to Pegoles: stop routing input to the computer. Idempotent.
#[tauri::command]
pub async fn return_control(state: tauri::State<'_, SharedState>) -> Result<ComputerView, String> {
    with_state(state.inner().clone(), |s| {
        s.registry.return_control().map_err(err)?;
        Ok(s.registry.computer_view(s.preparing_image))
    })
    .await
}

// --- tasks (created pending; `run_task` starts the agent) ---

#[tauri::command]
pub async fn create_task(
    state: tauri::State<'_, SharedState>,
    title: String,
) -> Result<AgentTask, String> {
    with_state(state.inner().clone(), move |s| {
        s.tasks.submit_task(&title).map_err(err)
    })
    .await
}

/// All tasks, oldest first.
#[tauri::command]
pub async fn list_tasks(state: tauri::State<'_, SharedState>) -> Result<Vec<AgentTask>, String> {
    with_state(state.inner().clone(), |s| Ok(s.tasks.list())).await
}

// --- Eyes & Hands: direct actions (Design Lab; debug builds) ---

/// Execute ONE structured action inside Pegoles Computer (policy →
/// control → guest dispatch → lifecycle events). `task_id` attaches the
/// action to a task for audit; when omitted an ephemeral id is used and
/// the audit still records the computer + verb (no fake task linkage).
/// Debug builds only (Design Lab); the product path is `run_task`.
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn execute_action(
    state: tauri::State<'_, SharedState>,
    task_id: Option<String>,
    action: pegoles_protocol::ComputerAction,
    observe_after: Option<bool>,
) -> Result<pegoles_protocol::ActionResult, String> {
    with_state(state.inner().clone(), move |s| {
        let task_id = task_id
            .as_deref()
            .and_then(|t| t.parse::<pegoles_protocol::TaskId>().ok())
            .unwrap_or_default();
        let (result, _) = s.registry.execute_action(
            task_id,
            action,
            observe_after.unwrap_or(false),
            &pegoles_policy::PolicyContext::default(),
            &pegoles_core::CancellationToken::new(),
            true,
        );
        Ok(result)
    })
    .await
}

/// Cooperative cancel for the running agent sequence (Take Control also
/// cancels; this is the explicit dev path). Releases pressed state.
#[tauri::command]
pub async fn cancel_agent_input(
    state: tauri::State<'_, SharedState>,
    script: tauri::State<'_, ScriptCancel>,
    agent: tauri::State<'_, AgentSupervisor>,
) -> Result<(), String> {
    script.trip();
    agent.cancel(None);
    with_state(state.inner().clone(), |s| {
        s.registry.cancel_agent_input("user requested cancel");
        Ok(())
    })
    .await
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct CapturedFramePayload {
    pub meta: pegoles_protocol::ObservedFrameMeta,
    /// PNG-encoded frame (manual debugging path).
    pub png_base64: String,
}

/// Capture the guest framebuffer on demand (VM display only, never the
/// host screen). Pixels return out-of-band here; the event stream only
/// ever carries frame metadata.
#[tauri::command]
pub async fn capture_screen(
    state: tauri::State<'_, SharedState>,
) -> Result<CapturedFramePayload, String> {
    let (meta, bytes) = with_state(state.inner().clone(), |s| {
        let mut out = Vec::new();
        let meta = s.registry.capture_frame("preview", &mut out)?;
        let (_, bytes) = s
            .registry
            .last_frame_bytes()
            .ok_or_else(|| "frame cache lost".to_string())?;
        Ok((meta, bytes))
    })
    .await?;
    // Encode with the app state released.
    tauri::async_runtime::spawn_blocking(move || {
        let png = pegoles_computer::encode_png_rgb_fast(meta.width_px, meta.height_px, &bytes)
            .map_err(|e| e.to_string())?;
        Ok(CapturedFramePayload {
            meta,
            png_base64: pegoles_computer::base64_png(&png),
        })
    })
    .await
    .map_err(|e| format!("background task failed: {e}"))?
}

/// Run a deterministic action script (dev/test only). Acquires Agent
/// control once, aborts on first terminal failure, always releases.
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn run_input_script(
    state: tauri::State<'_, SharedState>,
    script: tauri::State<'_, ScriptCancel>,
    task_id: Option<String>,
    steps: Vec<pegoles_core::ScriptStep>,
) -> Result<pegoles_core::ScriptReport, String> {
    let handle = script.inner().clone();
    let cancel = handle.arm();
    let result = with_state(state.inner().clone(), move |s| {
        let task_id = task_id
            .as_deref()
            .and_then(|t| t.parse::<pegoles_protocol::TaskId>().ok())
            .unwrap_or_default();
        let (report, _) = s.registry.run_script(
            task_id,
            &steps,
            &pegoles_policy::PolicyContext::default(),
            &cancel,
        );
        Ok(report)
    })
    .await;
    handle.disarm();
    result
}

/// The deterministic smoke demo steps (precondition: the Pegoles input
/// fixture runs fullscreen in the guest).
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn demo_script_steps() -> Result<Vec<pegoles_core::ScriptStep>, String> {
    Ok(pegoles_core::pegoles_demo_script())
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct InputStatusPayload {
    pub available: bool,
    pub frame_available: bool,
    pub agent_busy: bool,
    pub pressed_clean: bool,
    pub audit_len: usize,
    pub last_frame: Option<pegoles_protocol::ObservedFrameMeta>,
    pub unavailable: Vec<UnavailabilityRow>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct UnavailabilityRow {
    pub capability: String,
    pub reason: String,
}

#[tauri::command]
pub async fn input_status(
    state: tauri::State<'_, SharedState>,
) -> Result<InputStatusPayload, String> {
    with_state(state.inner().clone(), |s| {
        let st = s.registry.input_status();
        Ok(InputStatusPayload {
            available: st.available,
            frame_available: st.frame_available,
            agent_busy: st.agent_busy,
            pressed_clean: st.pressed_clean,
            audit_len: st.audit_len,
            last_frame: st.last_frame,
            unavailable: st
                .unavailable
                .into_iter()
                .map(|d| UnavailabilityRow {
                    capability: d.capability,
                    reason: d.reason,
                })
                .collect(),
        })
    })
    .await
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct AuditRow {
    pub at: String,
    pub action_id: String,
    pub verb: &'static str,
    pub decision: String,
    pub outcome: String,
    pub duration_ms: u64,
    pub redacted: bool,
    pub text_len: Option<usize>,
}

/// Recent audit rows, newest last. Content-free by construction (verbs +
/// lengths, never typed text).
#[tauri::command]
pub async fn input_audit(
    state: tauri::State<'_, SharedState>,
    limit: Option<usize>,
) -> Result<Vec<AuditRow>, String> {
    with_state(state.inner().clone(), move |s| {
        let limit = limit.unwrap_or(50).min(500);
        Ok(s.registry
            .audit_rows(limit)
            .into_iter()
            .map(|e| AuditRow {
                at: e.at.to_rfc3339(),
                action_id: e.action_id.to_string(),
                verb: e.verb,
                decision: e.decision,
                outcome: e.outcome,
                duration_ms: e.duration_ms,
                redacted: e.redacted,
                text_len: e.text_len,
            })
            .collect())
    })
    .await
}

/// Return the computer to the sealed image (fresh disk, same identity).
/// Refused while an agent run is using it.
#[tauri::command]
pub async fn reset_computer(
    state: tauri::State<'_, SharedState>,
    agent: tauri::State<'_, AgentSupervisor>,
) -> Result<ComputerPayload, String> {
    if agent.active().is_some() {
        return Err("stop the running task before resetting the computer".to_string());
    }
    with_state(state.inner().clone(), |s| lifecycle(s, |r| r.reset())).await
}

/// Destroy the computer and its private disk. Refused during a run.
#[tauri::command]
pub async fn destroy_computer(
    state: tauri::State<'_, SharedState>,
    agent: tauri::State<'_, AgentSupervisor>,
) -> Result<ComputerPayload, String> {
    if agent.active().is_some() {
        return Err("stop the running task before removing the computer".to_string());
    }
    with_state(state.inner().clone(), |s| {
        s.registry.destroy().map_err(err)?;
        Ok(ComputerPayload { info: None })
    })
    .await
}

// --- agent runs + model settings --------------------------------------

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct ModelSettingsPayload {
    /// Whether an API key is available. The key itself never leaves Rust.
    pub configured: bool,
    pub key_source: Option<crate::agent::KeySource>,
    pub model: String,
    pub effort: String,
    pub models: Vec<&'static str>,
    pub efforts: Vec<&'static str>,
}

fn model_settings_payload() -> ModelSettingsPayload {
    let settings = crate::agent::load_settings();
    let key_source = crate::agent::load_api_key().map(|(_, source)| source);
    ModelSettingsPayload {
        configured: key_source.is_some(),
        key_source,
        model: settings.model,
        effort: settings.effort,
        models: pegoles_agent::anthropic::SUPPORTED_MODELS.to_vec(),
        efforts: pegoles_agent::anthropic::EFFORTS.to_vec(),
    }
}

/// Keychain + settings file I/O happen off the main thread; the cached
/// `model_configured` flag feeds `get_status`.
async fn settings_op(
    shared: SharedState,
    op: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<ModelSettingsPayload, String> {
    tauri::async_runtime::spawn_blocking(move || {
        op()?;
        let payload = model_settings_payload();
        lock_state(&shared).model_configured = payload.configured;
        Ok(payload)
    })
    .await
    .map_err(|e| format!("background task failed: {e}"))?
}

#[tauri::command]
pub async fn get_model_settings(
    state: tauri::State<'_, SharedState>,
) -> Result<ModelSettingsPayload, String> {
    settings_op(state.inner().clone(), || Ok(())).await
}

/// Store the Anthropic API key in the Keychain. The key is validated for
/// shape only and is never echoed back, logged, or sent to the guest.
#[tauri::command]
pub async fn set_api_key(
    state: tauri::State<'_, SharedState>,
    key: String,
) -> Result<ModelSettingsPayload, String> {
    settings_op(state.inner().clone(), move || {
        crate::agent::store_api_key(&key)
    })
    .await
}

#[tauri::command]
pub async fn clear_api_key(
    state: tauri::State<'_, SharedState>,
) -> Result<ModelSettingsPayload, String> {
    settings_op(state.inner().clone(), crate::agent::delete_api_key).await
}

#[tauri::command]
pub async fn set_model_settings(
    state: tauri::State<'_, SharedState>,
    model: String,
    effort: String,
) -> Result<ModelSettingsPayload, String> {
    settings_op(state.inner().clone(), move || {
        crate::agent::save_settings(&crate::agent::ModelSettings { model, effort })
    })
    .await
}

/// Start the agent on a pending task. The run prepares the computer
/// (create/boot/resume as needed), then loops observe → plan → act, with
/// every action checked by Pegoles Policy.
#[tauri::command]
pub async fn run_task(
    state: tauri::State<'_, SharedState>,
    agent: tauri::State<'_, AgentSupervisor>,
    task_id: String,
) -> Result<(), String> {
    let task: pegoles_protocol::TaskId = task_id.parse().map_err(|_| "invalid task id")?;
    let shared = state.inner().clone();
    let agent = agent.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let bus = lock_state(&shared).bus.clone();
        crate::agent::start_run(shared, bus, agent, task)
    })
    .await
    .map_err(|e| format!("background task failed: {e}"))?
}

/// Stop a task: cancels its run (the current action is interrupted and
/// held input released), or cancels it outright if it never started.
#[tauri::command]
pub async fn cancel_task(
    state: tauri::State<'_, SharedState>,
    agent: tauri::State<'_, AgentSupervisor>,
    script: tauri::State<'_, ScriptCancel>,
    task_id: String,
) -> Result<(), String> {
    let task: pegoles_protocol::TaskId = task_id.parse().map_err(|_| "invalid task id")?;
    if agent.cancel(Some(task)) {
        script.trip();
        return Ok(());
    }
    with_state(state.inner().clone(), move |s| {
        s.tasks.cancel_task(&task).map(|_| ()).map_err(err)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use pegoles_core::{BackendKind, ComputerRegistry};

    #[test]
    fn script_cancel_trips_only_the_armed_script() {
        let handle = ScriptCancel::default();
        handle.trip(); // nothing armed: no-op
        let token = handle.arm();
        assert!(!token.is_cancelled());
        handle.clone().trip();
        assert!(token.is_cancelled());
        handle.disarm();
        let next = handle.arm();
        assert!(!next.is_cancelled(), "a new script starts uncancelled");
    }

    fn state() -> AppState {
        AppState::with_registry(|bus| {
            ComputerRegistry::with_dirs(
                bus.clone(),
                BackendKind::Mock,
                std::env::temp_dir().join("pegoles-desktop-status"),
            )
        })
    }

    #[test]
    fn status_payload_flattens_the_computer_view() {
        let mut s = state();
        let v = serde_json::to_value(status_of(&s, &AgentSupervisor::default())).unwrap();
        for key in [
            "computer_state",
            "guest_state",
            "viewport_state",
            "viewport_issue",
            "display_available",
            "display_config",
            "display_backend",
            "display_attached",
            "graphical_session",
            "control_owner",
            "display_ready_ms",
            "display_error",
            "display_setup_error",
            "input_available",
            "agent_busy",
        ] {
            assert!(v.get(key).is_some(), "missing {key}: {v}");
        }
        assert_eq!(v["viewport_state"], "off");
        assert_eq!(v["display_backend"], "unavailable");
        assert_eq!(v["control_owner"], "none");
        assert_eq!(v["graphical_session"]["state"], "unavailable");
        s.registry.create_default().unwrap();
        s.registry.start().unwrap();
        let v = serde_json::to_value(status_of(&s, &AgentSupervisor::default())).unwrap();
        assert_eq!(v["viewport_state"], "ready");
        assert_eq!(v["display_available"], false);
        assert_eq!(v["display_config"], serde_json::Value::Null);
    }

    #[test]
    fn geometry_payload_shape() {
        let v = serde_json::to_value(DisplayGeometryPayload {
            outcome: GeometryOutcome::Deferred,
        })
        .unwrap();
        assert_eq!(v, serde_json::json!({"outcome": "deferred"}));
        // The command argument deserializes from the documented shape.
        let g: DisplayGeometry = serde_json::from_value(serde_json::json!({
            "rect": {"x": 10.0, "y": 20.0, "width": 640.0, "height": 400.0},
            "visible": true,
            "animate_ms": 260
        }))
        .unwrap();
        assert_eq!(g.animate_ms, 260);
        let g: DisplayGeometry = serde_json::from_value(serde_json::json!({
            "rect": {"x": 0, "y": 0, "width": 1, "height": 1},
            "visible": false
        }))
        .unwrap();
        assert_eq!(g.animate_ms, 0, "animate_ms is optional");
    }
}
