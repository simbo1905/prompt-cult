//! Per-service JSONC configuration
//! ([`schemas/proxy-config.jtd.json`](../../../schemas/proxy-config.jtd.json)).
//!
//! Each service owns exactly one config file, `<service-id>.jsonc`, in the
//! proxy config directory. A missing file resolves to compiled-in defaults.
//! A present-but-malformed file (bad JSONC, unknown key, invalid glob,
//! mutually exclusive fields both set) is a **hard startup error** —
//! silently ignoring a broken config is how "the wrong model ran" bugs
//! happen. Storage is behind the [`ConfigStore`] trait: plain disk today;
//! enterprise secure storage or k8s ConfigMap mounts later, without
//! touching proxy logic.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use globset::Glob;
use globset::GlobSet;
use globset::GlobSetBuilder;
use serde::Deserialize;

use crate::model::ModelPreference;

/// Where a proxy reads its per-service config from. Implementations must
/// return the raw file contents for a service id, or `None` when no config
/// exists (compiled-in defaults apply).
pub trait ConfigStore {
    /// Read the raw config file contents for `service_id`; `None` if absent.
    fn read(&self, service_id: &str) -> Result<Option<String>>;

    /// Human-readable location of the config file for `service_id`, used in
    /// startup logging and error messages.
    fn location(&self, service_id: &str) -> String;
}

/// Plain-filesystem `ConfigStore`: `<root>/<service-id>.jsonc`.
pub struct JsoncDiskConfigStore {
    root: PathBuf,
}

impl JsoncDiskConfigStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl ConfigStore for JsoncDiskConfigStore {
    fn read(&self, service_id: &str) -> Result<Option<String>> {
        let path = self.root.join(config_filename(service_id));
        match std::fs::read_to_string(&path) {
            Ok(raw) => Ok(Some(raw)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => {
                Err(err).with_context(|| format!("reading proxy config {}", path.display()))
            }
        }
    }

    fn location(&self, service_id: &str) -> String {
        self.root
            .join(config_filename(service_id))
            .display()
            .to_string()
    }
}

/// Filename of a service's settings file inside the config dir.
pub fn config_filename(service_id: &str) -> String {
    format!("{service_id}.jsonc")
}

/// Proxy logging verbosity, set via the `log_level` settings key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    #[default]
    Normal,
    Verbose,
}

/// Compiled-in defaults a proxy supplies when no settings file exists. Also
/// the fallback for individual keys the file omits.
#[derive(Debug, Clone)]
pub struct ServiceDefaults {
    pub upstream_base_url: &'static str,
    pub model_exclude_globs: &'static [&'static str],
}

/// Raw shape of `<service-id>.jsonc`. Every key is optional; omitted keys
/// fall back to the service's [`ServiceDefaults`].
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SettingsFile {
    upstream_base_url: Option<String>,
    /// Service kill-switch: `false` disables the whole service even when the
    /// key is present. Final enablement is this AND key-present.
    enabled: Option<bool>,
    model_exclude_globs: Option<Vec<String>>,
    model_overrides: Option<HashMap<String, ModelOverride>>,
    log_level: Option<LogLevel>,
}

/// Per-model metadata override. Every field is optional; a present field
/// replaces the value the proxy would otherwise synthesize. Keys naming
/// unknown/retired models are ignored so stale entries do not break
/// discovery.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelOverride {
    /// System instructions served with the model in discovery. Mutually
    /// exclusive with `base_instructions_file`.
    pub base_instructions: Option<String>,
    /// Absolute path to a UTF-8 file whose contents replace
    /// `base_instructions`, for instructions too large for inline JSONC.
    /// Mutually exclusive with the inline form.
    pub base_instructions_file: Option<PathBuf>,
    /// Per-model lockdown: `false` removes the model from discovery AND
    /// refuses requests naming it. Default `true`.
    pub enabled: Option<bool>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub supported_reasoning_efforts: Option<Vec<String>>,
    pub preference: Option<ModelPreference>,
    pub upstream_model_id: Option<String>,
    pub cost_input_per_million_usd: Option<f64>,
    pub cost_output_per_million_usd: Option<f64>,
}

/// The effective configuration a proxy runs with.
#[derive(Debug)]
pub struct ResolvedConfig {
    /// Upstream override from the file, if any. CLI flags take precedence;
    /// the caller falls back to [`ServiceDefaults::upstream_base_url`].
    pub upstream_base_url: Option<String>,
    /// Service kill-switch from the file; `true` by default. Final
    /// enablement is this AND key-present.
    pub enabled: bool,
    /// Compiled exclusion set; matches are dropped from model discovery.
    pub exclude: GlobSet,
    /// Number of glob patterns in [`Self::exclude`], for startup logging.
    pub exclude_pattern_count: usize,
    pub log_level: LogLevel,
    /// Resolved per-model overrides (`base_instructions_file` contents
    /// inlined), keyed by upstream model ID.
    pub model_overrides: HashMap<String, ModelOverride>,
    /// Location of the settings file when one was loaded; `None` means
    /// compiled-in defaults are in use.
    pub loaded_from: Option<String>,
}

