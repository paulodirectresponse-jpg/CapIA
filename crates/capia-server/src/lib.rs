//! `capia-server` — host local da Engine API (Fase 6, ADR-102): REST `/v1`, MCP e webhooks sobre o
//! MESMO `capia-editor-api` + `capia-intelligence` que a UI usa. Ver `docs/phase6/`.

#![forbid(unsafe_code)]

pub mod auth;
pub mod catalog;
pub mod config;
pub mod core;
pub mod error;
pub mod http;
pub mod mac;
pub mod mcp;
pub mod openapi;
pub mod ops;
pub mod pump;
pub mod ratelimit;
pub mod scope;
pub mod server;
pub mod uploads;
pub mod webhooks;

pub use crate::config::ServerConfig;
pub use crate::core::{CallCtx, Core, Reply};
pub use crate::server::Server;
