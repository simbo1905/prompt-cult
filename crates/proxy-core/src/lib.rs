//! Shared library for Prompt Cult secure proxies.
//!
//! Prompt Cult proxies are application-layer relays that hold LLM API keys on
//! behalf of a network-isolated harness: the harness has no credentials and no
//! provider configuration, and discovers the services and models a proxy
//! offers through the proxy's HTTP surface (`GET /service`, `GET /v1/models`).
//!
//! This crate provides everything a proxy needs except the vendor-specific
//! translation logic:
//!
//! - the `Service` and `Model` value objects (RFC 8927 JTD-conformant, see
//!   `schemas/`),
//! - the per-service JSONC configuration loader with hard-error semantics,
//! - the memory-hardened API-key handling (`secrets`) and process
//!   hardening, both descended from OpenAI's battle-tested
//!   `responses-api-proxy` line (see `docs/hardening.md` for the lineage
//!   audit),
//! - the codex-compatible discovery types for `GET /v1/models`.
//!
//! The binding contract for every proxy built on this crate is
//! `docs/proxy-spec.md`.

pub mod config;
pub mod discovery;
pub mod error;
pub mod model;
pub mod process_hardening;
pub mod secrets;
pub mod service;
