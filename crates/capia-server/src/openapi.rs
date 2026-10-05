//! Documento OpenAPI 3.1 gerado do catálogo (a mesma fonte de REST, MCP e scopes).

use crate::catalog::{self, Surface};
use serde_json::{Map, Value, json};

fn error_schema() -> Value {
    json!({
        "type": "object",
        "required": ["code", "message", "request_id"],
        "properties": {
            "code": {"type": "string"}, "message": {"type": "string"},
            "details": {"type": "object"}, "request_id": {"type": "string"}
        }
    })
}

fn path_params(path: &str) -> Vec<String> {
    path.split('/')
        .filter_map(|s| s.strip_prefix('{').and_then(|x| x.strip_suffix('}')))
        .map(str::to_owned)
        .collect()
}

pub fn document() -> Value {
    let mut paths: Map<String, Value> = Map::new();
    for def in catalog::ops() {
        let pp = path_params(def.path);
        let props = def.schema["properties"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let required: Vec<&str> = def.schema["required"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut parameters: Vec<Value> = pp
            .iter()
            .map(|n| json!({"name": n, "in": "path", "required": true, "schema": props.get(n).cloned().unwrap_or(json!({"type":"string"}))}))
            .collect();
        let mut operation = json!({
            "operationId": def.name,
            "summary": def.summary,
            "x-capia-scope": def.scope.map(crate::scope::Scope::as_str),
            "x-capia-mutating": def.mutating,
            "x-capia-rate-class": def.class.as_str(),
            "x-capia-mcp-tool": (def.surface == Surface::Both).then(|| def.tool_name()),
            "responses": {
                def.status.to_string(): {"description": "Success"},
                "400": {"description": "Malformed request", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Error"}}}},
                "401": {"description": "Missing/invalid token"},
                "403": {"description": "Insufficient scope"},
                "409": {"description": "Conflict (stale revision/plan, project not open, idempotency)"},
                "422": {"description": "Invalid parameters"},
                "429": {"description": "Rate limited (see Retry-After)"},
            },
        });
        if def.scope.is_none() {
            operation["security"] = json!([]);
        } else {
            operation["security"] = json!([{"bearer": []}]);
        }
        if def.name == "uploads.create" {
            parameters.push(json!({"name": "X-Capia-Filename", "in": "header", "required": true, "schema": {"type": "string"}}));
            parameters.push(json!({"name": "X-Capia-Sha256", "in": "header", "required": false, "schema": {"type": "string"}}));
            operation["requestBody"] = json!({
                "required": true,
                "content": {"application/octet-stream": {"schema": {"type": "string", "format": "binary"}}}
            });
        } else if def.method == "GET" || def.method == "DELETE" {
            for (k, v) in &props {
                if !pp.contains(k) {
                    parameters.push(json!({"name": k, "in": "query", "required": required.contains(&k.as_str()), "schema": v}));
                }
            }
        } else {
            let body_props: Map<String, Value> = props
                .iter()
                .filter(|(k, _)| !pp.contains(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let body_req: Vec<&&str> = required
                .iter()
                .filter(|k| !pp.iter().any(|p| p == **k))
                .collect();
            if !body_props.is_empty() {
                operation["requestBody"] = json!({
                    "required": !body_req.is_empty(),
                    "content": {"application/json": {"schema": {"type": "object", "properties": body_props, "required": body_req, "additionalProperties": false}}}
                });
            }
        }
        if def.mutating {
            parameters.push(json!({"name": "Idempotency-Key", "in": "header", "required": false, "schema": {"type": "string", "maxLength": 128}}));
        }
        operation["parameters"] = Value::Array(parameters);
        let entry = paths
            .entry(def.path.to_owned())
            .or_insert_with(|| json!({}));
        entry[def.method.to_ascii_lowercase()] = operation;
    }
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "CapIA local API",
            "version": crate::core::API_VERSION,
            "description": "Local REST API of the CapIA editor (same Engine API as the UI). Loopback by default; bearer tokens with scopes.",
        },
        "servers": [{"url": "http://127.0.0.1:{port}", "variables": {"port": {"default": "5400"}}}],
        "components": {
            "securitySchemes": {"bearer": {"type": "http", "scheme": "bearer", "description": "capia_<64 hex>"}},
            "schemas": {"Error": error_schema()},
        },
        "paths": paths,
    })
}

/// Catálogo em JSON (consumido pelo gerador de documentação e pela matriz de scopes).
pub fn catalog_json() -> Value {
    Value::Array(
        catalog::ops()
            .iter()
            .map(|d| {
                json!({
                    "name": d.name, "summary": d.summary, "scope": d.scope.map(crate::scope::Scope::as_str),
                    "mutating": d.mutating, "class": d.class.as_str(), "method": d.method, "path": d.path,
                    "status": d.status, "project": d.project,
                    "surface": if d.surface == Surface::Both { "rest+mcp" } else { "rest" },
                    "tool": (d.surface == Surface::Both).then(|| d.tool_name()),
                    "schema": d.schema,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_document_covers_every_operation_with_scope_and_idempotency_metadata() {
        let doc = document();
        assert_eq!(doc["openapi"], "3.1.0");
        let mut n = 0;
        for def in catalog::ops() {
            let op = &doc["paths"][def.path][def.method.to_ascii_lowercase()];
            assert_eq!(op["operationId"], def.name, "{}", def.path);
            if def.scope.is_some() {
                assert_eq!(op["security"], json!([{"bearer": []}]));
                assert!(op["x-capia-scope"].is_string());
            }
            if def.mutating {
                assert!(
                    op["parameters"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|p| p["name"] == "Idempotency-Key"),
                    "{}",
                    def.name
                );
            }
            for p in path_params(def.path) {
                assert!(
                    op["parameters"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|x| x["name"] == p.as_str() && x["in"] == "path")
                );
            }
            n += 1;
        }
        assert_eq!(n, catalog::ops().len());
        assert!(
            doc["components"]["schemas"]["Error"]["required"]
                .as_array()
                .unwrap()
                .len()
                == 3
        );
    }
}
