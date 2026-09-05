//! Codex-compatible model discovery types
//! ([`schemas/models-response.jtd.json`](../../../schemas/models-response.jtd.json)).
//!
//! `GET /v1/models` MUST return this exact `{"models":[…]}` shape: harnesses
//! decode it strictly and silently fall back to bundled catalogs on a
//! mismatch. Field names, null-vs-omitted behavior, and serde renames mirror
//! OpenAI codex's `ModelInfo` wire format exactly (verified against
//! `codex-rs/protocol/src/openai_models.rs`); this module is a faithful
//! minimal port of the parts of that wire type a proxy actually constructs.
//! `model_messages` and `used_fallback_model_metadata` are omitted: the
//! former is always `None` for our proxies (skipped on the wire), the latter
//! never crosses the wire.

use serde::Deserialize;
use serde::Serialize;

/// Reasoning effort levels as they appear in `supported_reasoning_levels`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

/// A named reasoning level with its user-facing description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningEffortPreset {
    pub effort: ReasoningEffort,
    pub description: String,
}

/// How the model's context is truncated when full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TruncationMode {
    Bytes,
    Tokens,
}

/// Configured truncation mode and limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruncationPolicyConfig {
    pub mode: TruncationMode,
    pub limit: i64,
}

impl TruncationPolicyConfig {
    pub const fn bytes(limit: i64) -> Self {
        Self {
            mode: TruncationMode::Bytes,
            limit,
        }
    }

    pub const fn tokens(limit: i64) -> Self {
        Self {
            mode: TruncationMode::Tokens,
            limit,
        }
    }
}

/// Which shell tool implementation the model should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigShellToolType {
    Default,
    Local,
    UnifiedExec,
    Disabled,
    ShellCommand,
}

/// Whether a model appears in pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelVisibility {
    /// Listed everywhere.
    List,
    /// Hidden from the initial list, still selectable via /model.
    Hide,
    /// Not selectable at all.
    None,
}

/// Reasoning summary verbosity the model supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningSummary {
    #[default]
    Auto,
    Concise,
    Detailed,
    None,
}

/// Response verbosity the model supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verbosity {
    Low,
    Medium,
    High,
}

/// How image inputs may be passed to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchToolType {
    #[default]
    Text,
    TextAndImage,
}

/// The patch-application tool variant the model expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPatchToolType {
    Default,
    Diff,
}

/// Input modality, in codex-compat shape (text/image only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputModality {
    Text,
    Image,
}

impl InputModality {
    /// Default modality set for models whose catalog entry omits it.
    pub fn default_input_modalities() -> Vec<Self> {
        vec![Self::Text, Self::Image]
    }
}

/// Nudge shown once when a model becomes available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAvailabilityNux {
    pub message: String,
}

/// Suggested migration when a model is retired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfoUpgrade {
    pub model: String,
    pub migration_markdown: String,
}

/// A model's entry in a `ModelsResponse`, as consumed by codex-family
/// harnesses. Only what the wire format needs; a proxy fills in safe
/// defaults for fields the upstream catalog does not provide.
///
/// Wire behavior mirrored from codex: `description`, `availability_nux`,
/// `upgrade`, `default_verbosity`, and `apply_patch_tool_type` serialize as
/// JSON `null` when unset; `default_reasoning_level`, `context_window`, and
/// `auto_compact_token_limit` are omitted when unset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning_level: Option<ReasoningEffort>,
    pub supported_reasoning_levels: Vec<ReasoningEffortPreset>,
    pub shell_type: ConfigShellToolType,
    pub visibility: ModelVisibility,
    pub supported_in_api: bool,
    pub priority: i32,
    #[serde(default)]
    pub additional_speed_tiers: Vec<String>,
    pub availability_nux: Option<ModelAvailabilityNux>,
    pub upgrade: Option<ModelInfoUpgrade>,
    pub base_instructions: String,
    pub supports_reasoning_summaries: bool,
    #[serde(default)]
    pub default_reasoning_summary: ReasoningSummary,
    pub support_verbosity: bool,
    pub default_verbosity: Option<Verbosity>,
    pub apply_patch_tool_type: Option<ApplyPatchToolType>,
    #[serde(default)]
    pub web_search_tool_type: WebSearchToolType,
    pub truncation_policy: TruncationPolicyConfig,
    pub supports_parallel_tool_calls: bool,
    #[serde(default)]
    pub supports_image_detail_original: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    /// Token threshold for automatic compaction. When omitted, harnesses
    /// derive it from `context_window` (90%).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compact_token_limit: Option<i64>,
    /// Percentage of the context window considered usable for inputs.
    pub effective_context_window_percent: i64,
    pub experimental_supported_tools: Vec<String>,
    /// Input modalities accepted by the backend for this model.
    #[serde(default = "InputModality::default_input_modalities")]
    pub input_modalities: Vec<InputModality>,
    #[serde(default)]
    pub supports_search_tool: bool,
}

