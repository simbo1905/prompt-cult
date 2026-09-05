# Memory and process hardening

This document defines the security invariants every Prompt Cult proxy MUST
uphold, records where each invariant came from (OpenAI's upstream
`codex` lineage or this fork), and audits how closely our code tracks
upstream. It is the security core of the project: the whole point of a
Prompt Cult proxy is that the API keys live here — in hardened memory — and
nowhere else.

## Threat model

- The **harness** (agent/TUI/CLI) is untrusted with respect to secrets: it
  holds no credentials and no provider configuration. It may run
  network-isolated (container, pod, or sandbox) and talks to the proxy over
  loopback TCP (Unix-domain sockets are a future transport).
- The **proxy** is the only component that talks to the upstream LLM API. A
  privileged operator runs the proxy; unprivileged callers only reach its
  HTTP surface.
- The attacker we care about: anything that can read the proxy's memory,
  core dumps, swap, or logs. Hence: no key material on disk, no key material
  in logs, exactly one copy of the key in memory, locked.

## Invariants (normative MUSTs)

1. **Single copy.** There MUST be exactly one copy of the API key in memory,
   in the `Authorization` header value, page-locked with `mlock(2)` (Unix).
   The key is never materialized in a second `String`, `Vec`, or buffer that
   outlives the read.
2. **Low-level stdin read.** When reading from stdin, the proxy MUST use a
   low-level `read(2)` into a stack buffer. `std::io::stdin()` has an
   internal `BufReader`
   ([stdio.rs](https://github.com/rust-lang/rust/blob/bcbbdcb8522fd3cb4a8dde62313b251ab107694d/library/std/src/io/stdio.rs#L250-L252))
   that retains a copy of stdin data with no way to zero it out.
3. **Zeroize every error path.** The stack buffer MUST be zeroized on every
   exit — success and every failure (overlong key, invalid charset, EOF
   without newline, I/O error). No early `?` that skips the zeroize.
4. **Charset validation before allocation.** The key MUST match
   `/^[A-Za-z0-9\-_]+$/` before the heap copy is made, so no control
   characters or NULs enter the header value.
5. **`mlock(2)` best-effort, failure ignored.** The leaked header value is
   page-aligned and locked so it cannot be written to swap. Locking failures
   are logged at most and never fatal (same as upstream).
6. **Never logged, never persisted.** Log lines MUST NOT contain the key,
   request bodies, or response bodies. Error messages MUST NOT embed the
   key. Dump-to-disk of traffic is NOT supported in Prompt Cult proxies
   (upstream's `--dump-dir` was deliberately not ported).
7. **Environment-first key source.** The key comes from the process
   environment (the proxy's `read_auth_header(env_var, bin)` checks the
   service's `api_key_env_var` first), falling back to stdin. Whether a
   `.env` file is loaded into the environment is a deployment concern
   outside the core: the core only ever observes "key present: true|false".
8. **Loopback-only bind.** The proxy binds `127.0.0.1` only; it is never
   auto-spawned by a harness, and the operator launches it with the key in
   its environment.
9. **Process hardening pre-main.** Proxy binaries MUST call
   `process_hardening::pre_main_hardening()` from a `#[ctor]` before `main`:
   disable core dumps (`setrlimit(RLIMIT_CORE, 0)`), deny `ptrace` attach
   (`PR_SET_DUMPABLE` on Linux, `PT_DENY_ATTACH` on macOS), and strip
   dangerous loader environment variables (`LD_*`, `DYLD_*`, and macOS
   malloc stack-logging controls). This is defense in depth for invariant 1:
   even if a second copy existed, core dumps and debugger attach cannot
   harvest it.
10. **No request timeout on the relay client.** The upstream HTTP client
    MUST disable any default request timeout so long-lived response streams
    keep flowing (learned from upstream issue: a 30s default silently kills
    slow generations).

## Upstream survey: what OpenAI did and when

The `responses-api-proxy` line (openai/codex, Apache-2.0) is the origin of
invariants 1–7 and 10:

| Change | PR | Lesson carried forward |
|---|---|---|
| Introduce `responses-api-proxy`: strict forwarder, key on stdin, privileged operator runs proxy, unprivileged user runs codex | openai/codex#4246 | The core split: keys in a separate process from the agent |
| Remove default 30s timeout in the proxy | openai/codex#4336 | Invariant 10 |
| Extract `pre_main_hardening()` into its own crate | openai/codex#4403 | Invariant 9 |
| Use low-level stdin read logic to avoid a `BufReader` | openai/codex#4778 | Invariant 2; the `TAKE CARE WHEN MODIFYING THIS CODE` invariants in `secrets.rs` are reproduced from this change |
| Azure support (`--api-type`, `api-version` query) | openai/codex#6129 | Multi-vendor endpoints via flags, not code paths |
| WebSocket proxy support added, then reverted | openai/codex#9409, #9693 | Streaming transports beyond HTTP SSE were tried upstream and rolled back; treat as unstable ground |

Upstream's later `network-proxy` evolution (MITM CA keys kept in proxy
memory #29013, local credential broker #28034, hardened credential brokerage
#38049/#40466/#40484, hardened listener handoff #40999, hardened MITM
authorization #37211, Unix socket deny permissions #24970, Windows
SID-restricted routing #34613, explicitly permitted loopback targets
#34603) is **surveyed but deliberately not adopted**: it is a large
sandbox-oriented system and not a credential-holding relay. The one idea
parked for the future is #24970's Unix-socket deny permissions — the
security baseline for the planned UDS transport.

Upstream has published one security advisory to date
(GHSA-w5fx-fh39-j5rw / CVE-2025-59532, a sandbox path-configuration
bypass); it does not involve the proxy line.

## Lineage audit (as of 2026-09-05)

Fork point audited against `real-upstream/main` = openai/codex @ `728cb12fe5`;
the fork diverged at `6696e0bbc3` (2026-04-15).

| Our module | Origin | Diff vs upstream `main` | Verdict |
|---|---|---|---|
| `proxy-core/src/secrets.rs` (ported from fork `mistral-proxy/src/read_api_key.rs`) | upstream `responses-api-proxy/src/read_api_key.rs` | Upstream core files are **empty-diff** fork-point → main (only 5 packaging/docs/CI commits since). Fork deltas: adds env-first key path (upstream is stdin-only); strips upstream's invariant comments (restored verbatim in the port) | Upstream-current; the herd's review applies |
| `proxy-core/src/process_hardening.rs` (ported from fork `process-hardening`) | upstream `codex-rs/process-hardening` (extracted in #4403) | Fork removes upstream's unused `disable_process_dumping()` and adds macOS malloc stack-logging cleanup (upstream issue #11555) | Upstream-current plus two small, audited deltas |
| Relay skeleton: loopback bind, `Host` header from upstream URL, no client timeout, header filtering | upstream `responses-api-proxy/src/lib.rs` ideas | No code copied verbatim; ideas re-expressed | Upstream-informed, fork-authored |
| Per-service JSONC config loader (`proxy-core/src/config.rs`, from fork `proxy-protocol`) | fork-authored (4 commits, Issue #6 work) | No upstream equivalent | Fork's own; hard-error semantics are the fork's tested behavior |
| Translation layer (`mistral-proxy`: `translate_request.rs`, `translate_sse.rs`, `models_translate.rs`) | fork-authored | No upstream equivalent — upstream never translates Responses ↔ chat-completions | **Highest-risk code in the family.** No herd review; must stay minimal and strictly tested; first candidate for retirement when a vendor ships native Responses support |

## Staying with the herd

See `docs/proxy-spec.md` §12 for the advisory on tracking upstream (and when
not to).
