# Prompt Cult Proxy Specification

**Status: v1.0 — normative for reference implementations.** This is a family
spec for OpenAI-compatible secure proxies, not a single proxy: any process
that satisfies every MUST below is a Prompt Cult proxy, whatever vendor,
endpoint, or key it fronts. The Mistral proxy in this repo is the reference
implementation. Security invariants and lineage live in
[`hardening.md`](hardening.md); known bugs live in
[`appendix-proxy-bugs.md`](appendix-proxy-bugs.md). Wire and config shapes
are defined as RFC 8927 JSON Type Definition schemas in [`schemas/`](../schemas/).

## 1. Purpose & threat model

A Prompt Cult proxy exists so that an agent harness can be **network-isolated
and credential-free**. The proxy is an **application-layer relay**, not a
transparent web proxy: it terminates the harness's OpenAI-compatible
requests, holds the credentials, decides what is permitted, talks to the
upstream provider, and streams the answer back.

- The **harness** (agent/TUI/CLI) holds no API keys and no provider
  configuration. It learns what it may use by **discovering** it from the
  proxy. A harness that reads provider config or keys itself is not a
  compliant Prompt Cult harness.
- The **proxy** is the sole credential holder and policy enforcement point.
  An operator (or orchestrator) runs it with keys in its environment; the
  harness only ever reaches its HTTP surface.
- Deployment shape: sidecar in docker-compose / Kubernetes / podman, or plain
  loopback on a workstation. **Unix-domain-socket transport is planned but
  not specified in v1**; loopback TCP is the v1 transport. When UDS lands,
  upstream's deny-permissions approach (openai/codex#24970) is the baseline.
- The proxy MAY hold only one credential today (one LLM API key per
  service); the design must not preclude holding other kinds of API keys
  later.

The attack we design against: anything that can read the proxy's memory,
core dumps, swap, or logs; and a misbehaving harness attempting to escalate
beyond its allowed model set.

## 2. Identity: service slugs, aliases, stability rules

- Every service a proxy offers MUST have an id: a stable, lowercase,
  hyphenated slug (`mistral-ai`, `opencode-go`, `opencode-zen`).
- Convention: `vendor-endpoint` (§3). Mono providers may carry
  aliases (`anthropic` for `anthropic-anthropic`) surfaced at the UI/CLI
  layer; the wire id never changes once a proxy ships, or existing
  configs silently stop loading.
- Every log line a proxy emits MUST be prefixed with the proxy's id, so an
  operator running several proxies can attribute output unambiguously.
- Config files are keyed by id: `<service-id>.jsonc` (§7).

## 3. Inference services: vendor + endpoint + key

An **inference service** is the triple **vendor + endpoint + key**:

- **vendor** — the platform: "OpenCode AI", "Mistral AI", "Anthropic",
  "OpenAI".
- **endpoint** — the vendor's short name for the API surface: `go`, `zen`
  for OpenCode; `mistral` for Mistral; mono providers use the vendor name
  (`anthropic` for Anthropic). The endpoint short name appears in the
  service slug and MAY appear in the upstream URL path.
- **key** — one API key per service, sourced from a named environment
  variable (`api_key_env_var`, e.g. `MISTRAL_API_KEY`).

Rules:

- **Enabled = key present, AND-ed with operator intent.** A service is
  enabled iff its `api_key_env_var` resolves to a non-empty value in the
  proxy's environment AND the operator has not set `enabled: false` in the
  service config. Absent key ⇒ disabled service ⇒ no discovery, no
  traffic. Whether a `.env` file gets loaded into the environment is a
  deployment concern outside the core: the core observes only "key present:
  true|false".
- **One vendor, many endpoints, one key that may work on some and not
  others.** OpenCode AI ships `go` and `zen` endpoints under one license
  key; the key may be entitled for `go` but not `zen` (or vice versa, or
  both, or neither). Model this as two services (`opencode-go`,
  `opencode-zen`) sharing one `api_key_env_var` value where entitled, each
  independently enabled/disabled by key presence and operator config. The
  proxy MUST NOT assume one endpoint's entitlement implies another's.
