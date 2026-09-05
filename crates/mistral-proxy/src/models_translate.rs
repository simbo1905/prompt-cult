//! Translate Mistral's raw `/models` response into the two discovery shapes
//! of `docs/proxy-spec.md` §6:
//!
//! - the codex-compatible `ModelsResponse` (`{"models":[ModelInfo,…]}`) for
//!   `GET /v1/models` pickers,
//! - the Prompt Cult `ModelDocument` list embedded in `GET /service`.
//!
//! Only chat-capable models are surfaced; embeddings, moderation, OCR, and
//! other non-chat models Mistral lists are filtered out. Spec §5 lockdown is
//! applied here too: models matching exclusion globs and models disabled via
//! a `model_overrides` entry never reach either listing.

use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::Context;
use anyhow::Result;
use prompt_cult_proxy_core::config::ModelOverride;
use prompt_cult_proxy_core::discovery::ConfigShellToolType;
use prompt_cult_proxy_core::discovery::InputModality;
use prompt_cult_proxy_core::discovery::ModelInfo;
use prompt_cult_proxy_core::discovery::ModelVisibility;
use prompt_cult_proxy_core::discovery::ModelsResponse;
use prompt_cult_proxy_core::discovery::ReasoningSummary;
use prompt_cult_proxy_core::discovery::TruncationPolicyConfig;
use prompt_cult_proxy_core::discovery::WebSearchToolType;
use prompt_cult_proxy_core::model::ModelDocument;
use prompt_cult_proxy_core::service::Service;
use serde::Deserialize;

/// Fallback context window used when Mistral omits `max_context_length`.
const DEFAULT_CONTEXT_WINDOW: i64 = 128_000;
/// Byte budget for input truncation; overridable per-model via config.
const DEFAULT_TRUNCATION_BYTES: i64 = 10_000;

/// Top-level Mistral `/models` envelope. Extra fields are ignored.
#[derive(Debug, Deserialize)]
struct MistralModelList {
    #[serde(default)]
    data: Vec<MistralModel>,
}

