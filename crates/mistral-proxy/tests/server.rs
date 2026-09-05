//! Hermetic end-to-end tests for the reference proxy: a scripted mock
//! Mistral upstream and the real proxy server, both on loopback ephemeral
//! ports. No real network egress, no real credentials.

use std::io::Read;
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use pretty_assertions::assert_eq;
use prompt_cult_mistral_proxy::MISTRAL_DEFAULTS;
use prompt_cult_mistral_proxy::MistralProxy;
use prompt_cult_mistral_proxy::SERVICE_ID;
use prompt_cult_mistral_proxy::ShutdownPolicy;
use prompt_cult_mistral_proxy::serve;
use prompt_cult_mistral_proxy::service;
use prompt_cult_proxy_core::config::JsoncDiskConfigStore;
use prompt_cult_proxy_core::config::load_config;
use prompt_cult_proxy_core::secrets;
use serde_json::Value;
use serde_json::json;

/// A request the mock upstream received.
#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    body: String,
}

/// One scripted upstream response.
struct MockResponse {
    status: u16,
    content_type: &'static str,
    body: String,
}

fn json_response(status: u16, body: String) -> MockResponse {
    MockResponse {
        status,
        content_type: "application/json",
        body,
    }
}

fn sse_response(body: String) -> MockResponse {
    MockResponse {
        status: 200,
        content_type: "text/event-stream",
        body,
    }
}

/// A mock Mistral upstream serving a fixed script, in order. After the last
/// scripted response is served the listener closes; tests must therefore
/// predict the exact number of upstream requests they cause.
struct MockUpstream {
    port: u16,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl MockUpstream {
    fn start(responses: Vec<MockResponse>) -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind mock upstream");
        let port = server
            .server_addr()
            .to_ip()
            .expect("loopback address")
            .port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        std::thread::spawn(move || {
            for response in responses {
                let Ok(mut req) = server.recv() else {
                    return;
                };
                let method = req.method().as_str().to_string();
                let url = req.url().to_string();
                let authorization = req
                    .headers()
                    .iter()
                    .find(|h| {
                        h.field
                            .as_str()
                            .as_str()
                            .eq_ignore_ascii_case("authorization")
                    })
                    .map(|h| String::from_utf8_lossy(h.value.as_bytes()).to_string());
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                recorded
                    .lock()
                    .expect("recorded requests lock")
                    .push(RecordedRequest {
                        method,
                        path: url,
                        authorization,
                        body,
                    });
                let reply = tiny_http::Response::from_string(response.body)
                    .with_status_code(tiny_http::StatusCode(response.status))
                    .with_header(
                        tiny_http::Header::from_bytes(
                            b"content-type",
                            response.content_type.as_bytes(),
                        )
                        .expect("content-type header"),
                    );
                let _ = req.respond(reply);
            }
        });
        Self { port, requests }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn recorded(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .expect("recorded requests lock")
            .clone()
    }
}

/// The Mistral catalog the mock serves: two chat models plus one non-chat
/// embedding model and one chat model excluded by the default globs.
fn models_catalog() -> String {
    r#"{"object":"list","data":[
        {"id":"zai-glm-5-2","capabilities":{"completion_chat":true},"max_context_length":131072},
        {"id":"mistral-medium-latest","capabilities":{"completion_chat":true}},
        {"id":"mistral-embed","capabilities":{"completion_chat":false},"max_context_length":8192},
        {"id":"ministral-3b-2512","capabilities":{"completion_chat":true}}
    ]}"#
    .to_string()
}

fn chat_completion() -> String {
    r#"{
        "id": "chatcmpl-1",
        "model": "zai-glm-5-2",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "Hello back!"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    }"#
    .to_string()
}

fn chat_sse() -> String {
    let mut body = String::new();
    body.push_str("data: {\"model\":\"zai-glm-5-2\",\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Hi\"}}]}\n\n");
    body.push_str("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n");
    body.push_str("data: [DONE]\n\n");
    body
}

/// A port that refuses connections (bind, read the port, drop the listener).
fn dead_upstream_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

