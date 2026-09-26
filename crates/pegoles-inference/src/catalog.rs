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
            let allowed = [
                ".safetensors",
                ".json",
                ".txt",
                ".jinja",
                ".model",
                ".tiktoken",
            ];
            if !allowed.iter().any(|ext| lower.ends_with(ext)) {
                // No pickles (.bin/.pt/.pth) and no Python: formats that
                // execute code on load never enter the store.
                return Err(format!("{}: file type not allowed: {}", self.id, f.path));
            }
        }
        if !seen.contains("config.json") {
            return Err(format!("{}: no config.json", self.id));
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CatalogFile {
    schema: u32,
    default_model: String,
    /// Models the app offers to install. Others stay in the catalog so
    /// benchmark runs remain reproducible (e.g. 4-bit variants that
    /// failed the Pegoles benchmark).
    offered: Vec<String>,
    models: Vec<ModelSpec>,
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

    pub fn parse(raw: &str) -> Result<Catalog, String> {
        let file: CatalogFile = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        if file.schema != CATALOG_SCHEMA {
            return Err(format!("unsupported catalog schema {}", file.schema));
        }
        let mut ids = std::collections::HashSet::new();
        for m in &file.models {
            m.validate()?;
            if !ids.insert(m.id.clone()) {
                return Err(format!("duplicate model id {}", m.id));
            }
        }
        if !ids.contains(&file.default_model) {
            return Err("default model is not in the catalog".into());
        }
        if !file.offered.contains(&file.default_model)
            || file.offered.iter().any(|id| !ids.contains(id))
        {
            return Err("offered models must exist and include the default".into());
        }
        Ok(Catalog {
            default_model: file.default_model,
            offered: file.offered,
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
