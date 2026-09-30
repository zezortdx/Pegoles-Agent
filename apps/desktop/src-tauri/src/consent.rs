//! Native confirmation for the two settings that cross the trust boundary:
//! handing tasks to a cloud planner, and storing that planner's API key.
//!
//! Rust asks in a macOS alert that script in the webview can neither see
//! into nor answer, so a compromised page can't opt the user into the
//! cloud or plant its own key. The key itself is typed into the alert's
//! secure field: it never enters the webview's DOM or JavaScript.

use std::sync::Arc;

use pegoles_protocol::{InternetAccess, InternetMode};

use crate::agent::{IntelligenceSettings, Provider};

/// The user kept the local planner (also returned when the question could
/// not be asked).
pub const CLOUD_DECLINED: &str =
    "Pegoles Local still plans your tasks: switching to Anthropic wasn't confirmed in the macOS dialog.";

/// The key window was cancelled.
pub const KEY_CANCELLED: &str = "the macOS key window was cancelled, so nothing was stored.";

/// The questions Pegoles asks outside the webview. A trait so tests can
/// answer them; `Err` means the question could not be asked at all.
pub trait Consent: Send + Sync {
    /// Ask before tasks start going to `provider`. `Ok(true)` only when
    /// the user chose to continue.
    fn allow_cloud_planner(&self, provider: Provider) -> Result<bool, String>;
    /// Ask for an API key in a native secure field. `Ok(None)` when the
    /// user cancelled.
    fn api_key(&self, replacing: bool) -> Result<Option<String>, String>;
    /// Ask before one task's computer gets internet (docs/EGRESS.md):
    /// the domains it may reach, or the open-web warning. Cancel is the
    /// default. `Ok(true)` only for a deliberate yes; per task, never
    /// remembered.
    fn allow_internet(&self, access: &InternetAccess) -> Result<bool, String>;
}

/// The wording of the internet confirmation, shared by every native
/// dialog. `None` for `off`: nothing to ask.
#[derive(Debug, PartialEq, Eq)]
pub struct InternetPrompt {
    pub title: String,
    pub body: String,
    pub confirm: &'static str,
}

pub fn internet_prompt(access: &InternetAccess) -> Option<InternetPrompt> {
    const ENDS: &str =
        "Internet access ends when the task stops, pauses or fails, or when you take control.";
    match access.mode {
        InternetMode::Off => None,
        InternetMode::Allowlist => {
            let sites: String = access
                .domains
                .iter()
                .map(|d| format!("\u{2022} {d}\n"))
                .collect();
            Some(InternetPrompt {
                title: "Let this task use the internet?".to_string(),
                body: format!(
                    "Pegoles’ computer will be able to open only these sites (and their \
                     subdomains) during this task:\n\n{sites}\nEverything else stays blocked. \
                     Its virtual machine still has no network of its own: Pegoles fetches \
                     these pages for it and checks every request and download.\n\n{ENDS}"
                ),
                confirm: "Allow these sites",
            })
        }
        InternetMode::OpenWeb => Some(InternetPrompt {
            title: "Give this task open internet access?".to_string(),
            body: format!(
                "Warning: Pegoles’ computer will be able to open almost any public website \
                 during this task. Known malware and phishing sites, adult and gambling \
                 sites, and downloads of programs or archives are blocked, but a page can \
                 still be unsafe or hold untrusted content, and anything the agent types \
                 into a page can leave the computer. Don’t give it passwords or secrets.\n\n{ENDS}"
            ),
            confirm: "Allow open web",
        }),
    }
}

/// A prompt the page kept reopening after the person dismissed it.
pub const PROMPT_PAUSED: &str =
    "that macOS window was dismissed several times, so Pegoles won't show it again for a while (or until it restarts).";

