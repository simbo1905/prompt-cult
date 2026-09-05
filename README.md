# prompt-cult

Prompt Cult is a **family spec and shared library for OpenAI-compatible
secure proxies** — one process holds the LLM API keys so the agent harness
doesn't have to.

The harness (your own agent, TUI, or CLI) runs network-isolated with zero
credentials and zero provider configuration. A Prompt Cult proxy is an
application-layer relay, sidecar-deployable (docker-compose, Kubernetes, or
plain loopback), that:

- **holds the keys** in memory-hardened storage (read via low-level `read(2)`
  or from the environment, `mlock(2)`-locked, zeroized on every error path,
  never logged, never written to disk),
- **tells the harness what it may use** via `GET /service` and
  `GET /v1/models` — discovery, not configuration, is how a harness learns
  its allowable models,
- **enforces the lockdown**: requests naming models that are not allowed and
  enabled are refused (`403 model_not_enabled`), so a misbehaving agent
  cannot escalate to a non-approved model,
- **relays OpenAI-compatible traffic** (streaming included) to the upstream
  provider, injecting authentication.

## Repository layout

| Path | Contents |
|---|---|
| `docs/proxy-spec.md` | The binding proxy specification |
| `docs/hardening.md` | Memory/process hardening invariants + upstream lineage audit |
| `docs/appendix-proxy-bugs.md` | Known proxy-related bugs, old and new |
| `schemas/` | RFC 8927 JSON Type Definition schemas for all config and endpoints |
| `crates/proxy-core` | Shared library: service/model types, config store, hardened secrets |
| `crates/mistral-proxy` | Reference implementation: Mistral AI translating proxy |

## Status

Spec-first repository: the specification and shared library come first;
per-service proxies (Mistral shipped first; OpenCode Go/Zen planned) are
reference implementations of the spec. See `docs/proxy-spec.md` §12 for the
advisory on staying aligned with OpenAI's upstream proxy code.

## Running the reference proxy

```shell
export MISTRAL_API_KEY=...          # or pipe the key to stdin
cargo run -p prompt-cult-mistral-proxy -- --port 8090
```

The proxy binds loopback only, reads its key once (environment first,
low-level `read(2)` stdin fallback), and serves the spec's endpoint table:
`GET /service`, `GET /v1/models` (codex picker shape), `POST /v1/responses`
(streaming and non-streaming), `GET /health`, and `POST /shutdown`.
Optional per-service config lives at `$PC_PROXY_CONFIG_DIR/mistral-ai.jsonc`
(or `~/.prompt-cult/mistral-ai.jsonc`) — see
[`examples/mistral-ai.jsonc`](examples/mistral-ai.jsonc); a missing file
means compiled-in defaults, a malformed file is a hard startup error.

## Development

```shell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

Apache-2.0 — see `LICENSE` and `NOTICE` (this project derives from OpenAI's
`codex` repository).
