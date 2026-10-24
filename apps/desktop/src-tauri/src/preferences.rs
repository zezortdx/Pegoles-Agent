//! Validated, content-minimized local appearance preferences. No planner authority.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvatarProfile {
    pub shape: String,
    pub color: String,
    pub expression: String,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub schema_version: u8,
    pub avatar: AvatarProfile,
    pub locale: String,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            avatar: AvatarProfile {
                shape: "circle".into(),
                color: "silver".into(),
                expression: "attentive".into(),
                name: "Pegoles".into(),
            },
            locale: "en".into(),
        }
    }
}
pub struct PreferenceStore {
    directory: PathBuf,
}
impl PreferenceStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn read(&self) -> Result<Preferences, String> {
        use std::io::Read;
        check_directories(&self.directory)?;
        let path = self.directory.join("profile.json");
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
                return Err("Invalid preferences file.".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Preferences::default()),
            Err(_) => return Err("Could not inspect preferences.".into()),
        }
        let file = match private_options().read(true).open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Preferences::default()),
            Err(_) => return Err("Could not read preferences.".into()),
        };
        let metadata = file
            .metadata()
            .map_err(|_| "Could not inspect preferences.")?;
        if !metadata.is_file() || metadata.len() > 4096 {
            return Err("Invalid preferences file.".into());
        }
        let mut bytes = Vec::new();
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read preferences.")?;
        if bytes.len() > 4096 {
            return Err("Preferences file is too large.".into());
        }
        let parsed: Preferences =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid preferences file.")?;
        parsed.validate()
    }

    pub fn write(&self, preferences: Preferences) -> Result<Preferences, String> {
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let preferences = preferences.validate()?;
        let bytes =
            serde_json::to_vec(&preferences).map_err(|_| "Could not encode preferences.")?;
        if bytes.len() > 4096 {
            return Err("Preferences file is too large.".into());
        }
        check_directories(&self.directory)?;
        std::fs::create_dir_all(&self.directory)
            .map_err(|_| "Could not create preferences directory.")?;
        check_directories(&self.directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "Could not secure preferences directory.")?;
        }
        let path = self.directory.join("profile.json");
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
                return Err("Invalid preferences file.".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not inspect preferences.".into()),
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Clock unavailable.")?
            .as_nanos();
        let temporary = self.directory.join(format!(
            ".profile-{}-{}-{}.tmp",
            std::process::id(),
            nonce,
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = private_options()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|_| "Could not create preferences file.")?;
            file.write_all(&bytes)
                .map_err(|_| "Could not write preferences.")?;
            file.sync_all().map_err(|_| "Could not sync preferences.")?;
            std::fs::rename(&temporary, &path).map_err(|_| "Could not replace preferences.")?;
            #[cfg(unix)]
            private_options()
                .read(true)
                .open(&self.directory)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| "Could not sync preferences directory.")?;
            Ok(preferences)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

impl Preferences {
    pub fn validate(mut self) -> Result<Self, String> {
        const SHAPES: &[&str] = &[
            "circle", "pebble", "squircle", "capsule", "triangle", "hexagon", "cloud", "droplet",
        ];
        const EXPRESSIONS: &[&str] = &[
            "neutral",
            "attentive",
            "surprised",
            "excited",
            "happy",
            "laughing",
            "angry",
            "sad",
            "afraid",
            "suspicious",
            "confused",
            "curious",
            "proud",
            "shy",
            "bored",
            "sleepy",
        ];
        if self.schema_version != 1
            || !SHAPES.contains(&self.avatar.shape.as_str())
            || !EXPRESSIONS.contains(&self.avatar.expression.as_str())
            || !["silver", "graphite", "pearl"].contains(&self.avatar.color.as_str())
            || !["en", "pt-BR"].contains(&self.locale.as_str())
        {
            return Err("Unsupported preferences.".into());
        }
        // Validate before trimming: controls and bidi cannot hide at the edges.
        if self.avatar.name.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
            return Err("Invalid agent name.".into());
        }
        self.avatar.name = self.avatar.name.trim().into();
        if !(1..=32).contains(&self.avatar.name.chars().count()) {
            return Err("Agent name must have 1 to 32 characters.".into());
        }
        Ok(self)
    }
}

