//! `prompt-cult-mistral-proxy` — the reference Prompt Cult proxy: a
//! translating relay for the Mistral AI inference service.
//!
//! Accepts OAI Responses API requests from a network-isolated harness and
//! routes them to the Mistral Chat Completions API (`POST
//! {upstream}/chat/completions`), translating the upstream SSE stream back to
//! OAI Responses SSE. Serves the endpoint surface of `docs/proxy-spec.md` §6:
//! `GET /service`, `GET /v1/models` (codex picker shape), `POST
//! /v1/responses`, `GET /health`, and `POST /shutdown` (loopback only).
//!
//! Security model (inherited from `prompt-cult-proxy-core`, upstream lineage
//! openai/codex#4778): the API key is read from `MISTRAL_API_KEY` (stdin
//! fallback), held as a single mlock(2)-protected copy, and injected into
//! upstream requests — never logged, never written to disk, never echoed to
//! the harness. The harness holds no credentials and no provider
//! configuration; it discovers services and models from this proxy. Locked
//! models are absent from discovery AND refused at request time, so a
//! harness cannot escalate to a non-approved model.

use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use clap::Parser;
use prompt_cult_proxy_core::config::JsoncDiskConfigStore;
use prompt_cult_proxy_core::config::LogLevel;
use prompt_cult_proxy_core::config::ResolvedConfig;
use prompt_cult_proxy_core::config::ServiceDefaults;
use prompt_cult_proxy_core::config::config_filename;
use prompt_cult_proxy_core::config::load_config;
use prompt_cult_proxy_core::config::resolve_upstream_base;
use prompt_cult_proxy_core::error::ErrorCode;
use prompt_cult_proxy_core::error::ErrorDocument;
use prompt_cult_proxy_core::secrets;
use prompt_cult_proxy_core::service::Service;
use prompt_cult_proxy_core::service::ServiceKind;
use prompt_cult_proxy_core::service::ServiceList;
use reqwest::Url;
use reqwest::blocking::Client;
use reqwest::header::AUTHORIZATION;
use reqwest::header::HOST;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderName;
use reqwest::header::HeaderValue;
use serde::Serialize;
use tiny_http::Header;
use tiny_http::Method;
use tiny_http::Request;
use tiny_http::Response;
use tiny_http::Server;
use tiny_http::StatusCode;

mod models_translate;
mod translate_request;
mod translate_sse;

/// This proxy's service identity (spec §2-§3): vendor Mistral AI, endpoint
/// `mistral`, key from `MISTRAL_API_KEY`.
pub const SERVICE_ID: &str = "mistral-ai";
pub const API_KEY_ENV_VAR: &str = "MISTRAL_API_KEY";
const BIN_NAME: &str = "prompt-cult-mistral-proxy";

const DEFAULT_UPSTREAM_BASE: &str = "https://api.mistral.ai/v1";

/// Compiled-in defaults used when no `mistral-ai.jsonc` exists. The exclusion
/// list drops non-chat-purpose families from discovery; the bare alias
/// `glm-5-2` is exact-matched so `zai-glm-5-2` still passes.
pub const MISTRAL_DEFAULTS: ServiceDefaults = ServiceDefaults {
    upstream_base_url: DEFAULT_UPSTREAM_BASE,
    model_exclude_globs: &[
        "*-ocr-*",
        "*-mini-*",
        "magistral-*",
        "ministral-*",
        "voxtral-*",
        "glm-5-2",
    ],
};

/// The static service description this proxy fronts.
pub fn service() -> Service {
    Service {
        id: SERVICE_ID.to_string(),
        vendor: "Mistral AI".to_string(),
        endpoint: "mistral".to_string(),
        api_key_env_var: API_KEY_ENV_VAR.to_string(),
        kind: ServiceKind::Inference,
        display_name: Some("Mistral AI".to_string()),
        upstream_base_url: Some(DEFAULT_UPSTREAM_BASE.to_string()),
        aliases: Vec::new(),
    }
}