/// Declines of one native prompt. Each keeps it closed longer (30 s, then
/// 2 min, then 10 min) and after four it stays closed until Pegoles
/// restarts: a compromised page cannot wear the person down by reopening
/// the question. Accepting resets it.
#[derive(Debug, Default)]
pub struct DeclineBackoff {
    declines: usize,
    last: Option<std::time::Instant>,
}

impl DeclineBackoff {
    const WAITS: [std::time::Duration; 3] = [
        std::time::Duration::from_secs(30),
        std::time::Duration::from_secs(120),
        std::time::Duration::from_secs(600),
    ];

    /// Whether the prompt must not be shown at `now`.
    pub fn closed(&self, now: std::time::Instant) -> bool {
        match self.declines {
            0 => false,
            n if n > Self::WAITS.len() => true,
            n => self
                .last
                .is_some_and(|t| now.saturating_duration_since(t) < Self::WAITS[n - 1]),
        }
    }

    pub fn declined(&mut self, now: std::time::Instant) {
        self.declines += 1;
        self.last = Some(now);
    }

    pub fn accepted(&mut self) {
        *self = Self::default();
    }
}

/// Managed state: the app's `Consent` (native alerts in the app).
#[derive(Clone)]
pub struct ConsentGate(pub Arc<dyn Consent>);

/// Save a planner choice. Moving from Pegoles Local to a cloud provider
/// needs the user's native confirmation first; nothing is saved without it.
/// Settings are re-read after the (possibly long) prompt, so a change made
/// meanwhile is not overwritten with a stale copy.
pub fn change_provider(
    consent: &dyn Consent,
    provider: Provider,
    local_model: Option<String>,
    load: impl Fn() -> IntelligenceSettings,
    save: impl FnOnce(&IntelligenceSettings) -> Result<(), String>,
) -> Result<(), String> {
    let to_cloud = provider != Provider::Local && load().provider != provider;
    if to_cloud && !consent.allow_cloud_planner(provider)? {
        return Err(CLOUD_DECLINED.to_string());
    }
    let mut settings = load();
    settings.provider = provider;
    if let Some(m) = local_model {
        settings.local_model = m;
    }
    save(&settings)
}

/// Ask for the key natively and hand it to `store` (which checks its
/// shape). Cancelled: nothing is stored.
pub fn enter_api_key(
    consent: &dyn Consent,
    replacing: bool,
    store: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    match consent.api_key(replacing)? {
        Some(key) => store(&key),
        None => Err(KEY_CANCELLED.to_string()),
    }
}

#[cfg(target_os = "macos")]
pub use native::NativeConsent;

