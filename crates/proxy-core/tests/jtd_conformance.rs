//! RFC 8927 conformance: the serde objects in this crate must conform to the
//! JTD schemas in `schemas/`, and the composite `service-list.jtd.json` must
//! embed identical copies of the standalone `service` and `model` contracts
//! (RFC 8927 refs are same-document only, so the duplication is deliberate
//! and guarded).

use pretty_assertions::assert_eq;
use prompt_cult_proxy_core::error::ErrorCode;
use prompt_cult_proxy_core::error::ErrorDocument;
use prompt_cult_proxy_core::model::InputModality;
use prompt_cult_proxy_core::model::ModelDocument;
use prompt_cult_proxy_core::model::ModelPreference;
use prompt_cult_proxy_core::service::Service;
use prompt_cult_proxy_core::service::ServiceKind;
use prompt_cult_proxy_core::service::ServiceList;
use serde_json::Value;
use serde_json::json;
use std::collections::HashSet;

const SCHEMAS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas");

fn schema(name: &str) -> Value {
    let path = format!("{SCHEMAS_DIR}/{name}.jtd.json");
    let raw = std::fs::read_to_string(&path).expect("schema file exists");
    serde_json::from_str(&raw).expect("schema is valid JSON")
}

/// RFC 8927 JSON-type strings mapped to JSON values for type assertions.
fn jtd_type_matches(schema_type: &str, value: &Value) -> bool {
    match schema_type {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "uint32" => value.is_u64() && value.as_u64().expect("u64") <= u32::MAX as u64,
        "int64" => value.is_i64(),
        "float64" => value.is_number(),
        _ => false,
    }
}

/// Assert `document` conforms to a JTD object schema: every required property
/// present with the right type, no properties outside the declared set (when
/// additionalProperties is false), and enum values within the declared enum.
fn assert_conforms(document: &Value, schema: &Value) {
    let properties = schema
        .get("properties")
        .map(|p| p.as_object().expect("properties is object").clone())
        .unwrap_or_default();
    let optional = schema
        .get("optionalProperties")
        .map(|p| p.as_object().expect("optionalProperties is object").clone())
        .unwrap_or_default();
    let additional = schema
        .get("additionalProperties")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let obj = document.as_object().expect("document is object");
    for (name, def) in &properties {
        let value = obj
            .get(name)
            .unwrap_or_else(|| panic!("required property {name} missing in {document}"));
        if let Some(t) = def.get("type").and_then(Value::as_str) {
            assert!(
                jtd_type_matches(t, value),
                "property {name} should be JTD type {t}, got {value}"
            );
        }
        if let Some(enumeration) = def.get("enum").and_then(Value::as_array) {
            assert!(
                enumeration.contains(value),
                "property {name} value {value} not in enum {enumeration:?}"
            );
        }
    }
    for (name, def) in &optional {
        if let Some(value) = obj.get(name) {
            if let Some(t) = def.get("type").and_then(Value::as_str) {
                assert!(
                    jtd_type_matches(t, value),
                    "optional property {name} should be JTD type {t}, got {value}"
                );
            }
            if let Some(enumeration) = def.get("enum").and_then(Value::as_array) {
                assert!(
                    enumeration.contains(value),
                    "optional property {name} value {value} not in enum {enumeration:?}"
                );
            }
            if let Some(elements) = def.get("elements") {
                let list = value.as_array().expect("elements is array");
                for item in list {
                    if let Some(t) = elements.get("type").and_then(Value::as_str) {
                        assert!(
                            jtd_type_matches(t, item),
                            "element of {name} should be JTD type {t}, got {item}"
                        );
                    }
                    if let Some(enumeration) = elements.get("enum").and_then(Value::as_array) {
                        assert!(
                            enumeration.contains(item),
                            "element of {name} value {item} not in enum {enumeration:?}"
                        );
                    }
                }
            }
        }
    }
    if !additional {
        let declared: HashSet<&String> = properties.keys().chain(optional.keys()).collect();
        for name in obj.keys() {
            assert!(
                declared.contains(name),
                "property {name} is not declared in the JTD schema (additionalProperties: false)"
            );
        }
    }
}

fn mistral_service() -> Service {
    Service {
        id: "mistral-ai".to_string(),
        vendor: "Mistral AI".to_string(),
        endpoint: "mistral".to_string(),
        api_key_env_var: "MISTRAL_API_KEY".to_string(),
        kind: ServiceKind::Inference,
        display_name: Some("Mistral (via prompt-cult)".to_string()),
        upstream_base_url: Some("https://api.mistral.ai/v1".to_string()),
        aliases: vec!["mistral".to_string()],
    }
}