impl ResolvedConfig {
    /// Whether a discovered model survives filtering and lockdown: not
    /// excluded by globs and not explicitly disabled by an override. This is
    /// half of the spec §5 enforcement; the service gate (key presence) is the
    /// other half.
    pub fn model_allowed(&self, model_id: &str) -> bool {
        !self.exclude.is_match(model_id)
            && self
                .model_overrides
                .get(model_id)
                .is_none_or(|ovr| ovr.enabled.unwrap_or(true))
    }

    /// Service-level enablement: operator intent AND key presence. The
    /// `env_lookup` is passed in (rather than read from the process
    /// environment) so tests stay hermetic.
    pub fn service_enabled(
        &self,
        api_key_env_var: &str,
        env_lookup: impl Fn(&str) -> Option<String>,
    ) -> bool {
        self.enabled && env_lookup(api_key_env_var).is_some_and(|key| !key.trim().is_empty())
    }
}

/// Load and resolve `<service-id>.jsonc` from `store` against `defaults`.
///
/// A missing file resolves entirely to `defaults`. A present but malformed
/// file (bad JSONC, unknown key, invalid glob) is a hard error — silently
/// ignoring a broken config is how the wrong model ends up running.
pub fn load_config(
    store: &dyn ConfigStore,
    service_id: &str,
    defaults: &ServiceDefaults,
) -> Result<ResolvedConfig> {
    let raw = store.read(service_id)?;
    match raw {
        None => resolve(None, defaults, None),
        Some(raw) => {
            let file: SettingsFile = json5::from_str(&raw)
                .with_context(|| format!("parsing proxy config {}", store.location(service_id)))?;
            resolve(Some(file), defaults, Some(store.location(service_id)))
        }
    }
}

fn resolve(
    file: Option<SettingsFile>,
    defaults: &ServiceDefaults,
    loaded_from: Option<String>,
) -> Result<ResolvedConfig> {
    let file = file.unwrap_or_default();

    let patterns: Vec<String> = match file.model_exclude_globs {
        Some(list) => list,
        None => defaults
            .model_exclude_globs
            .iter()
            .map(ToString::to_string)
            .collect(),
    };

    let mut builder = GlobSetBuilder::new();
    for pattern in &patterns {
        let glob = Glob::new(pattern)
            .with_context(|| format!("invalid model exclusion glob {pattern:?}"))?;
        builder.add(glob);
    }
    let exclude = builder.build().context("compiling model exclusion globs")?;

    let mut model_overrides = HashMap::new();
    for (model_id, ovr) in file.model_overrides.unwrap_or_default() {
        if ovr.base_instructions.is_some() && ovr.base_instructions_file.is_some() {
            anyhow::bail!(
                "model override for {model_id:?} sets both base_instructions and \
                 base_instructions_file; pick one"
            );
        }
        let ovr = if let Some(path) = ovr.base_instructions_file.clone() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading base_instructions_file {}", path.display()))?;
            ModelOverride {
                base_instructions: Some(text),
                base_instructions_file: None,
                ..ovr
            }
        } else {
            ovr
        };
        model_overrides.insert(model_id, ovr);
    }

    Ok(ResolvedConfig {
        upstream_base_url: file.upstream_base_url,
        enabled: file.enabled.unwrap_or(true),
        exclude_pattern_count: patterns.len(),
        exclude,
        log_level: file.log_level.unwrap_or_default(),
        model_overrides,
        loaded_from,
    })
}