/// Refuse symlinks at every existing directory component. Same-user host
/// mutation races are outside the threat model; leaf opens also use O_NOFOLLOW.
fn check_directories(path: &std::path::Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                return Err("Invalid preferences directory.".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not inspect preferences directory.".into()),
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
        // Native constants avoid adding a runtime dependency for two flags.
        #[cfg(target_os = "macos")]
        const FLAGS: i32 = 0x100 | 0x4; // O_NOFOLLOW | O_NONBLOCK
        #[cfg(not(target_os = "macos"))]
        const FLAGS: i32 = 0x20000 | 0x800; // Linux O_NOFOLLOW | O_NONBLOCK
        options.mode(0o600).custom_flags(FLAGS);
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture {
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            Self {
                path: std::env::temp_dir().canonicalize().unwrap().join(format!(
                    "pegoles-preferences-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                )),
            }
        }
        fn store(&self) -> PreferenceStore {
            PreferenceStore::new(self.path.join("preferences"))
        }
        fn file(&self) -> PathBuf {
            self.path.join("preferences/profile.json")
        }
        fn raw(&self, bytes: &[u8]) {
            fs::create_dir_all(self.file().parent().unwrap()).unwrap();
            fs::write(self.file(), bytes).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
    fn customized() -> Preferences {
        Preferences {
            avatar: AvatarProfile {
                shape: "cloud".into(),
                color: "graphite".into(),
                expression: "curious".into(),
                name: "  Agent 🪩  ".into(),
            },
            locale: "pt-BR".into(),
            ..Preferences::default()
        }
    }
    #[test]
    fn absent_is_default_and_save_round_trips_after_reopen() {
        let f = Fixture::new();
        assert_eq!(f.store().read().unwrap(), Preferences::default());
        assert!(!f.file().exists());
        let mut expected = customized();
        expected.avatar.name = "Agent 🪩".into();
        assert_eq!(f.store().write(customized()).unwrap(), expected);
        assert_eq!(f.store().read().unwrap(), expected);
        assert_eq!(fs::read_dir(f.file().parent().unwrap()).unwrap().count(), 1);
    }
    #[test]
    fn invalid_enums_version_name_and_locale_never_replace_saved_preferences() {
        let f = Fixture::new();
        f.store().write(Preferences::default()).unwrap();
        for field in [
            "shape",
            "expression",
            "color",
            "locale",
            "schema_version",
            "name",
        ] {
            let mut value = serde_json::to_value(customized()).unwrap();
            match field {
                "locale" => value[field] = "es".into(),
                "schema_version" => value[field] = 2.into(),
                "name" => value["avatar"][field] = "\u{202e}Agent".into(),
                _ => value["avatar"][field] = "<svg>".into(),
            }
            assert!(
                f.store()
                    .write(serde_json::from_value(value).unwrap())
                    .is_err(),
                "{field}"
            );
            assert_eq!(f.store().read().unwrap(), Preferences::default());
        }
        for name in [
            "".into(),
            "   ".into(),
            "a".repeat(33),
            "Agent\nname".into(),
            "Agent\u{2066}".into(),
        ] {
            let mut p = customized();
            p.avatar.name = name;
            assert!(f.store().write(p).is_err());
        }
        let mut p = customized();
        p.avatar.name = "🪩".repeat(32);
        assert!(f.store().write(p).is_ok());
    }
    #[test]
    fn corruption_unknown_fields_oversize_and_disk_errors_are_reported() {
        let f = Fixture::new();
        for bytes in [b"not json".to_vec(), vec![b' '; 4097], br#"{"schema_version":1,"avatar":{"shape":"circle","color":"silver","expression":"attentive","name":"Pegoles","svg":"x"},"locale":"en"}"#.to_vec(), br#"{"schema_version":1,"avatar":{"shape":"circle","color":"silver","expression":"attentive","name":"Pegoles"},"locale":"en","path":"/tmp/x"}"#.to_vec()] {
            f.raw(&bytes); assert!(f.store().read().is_err());
        }
        fs::remove_file(f.file()).unwrap();
        fs::create_dir(f.file()).unwrap();
        assert!(f.store().read().is_err());
        assert!(f.store().write(customized()).is_err());
        let blocked = f.path.join("not-a-directory");
        fs::write(&blocked, "x").unwrap();
        assert!(PreferenceStore::new(blocked).write(customized()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_are_refused_and_file_permissions_are_private() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let f = Fixture::new();
        f.store().write(Preferences::default()).unwrap();
        assert_eq!(
            fs::metadata(f.file()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(f.file().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let outside = f.path.join("outside");
        fs::write(&outside, "untouched").unwrap();
        fs::remove_file(f.file()).unwrap();
        symlink(&outside, f.file()).unwrap();
        assert!(f.store().read().is_err());
        assert!(f.store().write(customized()).is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "untouched");
        fs::remove_file(f.file()).unwrap();
        fs::remove_dir(f.file().parent().unwrap()).unwrap();
        symlink(&f.path, f.file().parent().unwrap()).unwrap();
        assert!(f.store().read().is_err());
        assert!(f.store().write(customized()).is_err());
    }
}