fn full_model(service: &Service) -> ModelDocument {
    let mut doc = ModelDocument::new("mistral-large-latest", service);
    doc.display_name = Some("Mistral Large".to_string());
    doc.description = Some("Flagship".to_string());
    doc.context_window = Some(128_000);
    doc.max_output_tokens = Some(8_192);
    doc.base_instructions = Some("You are a coding agent.".to_string());
    doc.supported_reasoning_efforts = vec!["low".to_string(), "high".to_string()];
    doc.input_modalities = vec![InputModality::Text, InputModality::Image];
    doc.cost_input_per_million_usd = Some(2.0);
    doc.cost_output_per_million_usd = Some(8.0);
    doc.preference = Some(ModelPreference::Pinned);
    doc.upstream_model_id = Some("mistral-large-latest".to_string());
    doc.skills = vec!["code-review".to_string()];
    doc
}

#[test]
fn service_document_conforms_to_service_jtd() {
    let service = mistral_service();
    let doc = service.document(true, vec![]);
    let value = serde_json::to_value(&doc).expect("serialize");
    assert_conforms(&value, &schema("service"));
}

#[test]
fn model_document_conforms_to_model_jtd() {
    let service = mistral_service();
    let value = serde_json::to_value(full_model(&service)).expect("serialize");
    assert_conforms(&value, &schema("model"));
}

#[test]
fn minimal_model_document_conforms_to_model_jtd() {
    let service = mistral_service();
    let value = serde_json::to_value(ModelDocument::new("m", &service)).expect("serialize");
    assert_conforms(&value, &schema("model"));
}

#[test]
fn service_list_conforms_to_composite_jtd() {
    let service = mistral_service();
    let list = ServiceList {
        services: vec![service.document(true, vec![full_model(&service)])],
    };
    let value = serde_json::to_value(&list).expect("serialize");
    let composite = schema("service-list");
    assert_conforms(&value, &composite);
    // Embedded models must conform to the composite's model definition.
    let model_schema = composite["definitions"]["model"].clone();
    assert_conforms(&value["services"][0]["models"][0], &model_schema);
}

#[test]
fn error_documents_conform_to_error_jtd() {
    let error_schema = schema("error");
    for doc in [
        ErrorDocument::model_not_enabled("glm-5-2"),
        ErrorDocument::service_disabled("mistral-ai"),
        ErrorDocument::new(ErrorCode::NotFound, "nope"),
    ] {
        let value = serde_json::to_value(&doc).expect("serialize");
        assert_conforms(&value, &error_schema);
    }
}

#[test]
fn serde_wire_values_match_jtd_enum_members() {
    // service.kind enum is exactly ["inference"].
    let service_schema = schema("service");
    let kind_enum = service_schema["properties"]["kind"]["enum"]
        .as_array()
        .expect("enum");
    assert_eq!(kind_enum, &vec![json!("inference")]);
    let doc = mistral_service().document(true, vec![]);
    assert!(kind_enum.contains(&serde_json::to_value(&doc).expect("ser")["kind"]));

    // error codes enum covers every ErrorCode variant's wire name.
    let error_schema = schema("error");
    let codes = error_schema["definitions"]["error_code"]["enum"]
        .as_array()
        .expect("enum")
        .clone();
    assert_eq!(codes.len(), 7);
    let expected = [
        "model_not_enabled",
        "service_disabled",
        "forbidden",
        "not_found",
        "bad_request",
        "upstream_error",
        "internal",
    ];
    for code in expected {
        assert!(codes.contains(&json!(code)), "enum missing {code}");
    }
}

/// The composite `service-list.jtd.json` must embed structurally identical
/// copies of the standalone `service` and `model` contracts (RFC 8927 refs
/// are same-document only; this sync test is the spec's §11 stability rule).
/// Prose (`description`/`metadata`) lives only in the standalone contracts,
/// and the composite's `service` gains one property the standalone cannot
/// declare (`models` — its `ref: model` would dangle in the standalone
/// document; the standalone's metadata points at the composite).
#[test]
fn service_list_definitions_stay_in_sync_with_standalone_contracts() {
    let composite = schema("service-list");
    let mut composite_service = composite["definitions"]["service"].clone();
    composite_service
        .as_object_mut()
        .expect("service is object")
        .get_mut("optionalProperties")
        .and_then(|p| p.as_object_mut())
        .expect("optionalProperties")
        .remove("models");
    let structural = |schema: &Value| {
        json!({
            "properties": schema.get("properties").cloned().unwrap_or_default(),
            "optionalProperties": schema.get("optionalProperties").cloned().unwrap_or_default(),
            "additionalProperties": schema.get("additionalProperties").cloned().unwrap_or_default(),
        })
    };
    assert_eq!(
        structural(&composite_service),
        structural(&schema("service")),
        "service definition drifted from schemas/service.jtd.json"
    );
    assert_eq!(
        structural(&composite["definitions"]["model"]),
        structural(&schema("model")),
        "model definition drifted from schemas/model.jtd.json"
    );
}