- **Models appear on other vendors' endpoints** ("it's complicated"):
  e.g. Anthropic models reachable via an OpenAI-compatible aggregator.
  The service abstraction treats these as distinct models of distinct
  services; the `aliases` field and per-model `upstream_model_id` (§4) are
  the reconciliation points, never silent remapping.
- `GET /service` MUST list at least one service with `kind: "inference"`.
  The `kind` enum is deliberately closed at `["inference"]` in v1;
  future kinds (e.g. MCP tool services) extend the enum and add schemas.
- The standalone service contract is
  [`schemas/service.jtd.json`](../schemas/service.jtd.json); the composite
  response (services with embedded models) is
  [`schemas/service-list.jtd.json`](../schemas/service-list.jtd.json).

## 4. Models: the complete value object

A model is a **complete serde value object** defined by
[`schemas/model.jtd.json`](../schemas/model.jtd.json), not a bare slug:

- required: `id` (proxy-visible), `service_id`, `enabled`,
  `api_key_env_var` (denormalized so a model object is self-contained),
- optional: `display_name`, `description`, `context_window`,
  `max_output_tokens`, `base_instructions`, `supported_reasoning_efforts`,
  `input_modalities`, `upstream_model_id`, and the reserved fields
  `preference`, `cost_input_per_million_usd`, `cost_output_per_million_usd`,
  `skills`.

Rules:

- **Custom system prompts live in the proxy.** `base_instructions` is the
  system prompt served with the model; per-model overrides replace it
  (inline or from a file). The harness never supplies its own provider
  config for this.
- `upstream_model_id` carries the ID forwarded upstream when it differs
  from `id` (e.g. a proxy that prefixes IDs with `go-`/`zen-` for
  disambiguation strips the prefix upstream). Absent means they are equal.
- Reserved fields (`preference`, `cost_*`, `skills`) are
  **forward-compatible**: consumers MUST tolerate their absence; producers
  MUST NOT be required to fill them. They exist so cost-aware routing,
  user preference, and per-model skills can grow without schema breakage.
- Future skill advertisement (`skills`) follows the same rule.

## 5. Lockdown

An enterprise may lock a network-isolated harness to an approved model set.
The proxy is the enforcement point, and it MUST enforce at **two** places:

1. **Discovery**: enabled=false models and models matching exclusion globs
   are absent from `GET /service` and `GET /v1/models` listings. A disabled
   model is invisible, not greyed out.
2. **Request time**: any relay request naming a model that is not present,
   allowed, and enabled is refused with `403` and body
   `{"error":{"code":"model_not_enabled","model":…}}`
   ([`schemas/error.jtd.json`](../schemas/error.jtd.json)). A harness —
   however buggy or malicious — cannot escalate to a non-approved model,
   because it never learns a model ID the proxy would accept.

A request naming a service whose key is absent or disabled returns
`403 service_disabled`. Error bodies never contain key material, request
bodies, or upstream bodies.

## 6. Discovery & relay API

All endpoints are loopback-TCP, HTTP/1.1, JSON. The proxy is not a general
web proxy: it serves exactly these routes; anything else is `403 forbidden`.

| Route | Method | Purpose | Response schema |
|---|---|---|---|
| `/service` | GET | List services (≥1 inference); full model documents | `service-list.jtd.json` |
| `/v1/models` (also `/models`; any query string ignored) | GET | Codex-compatible picker list | `models-response.jtd.json` |
| `/v1/responses` | POST | OpenAI Responses relay (streaming and non-streaming) | passthrough/translated |
| `/health` | GET | Liveness: proxy id + upstream base URL | — |
| `/shutdown` | POST (or GET when started `--http-shutdown`) | Loopback-only graceful stop | — |

MUSTs:

- Route matching compares the **path only** — codex appends
  `?client_version=X` to `/v1/models`; exact-URL matching is the bug that
  made discovery 403 (see appendix).