#[cfg(target_os = "macos")]
mod native {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use std::time::Instant;

    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSSecureTextField};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    use super::{internet_prompt, Consent, DeclineBackoff, PROMPT_PAUSED};
    use crate::agent::Provider;
    use pegoles_protocol::InternetAccess;

    /// One alert at a time: a page can't stack prompts behind the one the
    /// user is reading.
    pub struct NativeConsent {
        app: tauri::AppHandle,
        open: AtomicBool,
        cloud_backoff: Mutex<DeclineBackoff>,
        key_backoff: Mutex<DeclineBackoff>,
        internet_backoff: Mutex<DeclineBackoff>,
    }

    struct Reopen<'a>(&'a AtomicBool);

    impl Drop for Reopen<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    impl NativeConsent {
        pub fn new(app: tauri::AppHandle) -> Self {
            Self {
                app,
                open: AtomicBool::new(false),
                cloud_backoff: Mutex::new(DeclineBackoff::default()),
                key_backoff: Mutex::new(DeclineBackoff::default()),
                internet_backoff: Mutex::new(DeclineBackoff::default()),
            }
        }

        /// Run `show` on the main thread (AppKit) and wait for the answer.
        /// Called from a blocking worker, never the main thread.
        fn ask<T: Send + 'static>(
            &self,
            show: impl FnOnce(MainThreadMarker) -> T + Send + 'static,
        ) -> Result<T, String> {
            if self.open.swap(true, Ordering::SeqCst) {
                return Err(
                    "another Pegoles confirmation is already open; answer it first.".into(),
                );
            }
            let _reopen = Reopen(&self.open);
            let (tx, rx) = std::sync::mpsc::channel();
            self.app
                .run_on_main_thread(move || {
                    let _ = tx.send(MainThreadMarker::new().map(show));
                })
                .map_err(|e| format!("could not show the confirmation: {e}"))?;
            rx.recv()
                .map_err(|_| "the confirmation closed before it was answered.".to_string())?
                .ok_or_else(|| "the confirmation could not be shown.".to_string())
        }
    }

    fn alert(mtm: MainThreadMarker, title: &str, text: &str, confirm: &str) -> Retained<NSAlert> {
        let alert = NSAlert::new(mtm);
        alert.setAlertStyle(NSAlertStyle::Warning);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(text));
        // First button: Return. A button titled "Cancel": Escape.
        alert.addButtonWithTitle(&NSString::from_str(confirm));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));
        alert
    }

    impl Consent for NativeConsent {
        fn allow_cloud_planner(&self, provider: Provider) -> Result<bool, String> {
            let name = match provider {
                Provider::Anthropic => "Anthropic",
                Provider::Local => return Ok(true),
            };
            if lock(&self.cloud_backoff).closed(Instant::now()) {
                return Err(PROMPT_PAUSED.to_string());
            }
            let allowed = self.ask(move |mtm| {
                let alert = alert(
                    mtm,
                    &format!("Use {name} to plan your tasks?"),
                    &format!(
                        "When a task runs, its text and screenshots of Pegoles’ computer (its own \
                         virtual machine, never your Mac’s screen) will be sent to {name}’s API. \
                         Pegoles Local keeps everything on this Mac.\n\nOnly continue if you \
                         chose {name} in Settings just now."
                    ),
                    &format!("Use {name}"),
                );
                // Return and Escape both keep Pegoles Local: sending tasks to
                // the cloud takes a deliberate click, never a keystroke a
                // page could have prompted.
                let buttons = alert.buttons();
                if let (Some(confirm), Some(cancel)) = (buttons.firstObject(), buttons.lastObject())
                {
                    confirm.setKeyEquivalent(&NSString::from_str(""));
                    cancel.setKeyEquivalent(&NSString::from_str("\r"));
                }
                alert.runModal() == NSAlertFirstButtonReturn
            })?;
            let mut backoff = lock(&self.cloud_backoff);
            if allowed {
                backoff.accepted();
            } else {
                backoff.declined(Instant::now());
            }
            Ok(allowed)
        }

        fn allow_internet(&self, access: &InternetAccess) -> Result<bool, String> {
            let Some(prompt) = internet_prompt(access) else {
                return Ok(true);
            };
            if lock(&self.internet_backoff).closed(Instant::now()) {
                return Err(PROMPT_PAUSED.to_string());
            }
            let allowed = self.ask(move |mtm| {
                let alert = alert(mtm, &prompt.title, &prompt.body, prompt.confirm);
                // Return and Escape both keep the task offline.
                let buttons = alert.buttons();
                if let (Some(confirm), Some(cancel)) = (buttons.firstObject(), buttons.lastObject())
                {
                    confirm.setKeyEquivalent(&NSString::from_str(""));
                    cancel.setKeyEquivalent(&NSString::from_str("\r"));
                }
                alert.runModal() == NSAlertFirstButtonReturn
            })?;
            let mut backoff = lock(&self.internet_backoff);
            if allowed {
                backoff.accepted();
            } else {
                backoff.declined(Instant::now());
            }
            Ok(allowed)
        }

        fn api_key(&self, replacing: bool) -> Result<Option<String>, String> {
            if lock(&self.key_backoff).closed(Instant::now()) {
                return Err(PROMPT_PAUSED.to_string());
            }
            let key = self.ask(move |mtm| {
                let alert = alert(
                    mtm,
                    if replacing {
                        "Replace your Anthropic API key"
                    } else {
                        "Add your Anthropic API key"
                    },
                    "Paste the key here. Pegoles keeps it in your Mac’s Keychain and never shows \
                     it again; the app window never sees it.",
                    "Save",
                );
                let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(320.0, 24.0));
                let field = NSSecureTextField::initWithFrame(NSSecureTextField::alloc(mtm), frame);
                field.setPlaceholderString(Some(&NSString::from_str("sk-ant-…")));
                alert.setAccessoryView(Some(&field));
                alert.window().setInitialFirstResponder(Some(&field));
                let saved = alert.runModal() == NSAlertFirstButtonReturn;
                let key = saved.then(|| field.stringValue().to_string());
                // Nothing lingers in the view once it closes.
                field.setStringValue(&NSString::from_str(""));
                key
            })?;
            let mut backoff = lock(&self.key_backoff);
            match key {
                Some(_) => backoff.accepted(),
                None => backoff.declined(Instant::now()),
            }
            Ok(key)
        }
    }

    fn lock(m: &Mutex<DeclineBackoff>) -> std::sync::MutexGuard<'_, DeclineBackoff> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Off macOS the cloud-planner and key questions have no native window yet