/// Start the real proxy server on a loopback ephemeral port, configured from
/// `config_dir`, forwarding to `upstream_base`. Returns the proxy's port.
fn start_proxy(config_dir: &Path, upstream_base: String) -> u16 {
    let auth_header = secrets::auth_header_from_key("testkey123").expect("auth header");
    let config = load_config(
        &JsoncDiskConfigStore::new(config_dir),
        SERVICE_ID,
        &MISTRAL_DEFAULTS,
    )
    .expect("config");
    let proxy =
        Arc::new(MistralProxy::new(auth_header, config, upstream_base, service()).expect("proxy"));
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind proxy");
    let port = listener.local_addr().expect("proxy local addr").port();
    let server = tiny_http::Server::from_listener(listener, None).expect("proxy server");
    std::thread::spawn(move || {
        let _ = serve(server, proxy, ShutdownPolicy::PostOnly);
    });
    port
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("test client")
}

fn proxy_url(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{port}{path}")
}

fn write_config(dir: &Path, contents: &str) {
    std::fs::write(dir.join("mistral-ai.jsonc"), contents).expect("write config");
}

fn error_code(body: &str) -> String {
    serde_json::from_str::<Value>(body).expect("error body is JSON")["error"]["code"]
        .as_str()
        .expect("error code")
        .to_string()
}

#[test]
fn health_reports_proxy_id_and_upstream() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port = start_proxy(
        dir.path(),
        format!("http://127.0.0.1:{}", dead_upstream_port()),
    );
    let resp = client()
        .get(proxy_url(port, "/health"))
        .send()
        .expect("health request");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().expect("health body");
    assert_eq!(body["status"], json!("ok"));
    assert_eq!(body["proxy"], json!("mistral-ai"));
    assert!(
        body["upstream"]
            .as_str()
            .is_some_and(|u| u.starts_with("http://"))
    );
}

