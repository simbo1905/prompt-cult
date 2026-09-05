# Appendix: proxy-related bugs and incidents (FYI)

A living record of known bugs — upstream (openai/codex), the fork
(prompt-cult/codex), and this project — that pertain to proxy features or
proxy security. Old or new, merged or not. Future projects adopting the
Prompt Cult proxy should read this as the "gotcha list".

## Upstream (openai/codex)

| Ref | State | Summary | Relevance |
|---|---|---|---|
| GHSA-w5fx-fh39-j5rw / CVE-2025-59532 | published | Sandbox bypass: model-generated `cwd` treated as sandbox writable root | Not proxy code, but the same trust boundary lesson: never let model-controlled strings define security parameters |
| #34138 | open | API key length/charset validation too strict for custom `responses-api-endpoint` keys | **Directly affects our `secrets.rs`.** Our `/^[A-Za-z0-9\-_]+$/` port rejects keys containing dots or other characters; verify the charset against each vendor before enabling a service (Mistral keys are alphanumeric; other vendors may not be) |
| #37393 | open | OpenCode-compatible providers: non-standard `/v1/models` key + no Responses API | The exact problem Prompt Cult proxies solve: vendors publish model lists under different keys and lack Responses endpoints; translation is required |
| #16079 | open | Linux + HTTP proxy env vars: CLI fails in both API-key and device-auth modes while curl works | Proxy *environment* (corporate proxies) interfering with the client; a reminder that deployment env vars are a common failure layer |
| #42914 | open | Responses WebSocket black-hole after VPN disconnect leaves CLI "Thinking" for 5 minutes | Streaming connections that die silently hang the harness; our relay must surface upstream transport errors, not swallow them |
| #4336 | merged | 30s default request timeout silently killed slow streams | Adopted as invariant 10 (no default timeout on the relay client) |
| #4778 | merged | `BufReader` on stdin retained an un-zeroable copy of the key | Adopted as invariant 2 (low-level `read(2)`) |
| #9409 + #9693 | merged + reverted | WebSocket proxy support added then rolled back | Streaming-transport experiments are unstable ground; keep the relay on HTTP SSE |
| #11555 | referenced | macOS malloc stack-logging sprayed allocator diagnostics into the TUI | The fork's process-hardening strips `MallocStackLogging`/`MallocLogFile`; noisy allocator diagnostics can also leak allocation patterns |

## Fork (prompt-cult/codex)

| Ref | State | Summary |
|---|---|---|
| zen-proxy (de7d536afb) | deprecated | First-generation proxy: no per-proxy config, no model filtering, no structured routing log; predates the proxy protocol; do not extend |
| TUI ↔ proxy config-server integration | abandoned | After four attempts the TUI could not be made bug-free as a proxy config client; the TUI escape pod was abandoned in favour of this proxy-first spec (harness-side, recorded as the motivating incident) |
| Non-hermetic app-server tests | fixed in escape pod v1 | Tests read the developer's real `~/.codex/config.toml` (rejecting `service_tier = "priority"`); fixed by pointing `ConfigBuilder` at a temp home. Lesson: config-loading code paths in tests must be hermetic |
| Hanging websocket-mock tests | pre-existing | `turn_start_forwards_client_metadata_to_responses_websocket_request_body_v2` and the `exec_process` integration test hang indefinitely on loopback websocket connect failure (unbounded `wait_for_request`); same class as upstream #42914 |
| Model discovery 403 on query string | fixed (1f9221dae2 era) | Route matching compared raw URL including `?client_version=X`, so `/v1/models` 403'd; fixed by path-only matching — codified as a MUST in the spec |

## This project (simbo1905/prompt-cult)

None recorded yet. Add entries here as they are found.