/// A single Mistral model entry. Only the fields we map are declared; all are
/// tolerant of omission so upstream schema drift does not hard-fail discovery.
#[derive(Debug, Deserialize)]
struct MistralModel {
    id: String,
    #[serde(default)]
    capabilities: MistralCapabilities,
    #[serde(default)]
    max_context_length: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct MistralCapabilities {
    #[serde(default)]
    completion_chat: bool,
}

/// Result of translating a Mistral `/models` payload: the same surviving
/// model set rendered in both discovery shapes.
#[derive(Debug)]
pub struct TranslatedModels {
    /// The codex `ModelsResponse` to serve `GET /v1/models`.
    pub response: ModelsResponse,
    /// The Prompt Cult model documents to embed in `GET /service`.
    pub documents: Vec<ModelDocument>,
    /// Chat-capable models seen upstream before exclusion/disabling, so the
    /// proxy can log `loaded N models, M after exclusions`.
    pub chat_loaded: usize,
}

impl TranslatedModels {
    /// Model IDs that survived filtering — the spec §5 request-time gate set.
    pub fn allowed_ids(&self) -> HashSet<String> {
        self.documents.iter().map(|doc| doc.id.clone()).collect()
    }
}

/// Parse raw Mistral `/models` JSON and build both discovery shapes.
///
/// Models whose ID matches any glob in `exclude`, and models disabled by an
/// override (`enabled: false`), are dropped before priorities are assigned,
/// so the surviving list is densely ordered. `overrides` is keyed by
/// upstream model ID; keys naming unknown models are ignored.
pub fn translate_mistral_models(
    raw: &[u8],
    service: &Service,
    exclude: &globset::GlobSet,
    overrides: &HashMap<String, ModelOverride>,
) -> Result<TranslatedModels> {
    let list: MistralModelList =
        serde_json::from_slice(raw).context("parsing Mistral /models response")?;

    let chat_capable: Vec<&MistralModel> = list
        .data
        .iter()
        .filter(|m| m.capabilities.completion_chat)
        .collect();
    let chat_loaded = chat_capable.len();

    let mut response_models = Vec::new();
    let mut documents = Vec::new();
    for (index, m) in chat_capable
        .into_iter()
        .filter(|m| {
            let ovr = overrides.get(&m.id);
            !exclude.is_match(&m.id) && ovr.is_none_or(|o| o.enabled.unwrap_or(true))
        })
        .enumerate()
    {
        let ovr = overrides.get(&m.id).cloned().unwrap_or_default();
        response_models.push(model_info_for(index, m, &ovr));
        documents.push(model_document_for(m, service, &ovr));
    }

    Ok(TranslatedModels {
        response: ModelsResponse {
            models: response_models,
        },
        documents,
        chat_loaded,
    })
}

/// Build a fully-populated [`ModelInfo`] for a chat-capable Mistral model.
///
/// `ModelInfo` has no `Default`, so every field is set explicitly. Optional and
/// reasoning-related metadata Mistral does not provide is left empty/`None`;
/// users can override context window and related limits via config.
/// Prompt Cult base instructions are served for every discovered model unless
/// a `model_overrides` entry replaces them for a specific model. Includes an
/// identity line so models answer "what model are you?" with the selected
/// model ID instead of an upstream alias name.
const DEFAULT_BASE_INSTRUCTIONS: &str = include_str!("../prompt.md");

fn model_info_for(index: usize, m: &MistralModel, ovr: &ModelOverride) -> ModelInfo {
    let context_window = ovr
        .context_window
        .map(i64::from)
        .or(m.max_context_length)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW);
    ModelInfo {
        slug: m.id.clone(),
        display_name: m.id.clone(),
        description: None,
        default_reasoning_level: None,
        supported_reasoning_levels: Vec::new(),
        shell_type: ConfigShellToolType::ShellCommand,
        visibility: ModelVisibility::List,
        supported_in_api: true,
        priority: index as i32,
        additional_speed_tiers: Vec::new(),
        availability_nux: None,
        upgrade: None,
        base_instructions: ovr
            .base_instructions
            .clone()
            .unwrap_or_else(|| DEFAULT_BASE_INSTRUCTIONS.to_string()),
        supports_reasoning_summaries: false,
        default_reasoning_summary: ReasoningSummary::Auto,
        support_verbosity: false,
        default_verbosity: None,
        apply_patch_tool_type: None,
        web_search_tool_type: WebSearchToolType::Text,
        truncation_policy: TruncationPolicyConfig::bytes(DEFAULT_TRUNCATION_BYTES),
        supports_parallel_tool_calls: true,
        supports_image_detail_original: false,
        context_window: Some(context_window),
        auto_compact_token_limit: None,
        effective_context_window_percent: 95,
        experimental_supported_tools: Vec::new(),
        input_modalities: InputModality::default_input_modalities(),
        supports_search_tool: false,
    }
}