/// Resolve the effective upstream base URL: CLI flag > config file >
/// compiled-in default.
pub fn resolve_upstream_base(
    cli_flag: Option<String>,
    config: &ResolvedConfig,
    default: &str,
) -> String {
    cli_flag
        .or_else(|| config.upstream_base_url.clone())
        .unwrap_or_else(|| default.to_string())
        .trim_end_matches('/')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::path::Path;

    const MISTRAL_DEFAULTS: ServiceDefaults = ServiceDefaults {
        upstream_base_url: "https://api.mistral.ai/v1",
        model_exclude_globs: &[
            "*-ocr-*",
            "*-mini-*",
            "magistral-*",
            "ministral-*",
            "voxtral-*",
            "glm-5-2",
        ],
    };

    struct MemoryStore {
        files: HashMap<String, String>,
    }

    impl ConfigStore for MemoryStore {
        fn read(&self, service_id: &str) -> Result<Option<String>> {
            Ok(self.files.get(service_id).cloned())
        }
        fn location(&self, service_id: &str) -> String {
            format!("/memory/{service_id}.jsonc")
        }
    }

    fn load(store: &MemoryStore) -> Result<ResolvedConfig> {
        load_config(store, "mistral-ai", &MISTRAL_DEFAULTS)
    }

    fn disk_load(dir: &Path) -> Result<ResolvedConfig> {
        load_config(
            &JsoncDiskConfigStore::new(dir),
            "mistral-ai",
            &MISTRAL_DEFAULTS,
        )
    }

    #[test]
    fn missing_file_uses_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = disk_load(dir.path()).expect("load");
        assert_eq!(cfg.loaded_from, None);
        assert_eq!(cfg.upstream_base_url, None);
        assert_eq!(cfg.log_level, LogLevel::Normal);
        assert_eq!(cfg.exclude_pattern_count, 6);
        assert!(cfg.enabled);
    }

    #[test]
    fn default_globs_cover_prefix_suffix_contains_and_exact() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = disk_load(dir.path()).expect("load");
        let excluded = |id: &str| cfg.exclude.is_match(id);

        assert!(excluded("mistral-ocr-2512"));
        assert!(excluded("mistral-ocr-latest"));
        assert!(excluded("voxtral-mini-latest"));
        assert!(excluded("voxtral-mini-tts-2603"));
        assert!(excluded("magistral-small-latest"));
        assert!(excluded("ministral-3b-2512"));
        // Exact-match ban of the bare alias only.
        assert!(excluded("glm-5-2"));
        assert!(!excluded("zai-glm-5-2"));

        assert!(!excluded("mistral-medium-latest"));
        assert!(!excluded("mistral-large-latest"));
        assert!(!excluded("mistral-code-agent-latest"));
        assert!(!excluded("devstral-latest"));
        assert!(!excluded("codestral-latest"));
    }

    #[test]
    fn file_overrides_globs_and_endpoint_and_log_level() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("mistral-ai.jsonc"),
            r#"{
                // jsonc comments must parse
                "upstream_base_url": "https://example.invalid/v1",
                "model_exclude_globs": ["devstral-*",],
                "log_level": "verbose",
            }"#,
        )
        .expect("write config");
        let cfg = disk_load(dir.path()).expect("load");
        assert_eq!(
            cfg.loaded_from,
            Some(dir.path().join("mistral-ai.jsonc").display().to_string())
        );
        assert_eq!(
            cfg.upstream_base_url.as_deref(),
            Some("https://example.invalid/v1")
        );
        assert_eq!(cfg.log_level, LogLevel::Verbose);
        assert_eq!(cfg.exclude_pattern_count, 1);
        assert!(cfg.exclude.is_match("devstral-latest"));
        // Overriding the list replaces the defaults entirely.
        assert!(!cfg.exclude.is_match("ministral-3b-2512"));
    }

    #[test]
    fn empty_glob_list_disables_filtering() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("mistral-ai.jsonc"),
            r#"{ "model_exclude_globs": [] }"#,
        )
        .expect("write config");
        let cfg = disk_load(dir.path()).expect("load");
        assert_eq!(cfg.exclude_pattern_count, 0);
        assert!(!cfg.exclude.is_match("mistral-ocr-2512"));
    }

    #[test]
    fn service_kill_switch_defaults_true_and_file_can_disable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = disk_load(dir.path()).expect("load");
        assert!(cfg.enabled);

        std::fs::write(
            dir.path().join("mistral-ai.jsonc"),
            r#"{ "enabled": false }"#,
        )
        .expect("write config");
        let cfg = disk_load(dir.path()).expect("load");
        assert!(!cfg.enabled);
    }

    fn env_with(key_val: &'static str) -> impl Fn(&str) -> Option<String> {
        move |var| (var == "MISTRAL_API_KEY").then(|| key_val.to_string())
    }

    #[test]
    fn service_enabled_and_gates_key_presence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = disk_load(dir.path()).expect("load");
        assert!(cfg.service_enabled("MISTRAL_API_KEY", env_with("abc")));
        assert!(!cfg.service_enabled("MISTRAL_API_KEY", env_with("")));
        assert!(!cfg.service_enabled("MISTRAL_API_KEY", env_with("  ")));
        assert!(!cfg.service_enabled("OTHER_VAR", env_with("abc")));

        std::fs::write(
            dir.path().join("mistral-ai.jsonc"),
            r#"{ "enabled": false }"#,
        )
        .expect("write config");
        let cfg = disk_load(dir.path()).expect("load");
        assert!(!cfg.service_enabled("MISTRAL_API_KEY", env_with("abc")));
    }

    #[test]
    fn model_allowed_blocks_excluded_and_disabled_models() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = disk_load(dir.path()).expect("load");
        assert!(!cfg.model_allowed("ministral-3b-2512"));
        assert!(cfg.model_allowed("mistral-large-latest"));

        std::fs::write(
            dir.path().join("mistral-ai.jsonc"),
            r#"{ "model_overrides": { "mistral-large-latest": { "enabled": false } } }"#,
        )
        .expect("write config");
        let cfg = disk_load(dir.path()).expect("load");
        assert!(!cfg.model_allowed("mistral-large-latest"));
        // No override for this one: default enabled.
        assert!(cfg.model_allowed("mistral-medium-latest"));
    }

    #[test]
    fn model_overrides_inline_instructions() {
        let store = MemoryStore {
            files: [(
                "mistral-ai".to_string(),
                r#"{
                    "model_overrides": {
                        "zai-glm-5-2": { "base_instructions": "You are zai-glm-5-2." }
                    }
                }"#
                .to_string(),
            )]
            .into_iter()
            .collect(),
        };
        let cfg = load(&store).expect("load");
        let ovr = cfg
            .model_overrides
            .get("zai-glm-5-2")
            .expect("override present");
        assert_eq!(
            ovr.base_instructions.as_deref(),
            Some("You are zai-glm-5-2.")
        );
    }

    #[test]
    fn model_overrides_instructions_file_is_inlined() {
        let dir = tempfile::tempdir().expect("tempdir");
        let instructions_path = dir.path().join("glm-instructions.md");
        std::fs::write(&instructions_path, "You are zai-glm-5-2 from a file.").expect("write md");
        std::fs::write(
            dir.path().join("mistral-ai.jsonc"),
            format!(
                r#"{{ "model_overrides": {{ "zai-glm-5-2": {{ "base_instructions_file": "{}" }} }} }}"#,
                instructions_path.display()
            ),
        )
        .expect("write config");
        let cfg = disk_load(dir.path()).expect("load");
        let ovr = cfg
            .model_overrides
            .get("zai-glm-5-2")
            .expect("override present");
        assert_eq!(
            ovr.base_instructions.as_deref(),
            Some("You are zai-glm-5-2 from a file.")
        );
    }

    #[test]
    fn model_overrides_both_fields_is_a_hard_error() {
        let store = MemoryStore {
            files: [(
                "mistral-ai".to_string(),
                r#"{
                    "model_overrides": {
                        "x": { "base_instructions": "a", "base_instructions_file": "/tmp/b.md" }
                    }
                }"#
                .to_string(),
            )]
            .into_iter()
            .collect(),
        };
        load(&store).expect_err("conflicting fields must fail");
    }

    #[test]
    fn model_overrides_missing_file_is_a_hard_error() {
        let store = MemoryStore {
            files: [(
                "mistral-ai".to_string(),
                r#"{
                    "model_overrides": { "x": { "base_instructions_file": "/nonexistent/nope.md" } }
                }"#
                .to_string(),
            )]
            .into_iter()
            .collect(),
        };
        let err = load(&store).expect_err("missing file must fail");
        assert!(format!("{err:#}").contains("nope.md"));
    }

    #[test]
    fn malformed_jsonc_is_a_hard_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("mistral-ai.jsonc"), "{ this is not jsonc")
            .expect("write config");
        let err = disk_load(dir.path()).expect_err("must fail");
        assert!(format!("{err:#}").contains("mistral-ai.jsonc"));
    }

    #[test]
    fn unknown_key_is_a_hard_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("mistral-ai.jsonc"),
            r#"{ "modle_exclude_globs": ["x"] }"#,
        )
        .expect("write config");
        disk_load(dir.path()).expect_err("typo key must fail");
    }

    #[test]
    fn invalid_glob_is_a_hard_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("mistral-ai.jsonc"),
            r#"{ "model_exclude_globs": ["[unclosed"] }"#,
        )
        .expect("write config");
        let err = disk_load(dir.path()).expect_err("must fail");
        assert!(format!("{err:#}").contains("[unclosed"));
    }

    #[test]
    fn upstream_base_precedence_flag_then_config_then_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = disk_load(dir.path()).expect("load");
        assert_eq!(
            resolve_upstream_base(None, &cfg, MISTRAL_DEFAULTS.upstream_base_url),
            "https://api.mistral.ai/v1"
        );
        std::fs::write(
            dir.path().join("mistral-ai.jsonc"),
            r#"{ "upstream_base_url": "https://example.invalid/v1/" }"#,
        )
        .expect("write config");
        let cfg = disk_load(dir.path()).expect("load");
        assert_eq!(
            resolve_upstream_base(None, &cfg, MISTRAL_DEFAULTS.upstream_base_url),
            "https://example.invalid/v1"
        );
        assert_eq!(
            resolve_upstream_base(
                Some("https://flag.invalid/v1//".to_string()),
                &cfg,
                MISTRAL_DEFAULTS.upstream_base_url
            ),
            "https://flag.invalid/v1"
        );
    }
}