/// (fail closed). The internet question has one on Windows (a system
/// message box the webview cannot answer) and fails closed elsewhere.
#[cfg(not(target_os = "macos"))]
pub struct NativeConsent {
    open: std::sync::atomic::AtomicBool,
    internet_backoff: std::sync::Mutex<DeclineBackoff>,
}

#[cfg(not(target_os = "macos"))]
impl NativeConsent {
    pub fn new(_app: tauri::AppHandle) -> Self {
        Self {
            open: std::sync::atomic::AtomicBool::new(false),
            internet_backoff: std::sync::Mutex::new(DeclineBackoff::default()),
        }
    }
}

/// The Windows question: a system message box (OK / Cancel, Cancel is the
/// default button and Escape). No owner window: nothing in the webview
/// can reach, answer or dismiss it.
#[cfg(windows)]
fn ask_ok_cancel(title: &str, text: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDOK, MB_DEFBUTTON2, MB_ICONWARNING, MB_OKCANCEL, MB_SETFOREGROUND, MB_TOPMOST,
    };
    let wide = |s: &str| {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let (title, text) = (wide(title), wide(text));
    // SAFETY: both buffers are NUL-terminated UTF-16 that outlive the call.
    let answer = unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OKCANCEL | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST,
        )
    };
    answer == IDOK
}

#[cfg(not(target_os = "macos"))]
impl Consent for NativeConsent {
    fn allow_internet(&self, access: &InternetAccess) -> Result<bool, String> {
        use std::sync::atomic::Ordering;
        let Some(prompt) = internet_prompt(access) else {
            return Ok(true);
        };
        if !cfg!(windows) {
            return Err("internet access needs a native confirmation window, which exists only on macOS and Windows so far.".into());
        }
        let lock = || {
            self.internet_backoff
                .lock()
                .unwrap_or_else(|e| e.into_inner())
        };
        if lock().closed(std::time::Instant::now()) {
            return Err(PROMPT_PAUSED.to_string());
        }
        if self.open.swap(true, Ordering::SeqCst) {
            return Err("another Pegoles confirmation is already open; answer it first.".into());
        }
        #[cfg(windows)]
        let allowed = {
            let text = format!(
                "{}\n\nChoose OK to {}, or Cancel to keep this task offline.",
                prompt.body,
                prompt.confirm.to_lowercase()
            );
            ask_ok_cancel(&prompt.title, &text)
        };
        #[cfg(not(windows))]
        let allowed = false;
        self.open.store(false, Ordering::SeqCst);
        let mut backoff = lock();
        if allowed {
            backoff.accepted();
        } else {
            backoff.declined(std::time::Instant::now());
        }
        Ok(allowed)
    }