/// Classification of an incoming request path, ignoring any query string.
///
/// Codex always appends `?client_version=X` to `/v1/models`, so route matching
/// must compare the path only. Matching the raw URL (path + query) is the bug
/// that made model discovery 403 (see `docs/appendix-proxy-bugs.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteKind {
    ServiceList,
    Models,
    Responses,
    Shutdown,
    Health,
    Forbidden,
}

/// Map a raw request URL (path plus optional query) to a [`RouteKind`].
fn classify_route(raw_url: &str) -> RouteKind {
    let path = raw_url.split('?').next().unwrap_or(raw_url);
    match path {
        "/service" => RouteKind::ServiceList,
        "/v1/models" | "/models" => RouteKind::Models,
        "/v1/responses" => RouteKind::Responses,
        "/shutdown" => RouteKind::Shutdown,
        "/health" => RouteKind::Health,
        _ => RouteKind::Forbidden,
    }
}

/// Which HTTP methods may trigger `/shutdown` (spec §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownPolicy {
    /// Default: POST only.
    PostOnly,
    /// With `--http-shutdown`: POST or GET.
    PostOrGet,
}

impl ShutdownPolicy {
    fn allows(self, method: &Method) -> bool {
        match self {
            Self::PostOnly => *method == Method::Post,
            Self::PostOrGet => *method == Method::Post || *method == Method::Get,
        }
    }
}

/// CLI arguments for the Mistral translating proxy.
#[derive(Debug, Clone, Parser)]
#[command(
    name = BIN_NAME,
    about = "Translating proxy: OAI Responses API ↔ Mistral Chat Completions"
)]
pub struct Args {
    /// Port to listen on. If not set, an ephemeral port is used.
    #[arg(long)]
    pub port: Option<u16>,

    /// Path to a JSON file to write startup info (single line). Includes {"port": <u16>}.
    #[arg(long, value_name = "FILE")]
    pub server_info: Option<PathBuf>,

    /// Also accept GET /shutdown (POST /shutdown is always enabled).
    #[arg(long)]
    pub http_shutdown: bool,

    /// Base URL of the Mistral API. Overrides `upstream_base_url` in
    /// `mistral-ai.jsonc`; default: https://api.mistral.ai/v1.
    /// Chat requests go to `{base}/chat/completions`, model listings to `{base}/models`.
    #[arg(long)]
    pub upstream_base: Option<String>,

    /// Proxy config directory. Overrides `PC_PROXY_CONFIG_DIR` and
    /// `~/.prompt-cult`.
    #[arg(long, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,
}

#[derive(Serialize)]
struct ServerInfo {
    port: u16,
    pid: u32,
}

/// The outcome of handling one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Shutdown,
}

/// Shared proxy state: every request-handling thread sees one instance.
pub struct MistralProxy {
    client: Client,
    auth_header: &'static str,
    service: Service,
    config: ResolvedConfig,
    upstream_base: String,
    host_header: HeaderValue,
    /// Cache of model IDs provably present in the upstream catalog,
    /// post-filtering — the spec §5 request-time gate set. Refreshed on
    /// first relay and on miss so newly released models pass without a
    /// proxy restart.
    allowed_models: Mutex<Option<HashSet<String>>>,
    /// Per-process request counter for verbose routing logs.
    request_counter: AtomicU64,
}

impl MistralProxy {
    /// Build the proxy state around an already-read auth header and a
    /// resolved config.
    ///
    /// A proxy that exists has its key, so the "key present" half of service
    /// enablement (spec §3) is structurally settled here; the operator
    /// `enabled: false` kill-switch in `config` remains the runtime gate.
    pub fn new(
        auth_header: &'static str,
        config: ResolvedConfig,
        upstream_base: String,
        service: Service,
    ) -> Result<Self> {
        let parsed = Url::parse(&upstream_base).context("parsing upstream base URL")?;
        let host = match (parsed.host_str(), parsed.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_string(),
            _ => return Err(anyhow!("upstream base URL must include a host")),
        };
        let host_header =
            HeaderValue::from_str(&host).context("constructing Host header from upstream URL")?;
        Ok(Self {
            // No default timeout: a silent 30s cap kills slow generations
            // (upstream #4336, see docs/appendix-proxy-bugs.md).
            client: Client::builder()
                .timeout(None::<Duration>)
                .build()
                .context("building reqwest client")?,
            auth_header,
            service,
            config,
            upstream_base,
            host_header,
            allowed_models: Mutex::new(None),
            request_counter: AtomicU64::new(0),
        })
    }

