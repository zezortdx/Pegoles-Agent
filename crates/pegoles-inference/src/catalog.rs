//! The models Pegoles knows how to run, pinned to exact bytes.
//!
//! Each entry names its source (repository + immutable commit), license,
//! quantization, and every file with its size and SHA-256. A model is
//! "installed" only when every listed file is present with exactly those
//! bytes; nothing is trusted because a file name matches.
//!
//! The catalog is compiled into the binary (`catalog/models.json`), so a
//! tampered download source cannot change what counts as valid.

use serde::{Deserialize, Serialize};

pub const CATALOG_SCHEMA: u32 = 1;
const BUILTIN: &str = include_str!("../catalog/models.json");

/// How Pegoles prompts a model and reads its answers. Two families can
/// share an architecture (MAI-UI is a Qwen3-VL fine-tune) and still need
/// different prompts and parsers.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    MaiUi,
    Qwen3Vl,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelSource {
    /// `https://huggingface.co/{repo}/resolve/{revision}/{path}`.
    Huggingface { repo: String, revision: String },
    /// Produced on this machine (e.g. `mlx_vlm.convert` of a pinned
    /// upstream). Not downloadable; importable only with matching bytes.
    LocalConversion {
        upstream_repo: String,
        upstream_revision: String,
        recipe: String,
    },
}

/// How a model's weights are stored, i.e. which local runtime runs it.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ModelFormat {
    /// MLX safetensors (the Python MLX worker, Apple silicon).
    #[default]
    Mlx,
    /// GGUF text model + multimodal projector (the llama.cpp worker).
    Gguf,
}

/// The two files of a GGUF vision model.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GgufFiles {
    pub model: String,
    pub projector: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelSpec {
    pub id: String,
    pub display_name: String,
    pub family: ModelFamily,
    /// Model architecture as the runtime sees it (config `model_type`).
    pub architecture: String,
    pub parameters: String,
    pub quantization: String,
    pub source: ModelSource,
    pub license: String,
    /// Oldest Pegoles version whose prompts/parsers support this entry.
    pub pegoles_min_version: String,
    /// Measured recommendation (model + VM + app); `None` until measured.
    pub recommended_min_ram_gb: Option<u32>,
    pub published: String,
    #[serde(default)]
    pub format: ModelFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gguf: Option<GgufFiles>,
    pub files: Vec<ModelFile>,
}

impl ModelSpec {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn downloadable(&self) -> bool {
        matches!(self.source, ModelSource::Huggingface { .. })
    }

    /// Structural validation: ids and paths that could escape the store,
    /// digests that are not SHA-256, formats that can execute code.
    pub fn validate(&self) -> Result<(), String> {
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 64
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
            && !self.id.starts_with('.');
        if !id_ok {
            return Err(format!("invalid model id {:?}", self.id));
        }
        if self.files.is_empty() || self.files.len() > 64 {
            return Err(format!("{}: bad file list", self.id));
        }
        let mut seen = std::collections::HashSet::new();
        for f in &self.files {
            validate_rel_path(&f.path).map_err(|e| format!("{}: {e}", self.id))?;
            if !seen.insert(f.path.as_str()) {
                return Err(format!("{}: duplicate file {}", self.id, f.path));
            }
            if f.sha256.len() != 64 || !f.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("{}: {} has no SHA-256", self.id, f.path));
            }
            let lower = f.path.to_ascii_lowercase();
            let allowed: &[&str] = match self.format {
                ModelFormat::Mlx => &[
                    ".safetensors",
                    ".json",
                    ".txt",
                    ".jinja",
                    ".model",
                    ".tiktoken",
                ],
                ModelFormat::Gguf => &[".gguf"],
            };
            if !allowed.iter().any(|ext| lower.ends_with(ext)) {
                // No pickles (.bin/.pt/.pth) and no Python: formats that
                // execute code on load never enter the store.
                return Err(format!("{}: file type not allowed: {}", self.id, f.path));
            }
        }
        match (self.format, &self.gguf) {
            (ModelFormat::Mlx, None) => {
                if !seen.contains("config.json") {
                    return Err(format!("{}: no config.json", self.id));
                }
            }
            (ModelFormat::Gguf, Some(g)) => {
                for name in [&g.model, &g.projector] {
                    if name.contains('/') || !seen.contains(name.as_str()) {
                        return Err(format!(
                            "{}: GGUF file {name} is not a listed top-level file",
                            self.id
                        ));
                    }
                }
                if g.model == g.projector {
                    return Err(format!("{}: model and projector must differ", self.id));
                }
            }
            _ => return Err(format!("{}: format and GGUF file names disagree", self.id)),
        }
        if let ModelSource::Huggingface { repo, revision } = &self.source {
            let repo_ok = repo.split('/').count() == 2
                && repo
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b))
                && !repo.contains("..");
            if !repo_ok {
                return Err(format!("{}: bad repository {repo:?}", self.id));
            }
            if revision.len() != 40 || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("{}: revision must be a full commit hash", self.id));
            }
        }
        Ok(())
    }
}

/// Relative, forward-slash, no `..`, no hidden components, bounded.
pub fn validate_rel_path(path: &str) -> Result<(), String> {
    let ok = !path.is_empty()
        && path.len() <= 200
        && !path.starts_with('/')
        && !path.contains('\\')
        && path.split('/').all(|c| {
            !c.is_empty()
                && !c.starts_with('.')
                && c.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        });
    if ok {
        Ok(())
    } else {
        Err(format!("unsafe file path {path:?}"))
    }
}

