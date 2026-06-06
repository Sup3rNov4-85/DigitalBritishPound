//! Local status file + optional localhost HTTP (read-only). Full RPC: docs/ROADMAP_V2.md.

pub mod http;
pub mod status;

pub use status::{read_status, write_status, NodeStatusSnapshot};
