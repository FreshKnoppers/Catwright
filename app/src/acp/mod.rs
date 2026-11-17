//! ACP (Agent Client Protocol) client core for `vibe-acp`: spawns the agent
//! as a child process, speaks JSON-RPC 2.0 over its stdin/stdout and emits
//! typed events over an mpsc channel. The wire format is specified (and
//! verified against Vibe 3.1.0) in docs/acp.md.
//!
//! - `rpc`: pure line codec for JSON-RPC 2.0 messages.
//! - `protocol`: tolerant serde types for the ACP schema + outgoing params.
//! - `client`: process/threads, request routing and `AcpEvent`s.
//! - `session`: pure, IO-free `SessionView` state built from ACP events (the UI renders only this).

pub mod client;
pub mod protocol;
pub mod rpc;
pub mod session;