    /// Whether the service answers discovery and relays traffic (spec §5):
    /// the operator `enabled: false` kill-switch hides everything.
    fn service_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Dispatch one request to its handler. Every path responds; internal
    /// errors are logged and answered with structured error bodies, never
    /// key material or request/response bodies.
    pub(crate) fn handle(&self, req: Request, shutdown_policy: ShutdownPolicy) -> Flow {
        let method = req.method().clone();
        let url = req.url().to_string();
        let route = classify_route(&url);

        eprintln!("{SERVICE_ID}: {method} {url} -> {route:?}");

        if method == Method::Get && route == RouteKind::Health {
            self.respond_health(req);
            return Flow::Continue;
        }

        if route == RouteKind::Shutdown && shutdown_policy.allows(&method) {
            self.respond_empty(req, 200);
            return Flow::Shutdown;
        }

        if method == Method::Get && route == RouteKind::ServiceList {
            self.handle_service_list(req);
            return Flow::Continue;
        }

        if method == Method::Get && route == RouteKind::Models {
            self.handle_models(req);
            return Flow::Continue;
        }

        if method == Method::Post && route == RouteKind::Responses {
            self.handle_responses(req);
            return Flow::Continue;
        }

        eprintln!("{SERVICE_ID}: 403 forbidden for {method} {url}");
        self.respond_error(
            req,
            ErrorDocument::new(ErrorCode::Forbidden, "not a proxy route"),
        );
        Flow::Continue
    }

    /// GET /service: the prompt-cult discovery document (spec §6) — the
    /// service with its full model documents. A disabled service lists no
    /// services rather than leaking its catalog.
    fn handle_service_list(&self, req: Request) {
        if !self.service_enabled() {
            self.respond_json(
                req,
                200,
                &ServiceList {
                    services: Vec::new(),
                },
            );
            return;
        }
        match self.fetch_catalog() {
            Ok(CatalogFetch::Ok(catalog)) => {
                let doc = self.service.document(true, catalog.documents);
                self.respond_json(
                    req,
                    200,
                    &ServiceList {
                        services: vec![doc],
                    },
                );
            }
            // Our own surface: relay nothing that is not a service list.
            Ok(CatalogFetch::NonOk(upstream)) => {
                eprintln!(
                    "{SERVICE_ID}: upstream models query returned {}",
                    upstream.status()
                );
                self.respond_error(req, upstream_error());
            }
            Err(e) => self.respond_error(req, upstream_error_with(e)),
        }
    }

    /// GET /v1/models: fetch `{upstream}/models` and translate Mistral's raw
    /// list into the codex `ModelsResponse` shape so pickers can decode it
    /// strictly. On an upstream non-200 the original error response is
    /// relayed verbatim so the harness can log the real cause.
    fn handle_models(&self, req: Request) {
        if !self.service_enabled() {
            self.respond_json(
                req,
                200,
                &prompt_cult_proxy_core::discovery::ModelsResponse { models: Vec::new() },
            );
            return;
        }
        match self.fetch_catalog() {
            Ok(CatalogFetch::Ok(catalog)) => {
                self.respond_json(req, 200, &catalog.response);
            }
            Ok(CatalogFetch::NonOk(upstream)) => {
                if let Err(e) = relay_response(req, upstream) {
                    eprintln!("{SERVICE_ID}: failed to relay response: {e}");
                }
            }
            Err(e) => self.respond_error(req, upstream_error_with(e)),
        }
    }