/// Build the Prompt Cult [`ModelDocument`] for a chat-capable Mistral model,
/// carrying every per-model override the config supplies (spec §4: the model
/// object is complete, including the credential name).
fn model_document_for(m: &MistralModel, service: &Service, ovr: &ModelOverride) -> ModelDocument {
    let mut doc = ModelDocument::new(m.id.clone(), service);
    doc.display_name = Some(m.id.clone());
    let context_window = ovr
        .context_window
        .map(i64::from)
        .or(m.max_context_length)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW);
    doc.context_window = Some(context_window.clamp(0, u32::MAX as i64) as u32);
    doc.max_output_tokens = ovr.max_output_tokens;
    doc.base_instructions = Some(
        ovr.base_instructions
            .clone()
            .unwrap_or_else(|| DEFAULT_BASE_INSTRUCTIONS.to_string()),
    );
    doc.supported_reasoning_efforts = ovr.supported_reasoning_efforts.clone().unwrap_or_default();
    doc.upstream_model_id = ovr.upstream_model_id.clone();
    doc.cost_input_per_million_usd = ovr.cost_input_per_million_usd;
    doc.cost_output_per_million_usd = ovr.cost_output_per_million_usd;
    doc.preference = ovr.preference;
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const FIXTURE: &str = include_str!("../tests/fixtures/mistral_models.json");

    fn test_service() -> Service {
        Service {
            id: "mistral-ai".to_string(),
            vendor: "Mistral AI".to_string(),
            endpoint: "mistral".to_string(),
            api_key_env_var: "MISTRAL_API_KEY".to_string(),
            kind: prompt_cult_proxy_core::service::ServiceKind::Inference,
            display_name: None,
            upstream_base_url: None,
            aliases: Vec::new(),
        }
    }

    fn no_exclusions() -> globset::GlobSet {
        globset::GlobSetBuilder::new()
            .build()
            .expect("empty globset")
    }

    fn no_overrides() -> HashMap<String, ModelOverride> {
        HashMap::new()
    }

    fn globset_of(patterns: &[&str]) -> globset::GlobSet {
        let mut builder = globset::GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(globset::Glob::new(pattern).expect("valid glob"));
        }
        builder.build().expect("globset")
    }

    #[test]
    fn filters_to_chat_capable_models() {
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &no_exclusions(),
            &no_overrides(),
        )
        .expect("translate");
        // Fixture: zai-glm-5-2 and mistral-medium-latest are chat; mistral-embed is not.
        assert_eq!(out.response.models.len(), 2);
        assert_eq!(out.documents.len(), 2);
        assert_eq!(out.chat_loaded, 2);
        assert!(
            out.response
                .models
                .iter()
                .all(|m| m.slug != "mistral-embed")
        );
        assert!(out.documents.iter().all(|d| d.id != "mistral-embed"));
    }

    #[test]
    fn exclusion_globs_drop_matching_models() {
        let exclude = globset_of(&["zai-glm-5-2", "*-medium-*"]);
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &exclude,
            &no_overrides(),
        )
        .expect("translate");
        assert!(out.response.models.is_empty());
        assert_eq!(out.chat_loaded, 2);
    }

    #[test]
    fn exact_exclusion_keeps_prefixed_variant() {
        let exclude = globset_of(&["glm-5-2"]);
        let raw = br#"{"object":"list","data":[
            {"id":"glm-5-2","capabilities":{"completion_chat":true}},
            {"id":"zai-glm-5-2","capabilities":{"completion_chat":true}}
        ]}"#;
        let out = translate_mistral_models(raw, &test_service(), &exclude, &no_overrides())
            .expect("translate");
        assert_eq!(out.response.models.len(), 1);
        assert_eq!(out.response.models[0].slug, "zai-glm-5-2");
        // Priorities are dense after filtering.
        assert_eq!(out.response.models[0].priority, 0);
    }

    #[test]
    fn disabled_override_drops_model_from_both_shapes() {
        let mut overrides = no_overrides();
        overrides.insert(
            "zai-glm-5-2".to_string(),
            ModelOverride {
                enabled: Some(false),
                ..ModelOverride::default()
            },
        );
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &no_exclusions(),
            &overrides,
        )
        .expect("translate");
        assert!(out.response.models.iter().all(|m| m.slug != "zai-glm-5-2"));
        assert!(out.documents.iter().all(|d| d.id != "zai-glm-5-2"));
        assert_eq!(out.chat_loaded, 2);
    }

    #[test]
    fn mvp_model_has_expected_fields() {
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &no_exclusions(),
            &no_overrides(),
        )
        .expect("translate");
        let glm = out
            .response
            .models
            .iter()
            .find(|m| m.slug == "zai-glm-5-2")
            .expect("glm present");
        assert_eq!(glm.display_name, "zai-glm-5-2");
        assert_eq!(glm.visibility, ModelVisibility::List);
        assert_eq!(glm.shell_type, ConfigShellToolType::ShellCommand);
        assert!(glm.supported_in_api);
        assert_eq!(glm.context_window, Some(131_072));
    }

    #[test]
    fn defaults_context_window_when_missing() {
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &no_exclusions(),
            &no_overrides(),
        )
        .expect("translate");
        let medium = out
            .response
            .models
            .iter()
            .find(|m| m.slug == "mistral-medium-latest")
            .expect("medium present");
        assert_eq!(medium.context_window, Some(DEFAULT_CONTEXT_WINDOW));
    }

    #[test]
    fn output_round_trips_through_models_response_deserializer() {
        // Mirrors the exact call a codex-family harness makes at its models
        // endpoint: strict decode of the picker shape.
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &no_exclusions(),
            &no_overrides(),
        )
        .expect("translate");
        let json = serde_json::to_vec(&out.response).expect("serialize");
        let reparsed: ModelsResponse = serde_json::from_slice(&json).expect("deserialize");
        assert_eq!(reparsed, out.response);
    }

    #[test]
    fn documents_round_trip_through_model_document() {
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &no_exclusions(),
            &no_overrides(),
        )
        .expect("translate");
        let json = serde_json::to_vec(&out.documents).expect("serialize");
        let reparsed: Vec<ModelDocument> = serde_json::from_slice(&json).expect("deserialize");
        assert_eq!(reparsed, out.documents);
    }

    #[test]
    fn documents_carry_service_identity() {
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &no_exclusions(),
            &no_overrides(),
        )
        .expect("translate");
        let glm = out
            .documents
            .iter()
            .find(|d| d.id == "zai-glm-5-2")
            .expect("glm present");
        assert_eq!(glm.service_id, "mistral-ai");
        assert_eq!(glm.api_key_env_var, "MISTRAL_API_KEY");
        assert!(glm.enabled);
        assert_eq!(glm.display_name.as_deref(), Some("zai-glm-5-2"));
        assert_eq!(glm.context_window, Some(131_072));
        assert!(
            glm.base_instructions
                .as_deref()
                .is_some_and(|s| s.contains("Prompt Cult"))
        );
    }

    #[test]
    fn default_instructions_come_from_prompt_md() {
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &no_exclusions(),
            &no_overrides(),
        )
        .expect("translate");
        let glm = out
            .response
            .models
            .iter()
            .find(|m| m.slug == "zai-glm-5-2")
            .expect("glm present");
        assert!(glm.base_instructions.contains("Prompt Cult"));
        assert!(glm.base_instructions.contains("Model identity"));
    }

    #[test]
    fn override_replaces_base_instructions_for_one_model() {
        let mut overrides = no_overrides();
        overrides.insert(
            "zai-glm-5-2".to_string(),
            ModelOverride {
                base_instructions: Some("You are zai-glm-5-2, period.".to_string()),
                ..ModelOverride::default()
            },
        );
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &no_exclusions(),
            &overrides,
        )
        .expect("translate");
        let glm = out
            .response
            .models
            .iter()
            .find(|m| m.slug == "zai-glm-5-2")
            .expect("glm present");
        assert_eq!(glm.base_instructions, "You are zai-glm-5-2, period.");
        let medium = out
            .response
            .models
            .iter()
            .find(|m| m.slug == "mistral-medium-latest")
            .expect("medium present");
        assert!(medium.base_instructions.contains("Prompt Cult"));
    }

    #[test]
    fn overrides_enrich_model_documents() {
        let mut overrides = no_overrides();
        overrides.insert(
            "zai-glm-5-2".to_string(),
            ModelOverride {
                context_window: Some(200_000),
                max_output_tokens: Some(16_384),
                supported_reasoning_efforts: Some(vec!["high".to_string()]),
                upstream_model_id: Some("glm-5-2".to_string()),
                cost_input_per_million_usd: Some(0.5),
                cost_output_per_million_usd: Some(2.0),
                ..ModelOverride::default()
            },
        );
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &no_exclusions(),
            &overrides,
        )
        .expect("translate");
        let glm = out
            .documents
            .iter()
            .find(|d| d.id == "zai-glm-5-2")
            .expect("glm present");
        assert_eq!(glm.context_window, Some(200_000));
        assert_eq!(glm.max_output_tokens, Some(16_384));
        assert_eq!(glm.supported_reasoning_efforts, vec!["high".to_string()]);
        assert_eq!(glm.upstream_id(), "glm-5-2");
        assert_eq!(glm.cost_input_per_million_usd, Some(0.5));
        assert_eq!(glm.cost_output_per_million_usd, Some(2.0));
        // The codex shape carries the context window too.
        let info = out
            .response
            .models
            .iter()
            .find(|m| m.slug == "zai-glm-5-2")
            .expect("glm present");
        assert_eq!(info.context_window, Some(200_000));
    }

    #[test]
    fn allowed_ids_match_visible_documents() {
        let exclude = globset_of(&["mistral-embed"]);
        let out = translate_mistral_models(
            FIXTURE.as_bytes(),
            &test_service(),
            &exclude,
            &no_overrides(),
        )
        .expect("translate");
        assert_eq!(
            out.allowed_ids(),
            [
                "zai-glm-5-2".to_string(),
                "mistral-medium-latest".to_string()
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn empty_data_yields_empty_models() {
        let out = translate_mistral_models(
            br#"{"object":"list","data":[]}"#,
            &test_service(),
            &no_exclusions(),
            &no_overrides(),
        )
        .expect("translate");
        assert!(out.response.models.is_empty());
        assert!(out.documents.is_empty());
        assert_eq!(out.chat_loaded, 0);
    }
}
