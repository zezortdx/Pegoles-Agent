//! Native confirmation for the two settings that cross the trust boundary:
//! handing tasks to a cloud planner, and storing that planner's API key.
//!
//! Rust asks in a macOS alert that script in the webview can neither see
//! into nor answer, so a compromised page can't opt the user into the
//! cloud or plant its own key. The key itself is typed into the alert's
//! secure field: it never enters the webview's DOM or JavaScript.

use std::sync::Arc;

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
    use std::time::{Duration, Instant};

    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSSecureTextField};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    use super::Consent;
    use crate::agent::Provider;

    /// After the user declines the cloud planner, further requests are
    /// refused for this long without showing anything, so a page cannot
    /// re-open the question until a keystroke lands on it.
    const CLOUD_DECLINE_COOLDOWN: Duration = Duration::from_secs(30);

    /// One alert at a time: a page can't stack prompts behind the one the
    /// user is reading.
    pub struct NativeConsent {
        app: tauri::AppHandle,
        open: AtomicBool,
        cloud_declined_at: Mutex<Option<Instant>>,
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
                cloud_declined_at: Mutex::new(None),
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
            {
                let declined = self
                    .cloud_declined_at
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if declined.is_some_and(|t| t.elapsed() < CLOUD_DECLINE_COOLDOWN) {
                    return Ok(false);
                }
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
            if !allowed {
                *self
                    .cloud_declined_at
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
            }
            Ok(allowed)
        }

        fn api_key(&self, replacing: bool) -> Result<Option<String>, String> {
            self.ask(move |mtm| {
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
            })
        }
    }
}

/// Off macOS there is no native confirmation yet: fail closed.
#[cfg(not(target_os = "macos"))]
pub struct NativeConsent;

#[cfg(not(target_os = "macos"))]
impl NativeConsent {
    pub fn new(_app: tauri::AppHandle) -> Self {
        Self
    }
}

#[cfg(not(target_os = "macos"))]
impl Consent for NativeConsent {
    fn allow_cloud_planner(&self, _provider: Provider) -> Result<bool, String> {
        Err("cloud planners need the macOS confirmation dialog.".into())
    }
    fn api_key(&self, _replacing: bool) -> Result<Option<String>, String> {
        Err("adding a key needs the macOS key dialog; set ANTHROPIC_API_KEY instead.".into())
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
}