#[test]
fn service_lists_inference_service_with_model_documents() {
    let upstream = MockUpstream::start(vec![json_response(200, models_catalog())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let port = start_proxy(dir.path(), upstream.base_url());

    let resp = client()
        .get(proxy_url(port, "/service"))
        .send()
        .expect("service request");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().expect("service body");

    let services = body["services"].as_array().expect("services");
    assert_eq!(services.len(), 1);
    let svc = &services[0];
    assert_eq!(svc["id"], json!("mistral-ai"));
    assert_eq!(svc["kind"], json!("inference"));
    assert_eq!(svc["vendor"], json!("Mistral AI"));
    assert_eq!(svc["endpoint"], json!("mistral"));
    assert_eq!(svc["api_key_env_var"], json!("MISTRAL_API_KEY"));
    assert_eq!(svc["enabled"], json!(true));

    let models = svc["models"].as_array().expect("models");
    let ids: Vec<&str> = models
        .iter()
        .map(|m| m["id"].as_str().expect("model id"))
        .collect();
    // Chat-capable models only; the default exclusion globs drop ministral.
    assert_eq!(ids, vec!["zai-glm-5-2", "mistral-medium-latest"]);
    let glm = &models[0];
    assert_eq!(glm["service_id"], json!("mistral-ai"));
    assert_eq!(glm["api_key_env_var"], json!("MISTRAL_API_KEY"));
    assert_eq!(glm["enabled"], json!(true));
    assert_eq!(glm["context_window"], json!(131_072));
    assert!(
        glm["base_instructions"]
            .as_str()
            .is_some_and(|s| s.contains("Prompt Cult"))
    );
}

#[test]
fn models_returns_codex_shape_and_ignores_query_string() {
    let upstream = MockUpstream::start(vec![json_response(200, models_catalog())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let port = start_proxy(dir.path(), upstream.base_url());

    let resp = client()
        .get(proxy_url(port, "/v1/models?client_version=42"))
        .send()
        .expect("models request");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().expect("models body");
    let models = body["models"].as_array().expect("models");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["slug"], json!("zai-glm-5-2"));
    assert_eq!(models[0]["shell_type"], json!("shell_command"));
    assert_eq!(models[0]["visibility"], json!("list"));
    assert_eq!(models[0]["context_window"], json!(131_072));
    // The mock upstream saw the key-bearing models fetch.
    let recorded = upstream.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].path, "/models");
    assert_eq!(
        recorded[0].authorization.as_deref(),
        Some("Bearer testkey123")
    );
}

#[test]
fn relay_refuses_model_not_in_catalog() {
    let upstream = MockUpstream::start(vec![json_response(200, models_catalog())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let port = start_proxy(dir.path(), upstream.base_url());

    let resp = client()
        .post(proxy_url(port, "/v1/responses"))
        .json(&json!({"model": "made-up-model", "input": []}))
        .send()
        .expect("relay request");
    assert_eq!(resp.status(), 403);
    let body: Value = resp.json().expect("error body");
    assert_eq!(body["error"]["code"], json!("model_not_enabled"));
    assert_eq!(body["error"]["model"], json!("made-up-model"));
    // Only the catalog fetch happened; no chat traffic was attempted.
    let recorded = upstream.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].path, "/models");
}

#[test]
fn relay_refuses_excluded_model() {
    let upstream = MockUpstream::start(vec![json_response(200, models_catalog())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let port = start_proxy(dir.path(), upstream.base_url());

    let resp = client()
        .post(proxy_url(port, "/v1/responses"))
        .json(&json!({"model": "ministral-3b-2512", "input": []}))
        .send()
        .expect("relay request");
    assert_eq!(resp.status(), 403);
    let body: Value = resp.json().expect("error body");
    assert_eq!(body["error"]["code"], json!("model_not_enabled"));
    assert_eq!(body["error"]["model"], json!("ministral-3b-2512"));
}

#[test]
fn relay_refuses_model_disabled_by_config() {
    let upstream = MockUpstream::start(vec![
        json_response(200, models_catalog()),
        json_response(200, models_catalog()),
    ]);
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(
        dir.path(),
        r#"{ "model_overrides": { "zai-glm-5-2": { "enabled": false } } }"#,
    );
    let port = start_proxy(dir.path(), upstream.base_url());

    let resp = client()
        .post(proxy_url(port, "/v1/responses"))
        .json(&json!({"model": "zai-glm-5-2", "input": []}))
        .send()
        .expect("relay request");
    assert_eq!(resp.status(), 403);
    let body: Value = resp.json().expect("error body");
    assert_eq!(body["error"]["code"], json!("model_not_enabled"));

    // The lockdown also hides the model from discovery.
    let models: Value = client()
        .get(proxy_url(port, "/v1/models"))
        .send()
        .expect("models request")
        .json()
        .expect("models body");
    let slugs: Vec<&str> = models["models"]
        .as_array()
        .expect("models")
        .iter()
        .map(|m| m["slug"].as_str().expect("slug"))
        .collect();
    assert_eq!(slugs, vec!["mistral-medium-latest"]);
}

#[test]
fn relay_non_streaming_translates_and_injects_key() {
    let upstream = MockUpstream::start(vec![
        json_response(200, models_catalog()),
        json_response(200, chat_completion()),
    ]);
    let dir = tempfile::tempdir().expect("tempdir");
    let port = start_proxy(dir.path(), upstream.base_url());

    let resp = client()
        .post(proxy_url(port, "/v1/responses"))
        .header("authorization", "Bearer harness-should-not-pass")
        .json(&json!({
            "model": "zai-glm-5-2",
            "instructions": "You are helpful.",
            "input": [{"type": "message", "role": "user", "content": "Hello"}],
            "max_output_tokens": 4096,
        }))
        .send()
        .expect("relay request");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().expect("response body");
    assert_eq!(body["object"], json!("response"));
    assert_eq!(body["status"], json!("completed"));
    // The client-visible model is the requested one, verbatim.
    assert_eq!(body["model"], json!("zai-glm-5-2"));
    assert_eq!(body["output"][0]["type"], json!("message"));
    assert_eq!(
        body["output"][0]["content"][0]["text"],
        json!("Hello back!")
    );
    assert_eq!(body["usage"]["input_tokens"], json!(10));
    assert_eq!(body["usage"]["output_tokens"], json!(5));

    // The upstream saw exactly the catalog gate plus the chat call.
    let recorded = upstream.recorded();
    assert_eq!(recorded.len(), 2);
    let chat = &recorded[1];
    assert_eq!(chat.method, "POST");
    assert_eq!(chat.path, "/chat/completions");
    // The proxy's key replaced the caller's credential header.
    assert_eq!(chat.authorization.as_deref(), Some("Bearer testkey123"));
    let sent: Value = serde_json::from_str(&chat.body).expect("upstream body is JSON");
    assert_eq!(sent["model"], json!("zai-glm-5-2"));
    assert_eq!(sent["max_tokens"], json!(4096));
    assert_eq!(sent["stream"], json!(false));
    assert_eq!(sent["messages"][0]["role"], json!("system"));
    assert_eq!(sent["messages"][0]["content"], json!("You are helpful."));
    assert_eq!(sent["messages"][1]["role"], json!("user"));
    assert_eq!(sent["messages"][1]["content"], json!("Hello"));
}

#[test]
fn relay_applies_explicit_upstream_model_mapping() {
    let upstream = MockUpstream::start(vec![
        json_response(200, models_catalog()),
        json_response(200, chat_completion()),
    ]);
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(
        dir.path(),
        r#"{ "model_overrides": { "zai-glm-5-2": { "upstream_model_id": "glm-5-2-alias" } } }"#,
    );
    let port = start_proxy(dir.path(), upstream.base_url());

    let resp = client()
        .post(proxy_url(port, "/v1/responses"))
        .json(&json!({
            "model": "zai-glm-5-2",
            "input": [{"type": "message", "role": "user", "content": "Hello"}],
        }))
        .send()
        .expect("relay request");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().expect("response body");
    // Client-visible identity stays the requested model, never the alias.
    assert_eq!(body["model"], json!("zai-glm-5-2"));

    let recorded = upstream.recorded();
    let chat = &recorded[1];
    let sent: Value = serde_json::from_str(&chat.body).expect("upstream body is JSON");
    assert_eq!(sent["model"], json!("glm-5-2-alias"));
}

#[test]
fn relay_streaming_translates_sse() {
    let upstream = MockUpstream::start(vec![
        json_response(200, models_catalog()),
        sse_response(chat_sse()),
    ]);
    let dir = tempfile::tempdir().expect("tempdir");
    let port = start_proxy(dir.path(), upstream.base_url());

    let mut resp = client()
        .post(proxy_url(port, "/v1/responses"))
        .json(&json!({
            "model": "zai-glm-5-2",
            "stream": true,
            "input": [{"type": "message", "role": "user", "content": "Hello"}],
        }))
        .send()
        .expect("relay request");
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .expect("content-type")
            .to_str()
            .expect("utf-8"),
        "text/event-stream"
    );
    let mut body = String::new();
    resp.read_to_string(&mut body).expect("read stream");
    assert!(body.contains("event: response.created"));
    assert!(body.contains("event: response.in_progress"));
    assert!(body.contains("event: response.output_text.delta"));
    assert!(body.contains("\"delta\":\"Hi\""));
    assert!(body.contains("event: response.completed"));
    assert!(body.contains("\"model\":\"zai-glm-5-2\""));
}

#[test]
fn upstream_transport_error_is_502() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port = start_proxy(
        dir.path(),
        format!("http://127.0.0.1:{}", dead_upstream_port()),
    );

    let resp = client()
        .post(proxy_url(port, "/v1/responses"))
        .json(&json!({"model": "zai-glm-5-2", "input": []}))
        .send()
        .expect("relay request");
    assert_eq!(resp.status(), 502);
    assert_eq!(error_code(&resp.text().expect("body")), "upstream_error");
}

#[test]
fn disabled_service_hides_discovery_and_refuses_traffic() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(dir.path(), r#"{ "enabled": false }"#);
    let port = start_proxy(
        dir.path(),
        format!("http://127.0.0.1:{}", dead_upstream_port()),
    );

    // No upstream traffic happens at all.
    let relay = client()
        .post(proxy_url(port, "/v1/responses"))
        .json(&json!({"model": "zai-glm-5-2", "input": []}))
        .send()
        .expect("relay request");
    assert_eq!(relay.status(), 403);
    assert_eq!(error_code(&relay.text().expect("body")), "service_disabled");

    let services: Value = client()
        .get(proxy_url(port, "/service"))
        .send()
        .expect("service request")
        .json()
        .expect("service body");
    assert_eq!(services["services"], json!([]));

    let models: Value = client()
        .get(proxy_url(port, "/v1/models"))
        .send()
        .expect("models request")
        .json()
        .expect("models body");
    assert_eq!(models["models"], json!([]));

    // Liveness still answers: a misconfigured sidecar must be diagnosable.
    let health = client()
        .get(proxy_url(port, "/health"))
        .send()
        .expect("health request");
    assert_eq!(health.status(), 200);
}

#[test]
fn unknown_route_is_403_forbidden() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port = start_proxy(
        dir.path(),
        format!("http://127.0.0.1:{}", dead_upstream_port()),
    );

    let resp = client()
        .get(proxy_url(port, "/nope"))
        .send()
        .expect("request");
    assert_eq!(resp.status(), 403);
    assert_eq!(error_code(&resp.text().expect("body")), "forbidden");

    let resp = client()
        .post(proxy_url(port, "/v1/models"))
        .json(&json!({}))
        .send()
        .expect("request");
    assert_eq!(resp.status(), 403);
}