    /// POST /v1/responses: translate to Mistral chat/completions and back.
    fn handle_responses(&self, mut req: Request) {
        if !self.service_enabled() {
            self.respond_error(req, ErrorDocument::service_disabled(SERVICE_ID));
            return;
        }

        let mut body_bytes = Vec::new();
        if let Err(e) = req.as_reader().read_to_end(&mut body_bytes) {
            eprintln!("{SERVICE_ID}: failed to read request body: {e}");
            self.respond_error(
                req,
                ErrorDocument::new(ErrorCode::BadRequest, "could not read request body"),
            );
            return;
        }
        let body: serde_json::Value = match serde_json::from_slice(&body_bytes) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{SERVICE_ID}: request body is not JSON: {e}");
                self.respond_error(
                    req,
                    ErrorDocument::new(ErrorCode::BadRequest, "request body is not JSON"),
                );
                return;
            }
        };
        let Some(model) = body["model"].as_str().map(str::to_string) else {
            self.respond_error(
                req,
                ErrorDocument::new(ErrorCode::BadRequest, "request body must name a model"),
            );
            return;
        };
        let is_stream = body["stream"].as_bool().unwrap_or(false);

        let verbose = self.config.log_level == LogLevel::Verbose;
        let req_id = self.request_counter.fetch_add(1, Ordering::Relaxed) + 1;
        if verbose {
            eprintln!("{SERVICE_ID}: req#{req_id} model={model} stream={is_stream}");
        }

        // Spec §5 request-time lockdown: the model must be present in the
        // current upstream catalog and allowed by config. Refuse before any
        // upstream traffic so a harness cannot escalate.
        match self.model_allowed(&model) {
            Ok(false) => {
                self.respond_error(req, ErrorDocument::model_not_enabled(&model));
                return;
            }
            Ok(true) => {}
            Err(e) => {
                self.respond_error(req, upstream_error_with(e));
                return;
            }
        }

        let mistral_body = translate_request::oai_to_mistral(&body);
        let upstream_url = format!("{}/chat/completions", self.upstream_base);
        let fwd_headers = build_upstream_headers(self.auth_header, &self.host_header, &req);

        let upstream_resp = match self
            .client
            .post(&upstream_url)
            .headers(fwd_headers)
            .json(&mistral_body)
            .send()
        {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("{SERVICE_ID}: forwarding chat request to upstream failed: {e}");
                self.respond_error(req, upstream_error());
                return;
            }
        };

        if upstream_resp.status().as_u16() != 200 {
            if let Err(e) = relay_response(req, upstream_resp) {
                eprintln!("{SERVICE_ID}: failed to relay response: {e}");
            }
            return;
        }

        if !is_stream {
            let mistral_body: serde_json::Value = match upstream_resp.json() {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("{SERVICE_ID}: reading Mistral response failed: {e}");
                    self.respond_error(req, upstream_error());
                    return;
                }
            };
            if verbose {
                let upstream_model = mistral_body["model"].as_str().unwrap_or("");
                eprintln!("{SERVICE_ID}: req#{req_id} upstream_model={upstream_model}");
            }
            let oai_resp = translate_request::mistral_response_to_oai(&mistral_body, &model);
            self.respond_json(req, 200, &oai_resp);
            return;
        }

        // Streaming: translate Mistral Chat SSE → OAI Responses SSE.
        let verbose_req = verbose.then_some(req_id);
        let translator =
            translate_sse::MistralToOaiStream::new(model, Box::new(upstream_resp), verbose_req);
        let resp = Response::new(
            StatusCode(200),
            vec![
                Header::from_bytes(b"content-type", b"text/event-stream")
                    .unwrap_or_else(|_| unreachable!()),
                Header::from_bytes(b"cache-control", b"no-cache")
                    .unwrap_or_else(|_| unreachable!()),
                Header::from_bytes(b"x-accel-buffering", b"no").unwrap_or_else(|_| unreachable!()),
            ],
            translator,
            None,
            None,
        );
        if let Err(e) = req.respond(resp) {
            eprintln!("{SERVICE_ID}: failed to respond stream: {e}");
        }
    }

    /// GET /health: liveness — proxy id and upstream base URL.
    fn respond_health(&self, req: Request) {
        let body = serde_json::json!({
            "status": "ok",
            "proxy": SERVICE_ID,
            "upstream": self.upstream_base,
        });
        self.respond_json(req, 200, &body);
    }

    /// Fetch and translate the upstream model catalog. Discovery stays
    /// dynamic (spec §6): the upstream catalog is queried on every call.
    fn fetch_catalog(&self) -> Result<CatalogFetch> {
        let upstream_url = format!("{}/models", self.upstream_base);
        eprintln!("{SERVICE_ID}: fetching upstream {upstream_url}");

        let mut headers = HeaderMap::new();
        let mut auth_value = HeaderValue::from_static(self.auth_header);
        auth_value.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth_value);
        headers.insert(HOST, self.host_header.clone());

        let upstream_resp = self
            .client
            .get(&upstream_url)
            .headers(headers)
            .send()
            .context("forwarding models request to upstream")?;

        eprintln!(
            "{SERVICE_ID}: upstream responded {}",
            upstream_resp.status()
        );

        if upstream_resp.status().as_u16() != 200 {
            return Ok(CatalogFetch::NonOk(upstream_resp));
        }

        let raw = upstream_resp
            .bytes()
            .context("reading Mistral models response")?;
        let translated = models_translate::translate_mistral_models(
            &raw,
            &self.service,
            &self.config.exclude,
            &self.config.model_overrides,
        )?;
        let kept = translated.documents.len();
        eprintln!(
            "{SERVICE_ID}: loaded {} models, {} after exclusions",
            translated.chat_loaded, kept
        );
        Ok(CatalogFetch::Ok(translated))
    }

    /// Spec §5 request-time gate: the model must be present in the current
    /// upstream catalog (post-filtering) AND allowed by config.
    ///
    /// The cached ID set is consulted first; on miss (or first use) the
    /// catalog is refreshed once and the check retried, so newly released
    /// models are accepted without a proxy restart while stale or invented
    /// IDs are refused.
    fn model_allowed(&self, model: &str) -> Result<bool> {
        let mut cache = self.allowed_models.lock().expect("discovery cache lock");
        if cache.as_ref().is_some_and(|set| set.contains(model)) && self.config.model_allowed(model)
        {
            return Ok(true);
        }
        match self.fetch_catalog()? {
            CatalogFetch::Ok(catalog) => {
                let set = catalog.allowed_ids();
                let allowed = set.contains(model) && self.config.model_allowed(model);
                *cache = Some(set);
                Ok(allowed)
            }
            CatalogFetch::NonOk(upstream) => {
                let status = upstream.status();
                Err(anyhow!("upstream models query returned {status}"))
            }
        }
    }

    /// Respond with a JSON body.
    fn respond_json<T: Serialize>(&self, req: Request, status: u16, body: &T) {
        let data = serde_json::to_vec(body).unwrap_or_default();
        let resp = Response::from_data(data)
            .with_status_code(StatusCode(status))
            .with_header(
                Header::from_bytes(b"content-type", b"application/json")
                    .unwrap_or_else(|_| unreachable!()),
            );
        if let Err(e) = req.respond(resp) {
            eprintln!("{SERVICE_ID}: failed to respond: {e}");
        }
    }

    /// Respond with a structured error body (spec §5; never key material).
    fn respond_error(&self, req: Request, doc: ErrorDocument) {
        self.respond_json(req, doc.error.code.http_status(), &doc);
    }

    /// Respond with an empty body.
    fn respond_empty(&self, req: Request, status: u16) {
        if let Err(e) = req.respond(Response::new_empty(StatusCode(status))) {
            eprintln!("{SERVICE_ID}: failed to respond: {e}");
        }
    }
}