/// What one host platform offers: its runtime decides the format.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct HostModels {
    default_model: String,
    /// Models the app offers to install. Others stay in the catalog so
    /// benchmark runs remain reproducible (e.g. 4-bit variants that
    /// failed the Pegoles benchmark).
    offered: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CatalogFile {
    schema: u32,
    /// `macos` (MLX) and `windows` (GGUF / llama.cpp).
    hosts: std::collections::BTreeMap<String, HostModels>,
    models: Vec<ModelSpec>,
}

/// The catalog section for the host this binary runs on.
pub fn host_key() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else {
        "macos"
    }
}

#[derive(Clone, Debug)]
pub struct Catalog {
    pub default_model: String,
    pub offered: Vec<String>,
    pub models: Vec<ModelSpec>,
}

impl Catalog {
    pub fn builtin() -> Catalog {
        Self::parse(BUILTIN).expect("built-in model catalog is valid")
    }

    /// The catalog as this host sees it (its default and offered models).
    pub fn parse(raw: &str) -> Result<Catalog, String> {
        Self::parse_for(raw, host_key())
    }

    pub fn parse_for(raw: &str, host: &str) -> Result<Catalog, String> {
        let file: CatalogFile = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        if file.schema != CATALOG_SCHEMA {
            return Err(format!("unsupported catalog schema {}", file.schema));
        }
        let mut ids = std::collections::HashMap::new();
        for m in &file.models {
            m.validate()?;
            if ids.insert(m.id.clone(), m.format).is_some() {
                return Err(format!("duplicate model id {}", m.id));
            }
        }
        // Every host section must be coherent, not only ours.
        for (name, section) in &file.hosts {
            let format = if name == "windows" {
                ModelFormat::Gguf
            } else {
                ModelFormat::Mlx
            };
            if !section.offered.contains(&section.default_model)
                || section
                    .offered
                    .iter()
                    .any(|id| ids.get(id) != Some(&format))
            {
                return Err(format!(
                    "{name}: offered models must exist, run on its runtime and include the default"
                ));
            }
        }
        let section = file
            .hosts
            .get(host)
            .ok_or_else(|| format!("the catalog has no models for {host}"))?;
        Ok(Catalog {
            default_model: section.default_model.clone(),
            offered: section.offered.clone(),
            models: file.models,
        })
    }

    pub fn get(&self, id: &str) -> Option<&ModelSpec> {
        self.models.iter().find(|m| m.id == id)
    }

    pub fn is_offered(&self, id: &str) -> bool {
        self.offered.iter().any(|o| o == id)
    }

    pub fn default_spec(&self) -> &ModelSpec {
        self.get(&self.default_model).expect("validated default")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_is_valid() {
        let c = Catalog::builtin();
        assert!(!c.models.is_empty());
        assert!(c.get(&c.default_model).is_some());
    }

    #[test]
    fn each_host_gets_models_its_runtime_can_run() {
        let mac = Catalog::parse_for(BUILTIN, "macos").unwrap();
        assert_eq!(mac.default_model, "mai-ui-2b-6bit");
        assert!(mac
            .offered
            .iter()
            .all(|id| mac.get(id).unwrap().format == ModelFormat::Mlx));
        let win = Catalog::parse_for(BUILTIN, "windows").unwrap();
        let default = win.get(&win.default_model).unwrap();
        assert_eq!(default.format, ModelFormat::Gguf);
        assert_eq!(default.family, ModelFamily::MaiUi);
        let g = default.gguf.as_ref().unwrap();
        assert!(g.model.ends_with(".gguf") && g.projector.contains("mmproj"));
        assert!(Catalog::parse_for(BUILTIN, "linux").is_err());
    }

    #[test]
    fn gguf_entries_carry_only_gguf_files() {
        let win = Catalog::parse_for(BUILTIN, "windows").unwrap();
        let mut s = win.get(&win.default_model).unwrap().clone();
        assert!(s.validate().is_ok());
        s.files.push(file("tokenizer.json"));
        assert!(s.validate().is_err());
        let mut s = win.get(&win.default_model).unwrap().clone();
        s.gguf = None;
        assert!(s.validate().is_err());
    }

    #[test]
    fn paths_that_escape_or_hide_are_rejected() {
        for bad in [
            "../x.json",
            "/etc/passwd",
            "a/../../b.json",
            ".hidden.json",
            "a\\b.json",
            "",
            "a//b.json",
        ] {
            assert!(validate_rel_path(bad).is_err(), "{bad}");
        }
        assert!(validate_rel_path("model-00001-of-00002.safetensors").is_ok());
        assert!(validate_rel_path("sub/tokenizer.json").is_ok());
    }

    fn spec_with(files: Vec<ModelFile>) -> ModelSpec {
        let mut s = Catalog::builtin().models[0].clone();
        s.files = files;
        s
    }

    fn file(path: &str) -> ModelFile {
        ModelFile {
            path: path.into(),
            size: 1,
            sha256: "a".repeat(64),
        }
    }

    #[test]
    fn executable_formats_and_missing_digests_are_rejected() {
        assert!(
            spec_with(vec![file("config.json"), file("pytorch_model.bin")])
                .validate()
                .is_err()
        );
        assert!(
            spec_with(vec![file("config.json"), file("modeling_evil.py")])
                .validate()
                .is_err()
        );
        let mut no_digest = file("model.safetensors");
        no_digest.sha256 = "abc".into();
        assert!(spec_with(vec![file("config.json"), no_digest])
            .validate()
            .is_err());
        assert!(spec_with(vec![file("model.safetensors")])
            .validate()
            .is_err());
        assert!(
            spec_with(vec![file("config.json"), file("model.safetensors")])
                .validate()
                .is_ok()
        );
    }
}
