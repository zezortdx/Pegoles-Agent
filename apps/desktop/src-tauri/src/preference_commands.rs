//! Appearance IPC only. The webview never supplies a file path.
use crate::preferences::{PreferenceStore, Preferences};
use std::sync::Mutex;

static STORE_LOCK: Mutex<()> = Mutex::new(());

async fn with_store(write: Option<Preferences>) -> Result<Preferences, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = STORE_LOCK.lock().map_err(|_| "Preferences unavailable.")?;
        let store = PreferenceStore::new(pegoles_computer::pegoles_data_dir().join("preferences"));
        match write {
            Some(preferences) => store.write(preferences),
            None => store.read(),
        }
    })
    .await
    .map_err(|_| "Preferences unavailable.".to_string())?
}

pub async fn get_preferences() -> Result<Preferences, String> {
    with_store(None).await
}

pub async fn set_preferences(preferences: Preferences) -> Result<Preferences, String> {
    with_store(Some(preferences)).await
}