- `GET /v1/models` MUST return the codex `ModelsResponse` shape
  (`{"models":[…]}`), never a raw upstream relay: harnesses decode it
  strictly and silently fall back to bundled catalogs otherwise.
- Discovery stays **dynamic**: the proxy queries the upstream catalog on
  every request, so newly released provider models appear without a proxy
  release.
- `POST /v1/responses` MUST forward the model ID **verbatim** — no
  substitution, no "upgrade", no canonicalisation. If the upstream reports
  a canonical name for an alias, the proxy logs both (verbose) and MUST
  NOT rewrite client-visible identity. When `upstream_model_id` applies,
  the mapping is explicit config, never inference.
- Streaming responses are relayed as SSE without buffering the whole body;
  the upstream client has **no default timeout** (a silent 30s cap kills
  slow generations — see appendix, upstream #4336).
- Upstream transport errors MUST surface to the harness as
  `502 upstream_error`; the proxy never swallows them (see appendix,
  upstream #42914's black-hole class).
- The relay injects `Authorization: Bearer <key>` upstream and strips any
  caller-supplied credential headers. The key is never echoed to the
  harness.
- A translating proxy (Responses ↔ chat-completions) is a legitimate
  implementation (the reference Mistral proxy is one), but translation is
  the highest-risk code in the family (§12) and MUST be minimal and
  strictly tested.

## 7. Configuration

- A proxy MUST NOT read any harness config (`config.toml` or otherwise).
  Each service owns exactly one JSONC file:
  `$CONFIG_DIR/<service-id>.jsonc`, shaped by
  [`schemas/proxy-config.jtd.json`](../schemas/proxy-config.jtd.json).
- The config dir is `PC_PROXY_CONFIG_DIR` if set, else the XDG-style prompt
  cult home (`~/.prompt-cult` on workstations), else the pod/sidecar
  mount point supplied by the orchestrator.
- Missing file ⇒ compiled-in defaults. **Present-but-malformed file
  (bad JSONC, unknown key, invalid glob, mutually exclusive fields both
  set) is a hard startup error** — silently ignoring a broken config is
  how "the wrong model ran" bugs happen. On startup the proxy MUST log
  one line: config file found (absolute path) or defaults in use.
- Keys: `upstream_base_url`, `enabled` (service kill-switch),
  `model_exclude_globs` (globs: `*` wildcard, no `**`, exact match
  without `*`, case-sensitive; overriding replaces defaults entirely;
  empty list disables filtering), `model_overrides` (per-model
  `base_instructions` | `base_instructions_file` — mutually exclusive —
  `enabled:false` lockdown, `context_window`, `supported_reasoning_efforts`,
  `upstream_model_id`, reserved fields; keys naming unknown models are
  ignored), `log_level`.
- Precedence: CLI flags > config file > compiled-in defaults.
- Storage is behind a `ConfigStore` trait: plain disk (the default) today;
  enterprise secure storage or k8s ConfigMap mounts later, without
  touching proxy logic. The file format MUST stay JSONC across stores
  that surface files.
- Every config key and every endpoint payload in this spec has a JTD
  schema; schemas are the contract, this prose is the guide.

## 8. Logging

- Normal level: startup banner (proxy id, listen address, upstream),
  config-found/defaults line, discovery totals
  (`loaded N models, M after exclusions`), upstream non-200s, fatal
  errors.
- Verbose level adds one line per relayed request: proxy id, per-process
  monotonically increasing counter, requested model, upstream-reported
  model when known.
- Logs MUST NEVER contain the API key, request bodies, or response bodies.
  Nothing is keyed by user identity; the counter is per-process.
- Error bodies use [`schemas/error.jtd.json`](../schemas/error.jtd.json);
  messages are operator-facing and MUST be treated as untrusted input by
  the harness.

## 9. Memory hardening invariants

The full normative set (single copy, low-level `read(2)`, zeroize on every
error path, charset validation, `mlock(2)`, never logged, environment-first
key source, loopback-only bind, pre-main process hardening, no relay
timeout) is defined in
[`hardening.md`](hardening.md) and is part of this spec by reference. Every
MUST there is a MUST here.

## 10. Deployment profiles

- **Sidecar** (docker-compose / k8s pod / podman): the proxy runs with the
  key; the harness container has no key env vars and no egress except the
  proxy. The proxy binds loopback; in shareable-network sidecars bind the
  pair interface only.
- **Workstation loopback**: `prompt-cult-<service>-proxy` on 127.0.0.1;
  the operator exports the key in the shell that starts the proxy.
- **Unix domain socket** (future): same routes over a UDS with
  deny-permissions; v1 proxies need not support it, but MUST not architect
  against it (keep the HTTP core transport-agnostic where cheap).

## 11. Versioning & JTD schema stability rules

- Schemas are the contract. A schema file changes only in a minor or major
  release; additive optional fields may land in patch releases. Required
  fields are never added, removed, or retyped within a major.
- The composite `service-list.jtd.json` duplicates the `service` and
  `model` definitions (RFC 8927 refs are same-document only); a schema
  sync test in `proxy-core` asserts the copies stay identical to the
  standalone contracts.
- Two documented JTD/Rust gaps: JSON null is not expressible in JTD, so
  nullable codex-compat fields (`description`, `upgrade`, …) are annotated
  in `metadata`; `additionalProperties: true` marks deliberate extension
  points. Anything not marked is closed (`additionalProperties: false`).
- A proxy MUST report its spec version in `/health` once a second spec
  version exists.

## 12. Staying with the herd (advisory)

This section is advice, not requirement. Following projects and proxy
authors choose.

OpenAI's `codex-rs/responses-api-proxy` (Apache-2.0) is the most-attacked,
most-reviewed proxy of this shape in existence. Reuse it **to avoid
security bugs by moving with the herd** — not for reuse's own sake:

- **The key-handling core should track upstream's lineage.** This repo's
  `proxy-core` secrets module is a diff-audited copy of that line
  (fork point `6696e0bbc3`, 2026-04-15; upstream core files unchanged
  since — see the lineage audit in `hardening.md`). When adopting this
  spec, prefer inheriting that module over reimplementing key handling;
  re-read the audit when you rebase.
- **For bring-your-own-OpenAI-key proxies, the ideal shape is OpenAI's
  crate itself**: source-link `openai/codex`'s `codex-rs/responses-api-proxy`
  (git submodule or cargo `[patch]`/path dep) and wrap it with the
  prompt-cult surface (`/service`, `/v1/models`, `/health`). Upstream's
  forwarding core is transport-agnostic enough for this (`--upstream-url`
  points at any Responses-compatible endpoint), and herd review plus
  upstream security fixes become a pull-and-track operation. **Stretch
  goal:** subscribe to upstream security advisories for source-linked
  code and re-audit the diff on every rebase. This spec does not build
  that tooling.
- **Translation logic cannot ride the herd.** Upstream never translates
  Responses ↔ chat-completions, and is moving toward pure Responses
  streaming; vendors (Mistral, OpenCode go/zen, aggregators) will lag at
  different rates. Translation is therefore the highest-risk code you
  will write against this spec: keep it minimal, strictly tested, and
  retire it in favour of a verbatim forwarder the day your vendor ships
  native Responses support. When a vendor falls behind, prefer dropping
  the service over growing the translation layer.
- **Watch upstream's direction, audit before following.** Upstream's
  proxy evolution (e.g. the `network-proxy` line) mostly serves their
  sandbox architecture, not credential-holding relays; adopt ideas
  selectively and only after a diff audit.

## 13. Invariant comments are normative

The reference implementation's secrets module carries upstream's
`TAKE CARE WHEN MODIFYING THIS CODE!!!` comments, reproduced verbatim
from openai/codex#4778. Those comments are the security invariants in
situ — they are normative documentation, not decoration. Edits to that
module that delete or weaken those comments are spec violations, whatever
the code does. The same rule applies to any future module ported from
upstream: the comments come with the code.
