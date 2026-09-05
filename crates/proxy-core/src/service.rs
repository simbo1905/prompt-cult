//! The inference service abstraction: **vendor + endpoint + key**
//! ([`schemas/service.jtd.json`](../../../schemas/service.jtd.json)).
//!
//! An inference service is the triple vendor (the platform), endpoint (the
//! vendor's short name for the API surface: `go`, `zen`, `mistral`; mono
//! providers use the vendor name), and key (one API key per service, sourced
//! from a named environment variable). One vendor may run several endpoints
//! whose entitlements differ; the service abstraction models them as
//! distinct services with independent enablement.

use serde::Deserialize;
use serde::Serialize;

use crate::model::ModelDocument;

/// The kind of service exposed at `GET /service`. v1 is deliberately closed
/// at `inference`; future kinds (e.g. MCP tool services) extend this enum and
/// the JTD enum together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceKind {
    Inference,
}

impl ServiceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inference => "inference",
        }
    }
}

/// Static description of a service a proxy fronts. Enablement is NOT stored
/// here: it is derived at runtime (key presence AND operator intent), so a
/// service object stays valid whatever environment it is deployed into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    /// Stable, lowercase, hyphenated slug, conventionally `vendor-endpoint`
    /// (`opencode-go`, `opencode-zen`, `mistral-ai`). The wire id never
    /// changes once a proxy ships, or existing configs silently stop loading.
    pub id: String,
    /// The platform: "OpenCode AI", "Mistral AI", "Anthropic", "OpenAI".
    pub vendor: String,
    /// The vendor's short name for the API surface (`go`, `zen`, `mistral`;
    /// mono providers use the vendor name).
    pub endpoint: String,
    /// The NAME of the environment variable holding the key, never its value.
    pub api_key_env_var: String,
    pub kind: ServiceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_base_url: Option<String>,
    /// Alternative names other tools may use for this service; advisory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

impl Service {
    /// A service is key-enabled iff its `api_key_env_var` resolves to a
    /// non-empty value. Whether a `.env` file got loaded into the environment
    /// is a deployment concern outside the core: the core observes only
    /// "key present: true|false".
    pub fn key_present(&self, env_lookup: impl Fn(&str) -> Option<String>) -> bool {
        env_lookup(&self.api_key_env_var).is_some_and(|key| !key.trim().is_empty())
    }

    /// Build the wire document for `GET /service`, embedding only the enabled
    /// models. Discovery responses never list disabled models.
    pub fn document(&self, enabled: bool, enabled_models: Vec<ModelDocument>) -> ServiceDocument {
        ServiceDocument {
            id: self.id.clone(),
            vendor: self.vendor.clone(),
            endpoint: self.endpoint.clone(),
            api_key_env_var: self.api_key_env_var.clone(),
            enabled,
            kind: self.kind,
            display_name: self.display_name.clone(),
            upstream_base_url: self.upstream_base_url.clone(),
            aliases: self.aliases.clone(),
            models: enabled_models,
        }
    }
}

/// A service entry as served by `GET /service`
/// ([`schemas/service-list.jtd.json`](../../../schemas/service-list.jtd.json)).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceDocument {
    pub id: String,
    pub vendor: String,
    pub endpoint: String,
    pub api_key_env_var: String,
    /// Derived: key present AND operator intent. Only enabled services
    /// appear in a service list.
    pub enabled: bool,
    pub kind: ServiceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelDocument>,
}

/// Response body of `GET /service`: at least one `inference` service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceList {
    pub services: Vec<ServiceDocument>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn opencode_go() -> Service {
        Service {
            id: "opencode-go".to_string(),
            vendor: "OpenCode AI".to_string(),
            endpoint: "go".to_string(),
            api_key_env_var: "OPENCODE_API_KEY".to_string(),
            kind: ServiceKind::Inference,
            display_name: None,
            upstream_base_url: Some("https://opencode.ai/zen/go/v1".to_string()),
            aliases: vec!["go".to_string()],
        }
    }

    #[test]
    fn key_present_requires_non_empty_value() {
        let service = opencode_go();
        assert!(service.key_present(|var| (var == "OPENCODE_API_KEY").then(|| "k".to_string())));
        assert!(!service.key_present(|_| None));
        assert!(!service.key_present(|var| (var == "OPENCODE_API_KEY").then(|| "  ".to_string())));
    }

    #[test]
    fn document_embeds_enabled_models_and_derived_state() {
        let service = opencode_go();
        let mut model = ModelDocument::new("go-claude-sonnet-4-5", &service);
        model.upstream_model_id = Some("claude-sonnet-4-5".to_string());
        let doc = service.document(true, vec![model]);
        let json = serde_json::to_value(&doc).expect("serialize");
        assert_eq!(json["id"], serde_json::json!("opencode-go"));
        assert_eq!(json["vendor"], serde_json::json!("OpenCode AI"));
        assert_eq!(json["endpoint"], serde_json::json!("go"));
        assert_eq!(
            json["api_key_env_var"],
            serde_json::json!("OPENCODE_API_KEY")
        );
        assert_eq!(json["enabled"], serde_json::json!(true));
        assert_eq!(json["kind"], serde_json::json!("inference"));
        assert_eq!(
            json["models"][0]["upstream_model_id"],
            serde_json::json!("claude-sonnet-4-5")
        );
        let reparsed: ServiceDocument = serde_json::from_value(json).expect("deserialize");
        assert_eq!(reparsed, doc);
    }

    #[test]
    fn service_list_round_trips() {
        let service = opencode_go();
        let list = ServiceList {
            services: vec![service.document(true, Vec::new())],
        };
        let json = serde_json::to_string(&list).expect("serialize");
        let reparsed: ServiceList = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(reparsed, list);
    }
}
