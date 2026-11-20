//! skipper: one binary with a deterministic core (`core`, the only part that may hold a client, a token or a write
//! path, and that works with no model) and an LLM advisor (`advisor`, read-only tools plus propose). The binary
//! (main.rs) serves them through the generated ServiceManager.

pub mod advisor;
pub mod core;