    fn allow_cloud_planner(&self, _provider: Provider) -> Result<bool, String> {
        Err("cloud models aren't available on this system yet: they need a native confirmation window, which exists only on macOS so far. Pegoles Local keeps planning.".into())
    }
    fn api_key(&self, _replacing: bool) -> Result<Option<String>, String> {
        Err("adding a key needs a native key window, which exists only on macOS so far.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::validate_api_key;
    use std::cell::RefCell;
    use std::sync::Mutex;

    /// Answers every question the same way and counts them.
    struct Scripted {
        cloud: Result<bool, String>,
        key: Result<Option<String>, String>,
        asked: Mutex<Vec<&'static str>>,
    }

    impl Scripted {
        fn new(cloud: Result<bool, String>, key: Result<Option<String>, String>) -> Self {
            Self {
                cloud,
                key,
                asked: Mutex::new(Vec::new()),
            }
        }
        fn asked(&self) -> Vec<&'static str> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Consent for Scripted {
        fn allow_cloud_planner(&self, _provider: Provider) -> Result<bool, String> {
            self.asked.lock().unwrap().push("cloud");
            self.cloud.clone()
        }
        fn api_key(&self, _replacing: bool) -> Result<Option<String>, String> {
            self.asked.lock().unwrap().push("key");
            self.key.clone()
        }
        fn allow_internet(&self, _access: &InternetAccess) -> Result<bool, String> {
            self.asked.lock().unwrap().push("internet");
            self.cloud.clone()
        }
    }

    fn local() -> IntelligenceSettings {
        IntelligenceSettings::default()
    }

    fn cloud() -> IntelligenceSettings {
        IntelligenceSettings {
            provider: Provider::Anthropic,
            ..IntelligenceSettings::default()
        }
    }

    /// Runs `change_provider` from `current` and returns what was saved.
    fn switch(
        consent: &Scripted,
        current: IntelligenceSettings,
        to: Provider,
    ) -> (Result<(), String>, Option<IntelligenceSettings>) {
        let saved = RefCell::new(None);
        let result = change_provider(
            consent,
            to,
            None,
            || current.clone(),
            |s| {
                *saved.borrow_mut() = Some(s.clone());
                Ok(())
            },
        );
        (result, saved.into_inner())
    }

    #[test]
    fn moving_to_the_cloud_is_saved_only_after_native_confirmation() {
        let yes = Scripted::new(Ok(true), Ok(None));
        let (result, saved) = switch(&yes, local(), Provider::Anthropic);
        assert_eq!(result, Ok(()));
        assert_eq!(saved.map(|s| s.provider), Some(Provider::Anthropic));
        assert_eq!(yes.asked(), ["cloud"]);
    }

    #[test]
    fn a_declined_or_unanswerable_confirmation_keeps_the_local_planner() {
        let no = Scripted::new(Ok(false), Ok(None));
        let (result, saved) = switch(&no, local(), Provider::Anthropic);
        assert_eq!(result, Err(CLOUD_DECLINED.to_string()));
        assert!(saved.is_none(), "nothing is saved without consent");

        let busy = Scripted::new(Err("another one is open".into()), Ok(None));
        let (result, saved) = switch(&busy, local(), Provider::Anthropic);
        assert_eq!(result, Err("another one is open".to_string()));
        assert!(saved.is_none());
    }

    #[test]
    fn staying_on_or_returning_to_a_planner_asks_nothing() {
        let never = Scripted::new(Ok(false), Ok(None));
        let (result, saved) = switch(&never, cloud(), Provider::Local);
        assert_eq!(result, Ok(()));
        assert_eq!(saved.map(|s| s.provider), Some(Provider::Local));
        let (result, _) = switch(&never, cloud(), Provider::Anthropic);
        assert_eq!(result, Ok(()), "already on Anthropic: not a new crossing");
        let (result, _) = switch(&never, local(), Provider::Local);
        assert_eq!(result, Ok(()));
        assert!(never.asked().is_empty());
    }

    #[test]
    fn the_local_model_choice_is_saved_with_the_provider() {
        let saved = RefCell::new(None);
        let never = Scripted::new(Ok(false), Ok(None));
        let result = change_provider(
            &never,
            Provider::Local,
            Some("other-model".into()),
            local,
            |s| {
                *saved.borrow_mut() = Some(s.clone());
                Ok(())
            },
        );
        assert_eq!(result, Ok(()));
        assert_eq!(saved.into_inner().unwrap().local_model, "other-model");
    }

    #[test]
    fn a_key_comes_only_from_the_native_field_and_is_shape_checked() {
        let key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        let typed = Scripted::new(Ok(false), Ok(Some(format!("  {key}\n"))));
        let stored = RefCell::new(None);
        let result = enter_api_key(&typed, false, |raw| {
            *stored.borrow_mut() = Some(validate_api_key(raw)?);
            Ok(())
        });
        assert_eq!(result, Ok(()));
        assert_eq!(stored.into_inner().as_deref(), Some(key));

        let short = Scripted::new(Ok(false), Ok(Some("sk-short".into())));
        let result = enter_api_key(&short, false, |raw| validate_api_key(raw).map(|_| ()));
        assert_eq!(
            result,
            Err("the key should be 20 to 256 characters".to_string())
        );
    }

    #[test]
    fn a_cancelled_key_window_stores_nothing() {
        let cancelled = Scripted::new(Ok(false), Ok(None));
        let result = enter_api_key(&cancelled, true, |_| panic!("nothing to store"));
        assert_eq!(result, Err(KEY_CANCELLED.to_string()));

        let busy = Scripted::new(Ok(false), Err("another one is open".into()));
        let result = enter_api_key(&busy, true, |_| panic!("nothing to store"));
        assert_eq!(result, Err("another one is open".to_string()));
    }

    #[test]
    fn the_internet_question_lists_the_domains_or_warns_and_off_asks_nothing() {
        let sites = InternetAccess {
            mode: InternetMode::Allowlist,
            domains: vec!["example.com".into(), "docs.example.org".into()],
        };
        let p = internet_prompt(&sites).unwrap();
        assert!(p.body.contains("\u{2022} example.com\n"));
        assert!(p.body.contains("\u{2022} docs.example.org\n"));
        assert!(p.body.contains("ends when the task stops"));
        let open = internet_prompt(&InternetAccess {
            mode: InternetMode::OpenWeb,
            domains: vec![],
        })
        .unwrap();
        assert!(open.body.starts_with("Warning:"));
        assert!(open.body.contains("untrusted") && open.body.contains("secrets"));
        assert_ne!(p.confirm, open.confirm);
        assert!(internet_prompt(&InternetAccess::off()).is_none());
    }

    #[test]
    fn declines_close_a_prompt_for_longer_each_time_then_for_good() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let mut b = DeclineBackoff::default();
        assert!(!b.closed(t0));
        b.declined(t0);
        assert!(b.closed(t0 + Duration::from_secs(29)));
        assert!(!b.closed(t0 + Duration::from_secs(31)));
        b.declined(t0);
        assert!(b.closed(t0 + Duration::from_secs(119)));
        assert!(!b.closed(t0 + Duration::from_secs(121)));
        b.declined(t0);
        assert!(b.closed(t0 + Duration::from_secs(599)));
        assert!(!b.closed(t0 + Duration::from_secs(601)));
        b.declined(t0);
        assert!(
            b.closed(t0 + Duration::from_secs(86_400)),
            "closed until restart"
        );
        b.accepted();
        assert!(!b.closed(t0));
    }
}
