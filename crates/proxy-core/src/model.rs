//! The complete model value object
//! ([`schemas/model.jtd.json`](../../../schemas/model.jtd.json)).
//!
//! "Complete" means: everything a harness needs to decide to use a model,
//! including which credential the proxy spends on its behalf. Custom system
//! prompts (`base_instructions`), cost, user preference, and skills are
//! proxy-side concerns; the harness discovers them, never configures them.

use serde::Deserialize;
use serde::Serialize;

/// Input modality a model accepts. Prompt Cult's own object includes
/// `audio` beyond the codex-compat pair; consumers must tolerate either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputModality {
    Text,
    Image,
    Audio,
}

/// Operator's preference for a model. Reserved (forward-compatible):
/// consumers MUST tolerate absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelPreference {
    /// Top of the picker list.
    Pinned,
    /// The service's default pick.
    Default,
    /// Excluded from listing but still requestable.
    Hidden,
}

/// A complete model value object served by discovery. Only enabled models
/// are ever listed; `enabled: true` in a discovery response is invariant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelDocument {
    /// Proxy-visible model ID a harness uses in requests.
    pub id: String,
    /// Owning service's stable slug.
    pub service_id: String,
    /// Always true in discovery responses; the field exists so config-facing
    /// contexts can reuse the same value object.
    pub enabled: bool,
    /// Name of the environment variable the proxy reads the key from.
    /// Denormalized from the service so a model object is self-contained.
    pub api_key_env_var: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_reasoning_efforts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modalities: Vec<InputModality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_input_per_million_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_output_per_million_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preference: Option<ModelPreference>,
    /// ID forwarded upstream when it differs from `id` (e.g. after
    /// prefix stripping). Absent means the upstream ID equals `id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_model_id: Option<String>,
    /// Reserved for future per-model skill advertisement.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
}

impl ModelDocument {
    /// A minimal, valid document: required fields only, all optional fields
    /// omitted. `api_key_env_var` must equal the owning service's value.
    pub fn new(id: impl Into<String>, service: &crate::service::Service) -> Self {
        Self {
            id: id.into(),
            service_id: service.id.clone(),
            enabled: true,
            api_key_env_var: service.api_key_env_var.clone(),
            display_name: None,
            description: None,
            context_window: None,
            max_output_tokens: None,
            base_instructions: None,
            supported_reasoning_efforts: Vec::new(),
            input_modalities: Vec::new(),
            cost_input_per_million_usd: None,
            cost_output_per_million_usd: None,
            preference: None,
            upstream_model_id: None,
            skills: Vec::new(),
        }
    }

    /// The model ID to forward upstream: `upstream_model_id` when set, `id`
    /// otherwise. The mapping is explicit config, never inference.
    pub fn upstream_id(&self) -> &str {
        self.upstream_model_id.as_deref().unwrap_or(&self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Service;
    use crate::service::ServiceKind;
    use pretty_assertions::assert_eq;

    fn test_service() -> Service {
        Service {
            id: "mistral-ai".to_string(),
            vendor: "Mistral AI".to_string(),
            endpoint: "mistral".to_string(),
            api_key_env_var: "MISTRAL_API_KEY".to_string(),
            kind: ServiceKind::Inference,
            display_name: None,
            upstream_base_url: None,
            aliases: Vec::new(),
        }
    }

    #[test]
    fn minimal_document_serializes_required_fields_only() {
        let doc = ModelDocument::new("mistral-large-latest", &test_service());
        let json = serde_json::to_value(&doc).expect("serialize");
        let map = json.as_object().expect("object");
        assert_eq!(
            map.keys().collect::<Vec<_>>(),
            ["api_key_env_var", "enabled", "id", "service_id"]
        );
        assert_eq!(map["id"], serde_json::json!("mistral-large-latest"));
        assert_eq!(map["service_id"], serde_json::json!("mistral-ai"));
        assert_eq!(map["api_key_env_var"], serde_json::json!("MISTRAL_API_KEY"));
        assert_eq!(map["enabled"], serde_json::json!(true));
    }

    #[test]
    fn full_document_round_trips() {
        let mut doc = ModelDocument::new("go-claude-sonnet-4-5", &test_service());
        doc.upstream_model_id = Some("claude-sonnet-4-5".to_string());
        doc.context_window = Some(200_000);
        doc.supported_reasoning_efforts = vec!["low".to_string(), "high".to_string()];
        doc.input_modalities = vec![InputModality::Text, InputModality::Image];
        doc.preference = Some(ModelPreference::Pinned);
        doc.cost_input_per_million_usd = Some(3.0);
        doc.cost_output_per_million_usd = Some(15.0);
        doc.skills = vec!["code-review".to_string()];
        let json = serde_json::to_string(&doc).expect("serialize");
        let reparsed: ModelDocument = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(reparsed, doc);
    }

    #[test]
    fn upstream_id_prefers_explicit_mapping() {
        let mut doc = ModelDocument::new("go-gpt-5-mini", &test_service());
        assert_eq!(doc.upstream_id(), "go-gpt-5-mini");
        doc.upstream_model_id = Some("gpt-5-mini".to_string());
        assert_eq!(doc.upstream_id(), "gpt-5-mini");
    }

    #[test]
    fn reserved_fields_are_forward_compatible() {
        // A consumer that does not know the reserved fields still round-trips.
        let json = r#"{
            "id": "m", "service_id": "s", "enabled": true, "api_key_env_var": "K",
            "preference": "default", "cost_input_per_million_usd": 1.5, "skills": ["x"]
        }"#;
        let doc: ModelDocument = serde_json::from_str(json).expect("deserialize");
        assert_eq!(doc.preference, Some(ModelPreference::Default));
        let _ = serde_json::to_string(&doc).expect("serialize");
    }
}