/// Response body of `GET /v1/models` (and `/models`): the codex-compatible
/// picker list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelsResponse {
    pub models: Vec<ModelInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Minimal `ModelInfo` in the shape the reference proxy synthesizes; all
    /// wire-visible fields must round-trip.
    fn test_model() -> ModelInfo {
        ModelInfo {
            slug: "mistral-large-latest".to_string(),
            display_name: "Mistral Large".to_string(),
            description: Some("Mistral's flagship model".to_string()),
            supported_reasoning_levels: vec![ReasoningEffortPreset {
                effort: ReasoningEffort::High,
                description: "Thinks longer".to_string(),
            }],
            default_reasoning_level: Some(ReasoningEffort::Low),
            shell_type: ConfigShellToolType::ShellCommand,
            visibility: ModelVisibility::List,
            supported_in_api: true,
            priority: 1,
            additional_speed_tiers: vec![],
            availability_nux: None,
            upgrade: None,
            base_instructions: "You are a coding agent.".to_string(),
            supports_reasoning_summaries: true,
            default_reasoning_summary: ReasoningSummary::Auto,
            support_verbosity: true,
            default_verbosity: Some(Verbosity::Medium),
            apply_patch_tool_type: Some(ApplyPatchToolType::Default),
            web_search_tool_type: WebSearchToolType::Text,
            truncation_policy: TruncationPolicyConfig::tokens(4_096),
            supports_parallel_tool_calls: true,
            supports_image_detail_original: true,
            effective_context_window_percent: 95,
            experimental_supported_tools: vec![],
            input_modalities: InputModality::default_input_modalities(),
            supports_search_tool: true,
            context_window: Some(128_000),
            auto_compact_token_limit: Some(121_600),
        }
    }

    #[test]
    fn output_round_trips_through_models_response_deserializer() {
        let response = ModelsResponse {
            models: vec![test_model()],
        };
        // Mirrors the exact call codex-api makes at its models endpoint.
        let json = serde_json::to_string(&response).expect("serialize");
        let reparsed: ModelsResponse = serde_json::from_str(&json).expect("deserialize");
        let reserialized = serde_json::to_string(&reparsed).expect("reserialize");
        assert_eq!(json, reserialized);
        assert_eq!(reparsed, response);
    }

    #[test]
    fn wire_names_match_codex_compatibility() {
        let response = ModelsResponse {
            models: vec![test_model()],
        };
        let json = serde_json::to_value(&response).expect("serialize");
        let model = &json["models"][0];
        assert_eq!(model["slug"], serde_json::json!("mistral-large-latest"));
        assert_eq!(model["shell_type"], serde_json::json!("shell_command"));
        assert_eq!(model["visibility"], serde_json::json!("list"));
        assert_eq!(
            model["default_reasoning_summary"],
            serde_json::json!("auto")
        );
        assert_eq!(model["default_verbosity"], serde_json::json!("medium"));
        assert_eq!(model["web_search_tool_type"], serde_json::json!("text"));
        assert_eq!(model["apply_patch_tool_type"], serde_json::json!("default"));
        assert_eq!(
            model["truncation_policy"]["mode"],
            serde_json::json!("tokens")
        );
        assert_eq!(
            model["input_modalities"],
            serde_json::json!(["text", "image"])
        );
        // Nulls serialize as null (not omitted): codex Option without skip.
        assert_eq!(model["availability_nux"], serde_json::json!(null));
        assert_eq!(model["upgrade"], serde_json::json!(null));
        assert_eq!(
            model["description"],
            serde_json::json!("Mistral's flagship model")
        );
    }

    #[test]
    fn omitted_fields_stay_off_the_wire() {
        let mut model = test_model();
        model.default_reasoning_level = None;
        model.context_window = None;
        model.auto_compact_token_limit = None;
        let json = serde_json::to_value(ModelsResponse {
            models: vec![model],
        })
        .expect("serialize");
        let model = &json["models"][0];
        assert!(model.get("default_reasoning_level").is_none());
        assert!(model.get("context_window").is_none());
        assert!(model.get("auto_compact_token_limit").is_none());
    }

    #[test]
    fn absent_optional_fields_deserialize_with_defaults() {
        let json = r#"{"models":[{
            "slug": "m",
            "display_name": "M",
            "description": null,
            "supported_reasoning_levels": [],
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 0,
            "availability_nux": null,
            "upgrade": null,
            "base_instructions": "",
            "supports_reasoning_summaries": false,
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": null,
            "truncation_policy": {"mode": "bytes", "limit": 10000},
            "supports_parallel_tool_calls": false,
            "effective_context_window_percent": 95,
            "experimental_supported_tools": [],
            "supports_search_tool": false
        }]}"#;
        let response: ModelsResponse = serde_json::from_str(json).expect("deserialize");
        let model = &response.models[0];
        assert_eq!(model.web_search_tool_type, WebSearchToolType::Text);
        assert_eq!(
            model.input_modalities,
            InputModality::default_input_modalities()
        );
        assert_eq!(model.default_reasoning_summary, ReasoningSummary::Auto);
        assert!(!model.supports_image_detail_original);
    }
}