#[test]
fn malformed_relay_body_is_400() {
    let upstream = MockUpstream::start(vec![json_response(200, models_catalog())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let port = start_proxy(dir.path(), upstream.base_url());

    let resp = client()
        .post(proxy_url(port, "/v1/responses"))
        .header("content-type", "application/json")
        .body("this is not json")
        .send()
        .expect("relay request");
    assert_eq!(resp.status(), 400);
    assert_eq!(error_code(&resp.text().expect("body")), "bad_request");
}

#[test]
fn upstream_non_200_on_models_is_relayed_verbatim() {
    let upstream = MockUpstream::start(vec![json_response(
        503,
        r#"{"error":"upstream exploded"}"#.to_string(),
    )]);
    let dir = tempfile::tempdir().expect("tempdir");
    let port = start_proxy(dir.path(), upstream.base_url());

    let resp = client()
        .get(proxy_url(port, "/v1/models"))
        .send()
        .expect("models request");
    assert_eq!(resp.status(), 503);
    let body = resp.text().expect("body");
    assert_eq!(body, r#"{"error":"upstream exploded"}"#);
}

#[test]
fn model_overrides_reach_both_discovery_shapes() {
    let upstream = MockUpstream::start(vec![
        json_response(200, models_catalog()),
        json_response(200, models_catalog()),
    ]);
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(
        dir.path(),
        r#"{
            "model_overrides": {
                "zai-glm-5-2": {
                    "base_instructions": "You are the tuned GLM.",
                    "context_window": 200000
                }
            }
        }"#,
    );
    let port = start_proxy(dir.path(), upstream.base_url());

    let services: Value = client()
        .get(proxy_url(port, "/service"))
        .send()
        .expect("service request")
        .json()
        .expect("service body");
    let glm = &services["services"][0]["models"][0];
    assert_eq!(glm["id"], json!("zai-glm-5-2"));
    assert_eq!(glm["context_window"], json!(200_000));
    assert_eq!(glm["base_instructions"], json!("You are the tuned GLM."));

    let models: Value = client()
        .get(proxy_url(port, "/v1/models"))
        .send()
        .expect("models request")
        .json()
        .expect("models body");
    assert_eq!(models["models"][0]["slug"], json!("zai-glm-5-2"));
    assert_eq!(models["models"][0]["context_window"], json!(200_000));
    assert_eq!(
        models["models"][0]["base_instructions"],
        json!("You are the tuned GLM.")
    );
}
