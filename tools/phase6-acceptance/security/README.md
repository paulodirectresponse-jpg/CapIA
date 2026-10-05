# security

Segurança da API local e do instalador. `node tools/phase6-acceptance/security/run.mjs` — passos em [`steps.json`](steps.json):

| Passo | Estado típico sem `CAPIA_P6_HEAVY` |
|---|---|
| `architecture-boundaries` (`tools/check-architecture.mjs`) | roda (`passed` se a matriz inclui o crate do servidor) |
| `webhook-receiver-forgery-replay` | `passed` |
| `catalog-scope-invariants`, `mac-primitives`, `webhook-delivery-policy`, `server-db-security` | `not_available` (compilam Rust; use `CAPIA_P6_HEAVY=1`) |
| `phase4-security-regression`, `phase5-security-regression` | `not_available` (heavy) |
| `api-security-suite`, `api-fuzz` | `not_available` até a frente do servidor entregar `tests/security.rs` / `tests/fuzz.rs` |
| `dependency-audit` | `not_available` sem `cargo-deny` |
| `independent-pentest` | **`pending_external`** (relatório de terceiro; o pentest básico da engenharia é a suíte acima) |

O que a suíte da API deve provar (PHASE6 §32): 401 sem token, 403 por scope, revogação imediata, escalada por rota, replay de idempotência, revisão antiga (409), JSON malformado, path traversal, upload acima do limite, SSRF (webhook e Gateway), CORS/Host, forja e replay de webhook, canário de segredo por REST/MCP/webhook/relatório de crash/diagnóstico, kill durante upload/apply/Run/webhook/export. Mutações esperadas (§25): auth ignorada, scope ignorado, idempotência desligada, assinatura não verificada, revisão antiga aceita, redação desligada — cada uma deve derrubar pelo menos um teste.

Evidência do pentest independente: `target/phase6-acceptance/evidence/pentest-report.json` com `performed_by`, `performed_at` (ISO, não futuro) e `result: "passed"|"failed"` (checagem genérica mínima; o relatório completo fica com quem contratou). Resumo do modelo de ameaças: [`docs/api/security.md`](../../../docs/api/security.md).