/// The result of a catalog fetch: translated models, or the upstream's
/// non-200 response for verbatim relay.
enum CatalogFetch {
    Ok(models_translate::TranslatedModels),
    NonOk(reqwest::blocking::Response),
}

/// The generic upstream failure body (spec §6: transport errors surface as
/// `502 upstream_error` and are never swallowed).
fn upstream_error() -> ErrorDocument {
    ErrorDocument::new(ErrorCode::UpstreamError, "upstream request failed")
}

fn upstream_error_with(e: anyhow::Error) -> ErrorDocument {
    // The error context names the failing step, never the body contents.
    eprintln!("{SERVICE_ID}: upstream error: {e:#}");
    upstream_error()
}

/// Extract forwarding headers from an incoming request (all except auth/host
/// and hop-by-hop / body-describing headers that reqwest recalculates for the
/// translated JSON body). The proxy's own `Authorization` replaces any
/// caller-supplied credential header.
fn build_upstream_headers(
    auth_header: &'static str,
    host_header: &HeaderValue,
    req: &Request,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for header in req.headers() {
        let name_lower = header.field.as_str().to_ascii_lowercase();
        // Strip auth (replaced below), host (replaced below), and body /
        // transport headers that reqwest recomputes for the translated
        // `mistral_body` sent via `.json()`. Forwarding a stale
        // `content-length` or `content-type` would mismatch the new body and
        // cause truncation or 400s upstream.
        if matches!(
            name_lower.as_str(),
            "authorization"
                | "host"
                | "content-length"
                | "content-type"
                | "transfer-encoding"
                | "connection"
        ) {
            continue;
        }
        let Ok(header_name) = HeaderName::from_bytes(name_lower.as_bytes()) else {
            continue;
        };
        if let Ok(value) = HeaderValue::from_bytes(header.value.as_bytes()) {
            headers.append(header_name, value);
        }
    }
    let mut auth_value = HeaderValue::from_static(auth_header);
    auth_value.set_sensitive(true);
    headers.insert(AUTHORIZATION, auth_value);
    headers.insert(HOST, host_header.clone());
    headers
}

