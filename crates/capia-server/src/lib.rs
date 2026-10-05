//! `capia-server` — host local da Engine API (Fase 6, ADR-102): REST `/v1`, MCP e webhooks sobre o
//! MESMO `capia-editor-api` + `capia-intelligence` que a UI usa. Ver `docs/phase6/`.

#![forbid(unsafe_code)]

pub mod catalog;
pub mod error;
pub mod mac;
pub mod scope;
