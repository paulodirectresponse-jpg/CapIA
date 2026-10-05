# capia-secrets

Fronteira de segredos da Fase 4 (ADR-078). Crate-folha: não conhece modelo, projeto, mídia, providers nem rede.

- `SecretString` — zeroize; `Debug`/`Display` redigidos; sem `Clone`/`Serialize`.
- `SecretStore` — `platform_store()` (Windows Credential Manager) ou `MemoryStore` **explícito** (nunca silencioso); `CredentialRef` = ponteiro `capia/provider/<id>`.
- Redator central (`redact`, `register_global`): valores registrados (claro/base64/URL-encoded) + padrões (`Authorization`, `x-api-key`, `?key=`, `sk-…`, `AIza…`).
- `install_redacting_panic_hook` — saída de crash sem segredo.

Testes: `cargo test -p capia-secrets`. Canário de ponta a ponta: `capia-intelligence/tests/service.rs`.