/// Relay a reqwest response back through tiny_http (passthrough).
///
/// This reads the entire upstream body into memory before forwarding. That is
/// acceptable for the bounded `/v1/models` list response, which is the only
/// current caller. If a future passthrough endpoint returns a chunked or
/// unbounded streaming body, switch to a streaming relay to avoid blocking
/// until the upstream stream closes.
fn relay_response(req: Request, upstream_resp: reqwest::blocking::Response) -> Result<()> {
    let status = upstream_resp.status();
    let mut response_headers = Vec::new();
    for (name, value) in upstream_resp.headers().iter() {
        if matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "connection" | "trailer" | "upgrade"
        ) {
            continue;
        }
        if let Ok(h) = Header::from_bytes(name.as_str().as_bytes(), value.as_bytes()) {
            response_headers.push(h);
        }
    }

    let content_length = upstream_resp.content_length().and_then(|len| {
        if len <= usize::MAX as u64 {
            Some(len as usize)
        } else {
            None
        }
    });

    let response = Response::new(
        StatusCode(status.as_u16()),
        response_headers,
        Box::new(upstream_resp) as Box<dyn Read + Send>,
        content_length,
        None,
    );
    if let Err(e) = req.respond(response) {
        eprintln!("{SERVICE_ID}: failed to relay response: {e}");
    }
    Ok(())
}

/// Entry point: read the key, load config, bind, and serve.
pub fn run_main(args: Args) -> Result<()> {
    let auth_header = secrets::read_auth_header(API_KEY_ENV_VAR, BIN_NAME)?;

    let config_dir = resolve_config_dir(args.config_dir)?;
    let resolved = load_config(
        &JsoncDiskConfigStore::new(&config_dir),
        SERVICE_ID,
        &MISTRAL_DEFAULTS,
    )?;
    match &resolved.loaded_from {
        Some(path) => eprintln!("{SERVICE_ID}: loaded config from {path}"),
        None => eprintln!(
            "{SERVICE_ID}: no {} found, using built-in defaults",
            config_filename(SERVICE_ID)
        ),
    }

    let upstream_base = resolve_upstream_base(args.upstream_base, &resolved, DEFAULT_UPSTREAM_BASE);
    let proxy = Arc::new(MistralProxy::new(
        auth_header,
        resolved,
        upstream_base,
        service(),
    )?);

    let (listener, bound_addr) = bind_listener(args.port)?;
    if let Some(path) = args.server_info.as_ref() {
        write_server_info(path, bound_addr.port())?;
    }
    let server = Server::from_listener(listener, None)
        .map_err(|err| anyhow!("creating HTTP server: {err}"))?;

    eprintln!(
        "{SERVICE_ID} listening on {bound_addr} → {}",
        proxy.upstream_base
    );
    if !proxy.service_enabled() {
        eprintln!(
            "{SERVICE_ID}: service disabled by config; discovery is empty and traffic is refused"
        );
    }

    let shutdown_policy = if args.http_shutdown {
        ShutdownPolicy::PostOrGet
    } else {
        ShutdownPolicy::PostOnly
    };
    serve(server, proxy, shutdown_policy)
}

/// Resolve the proxy config directory (spec §7): CLI flag >
/// `PC_PROXY_CONFIG_DIR` > `~/.prompt-cult`.
fn resolve_config_dir(cli_flag: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = cli_flag {
        return Ok(dir);
    }
    if let Ok(dir) = std::env::var("PC_PROXY_CONFIG_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").context("resolving ~/.prompt-cult: HOME is not set")?;
    Ok(Path::new(&home).join(".prompt-cult"))
}

/// The accept loop: each request is handled on its own thread. A graceful
/// `/shutdown` responds first, then terminates the process (the loop itself
/// is parked in `accept`).
pub fn serve(
    server: Server,
    proxy: Arc<MistralProxy>,
    shutdown_policy: ShutdownPolicy,
) -> Result<()> {
    for request in server.incoming_requests() {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            if proxy.handle(request, shutdown_policy) == Flow::Shutdown {
                std::process::exit(0);
            }
        });
    }
    Err(anyhow!("server stopped unexpectedly"))
}

fn bind_listener(port: Option<u16>) -> Result<(TcpListener, SocketAddr)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port.unwrap_or(0)));
    let listener = TcpListener::bind(addr).with_context(|| format!("failed to bind {addr}"))?;
    let bound = listener.local_addr().context("failed to read local_addr")?;
    Ok((listener, bound))
}

fn write_server_info(path: &Path, port: u16) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let info = ServerInfo {
        port,
        pid: std::process::id(),
    };
    let mut data = serde_json::to_string(&info)?;
    data.push('\n');
    let mut f = File::create(path)?;
    f.write_all(data.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod route_tests {
    use super::RouteKind;
    use super::classify_route;
    use pretty_assertions::assert_eq;

    #[test]
    fn models_route_ignores_query_string() {
        assert_eq!(
            classify_route("/v1/models?client_version=0.121.0"),
            RouteKind::Models
        );
        assert_eq!(classify_route("/v1/models"), RouteKind::Models);
        assert_eq!(classify_route("/models"), RouteKind::Models);
    }

    #[test]
    fn other_routes_classify() {
        assert_eq!(classify_route("/service"), RouteKind::ServiceList);
        assert_eq!(classify_route("/service?x=1"), RouteKind::ServiceList);
        assert_eq!(classify_route("/v1/responses"), RouteKind::Responses);
        assert_eq!(classify_route("/health"), RouteKind::Health);
        assert_eq!(classify_route("/shutdown"), RouteKind::Shutdown);
        assert_eq!(classify_route("/nope"), RouteKind::Forbidden);
        assert_eq!(classify_route("/v1/unknown?x=1"), RouteKind::Forbidden);
    }
}
