# capia-store

Persistência do projeto `.capia` (um arquivo SQLite, schema versionado). ADR-042 (formato), ADR-043 (histórico/idempotência), ADR-044 (transações e crash safety).

- **Modelo:** journal de eventos + snapshots. Estado = último snapshot + replay dos eventos posteriores. Histórico completo persistido (undo/redo após reabrir).
- **API:** `ProjectStore::{create, open, open_engine, create_engine, inspect, validate_file, ...}`; `impl Journal for ProjectStore` plugado no `Engine` (persiste antes de publicar).
- **Config:** WAL, `synchronous=FULL`, `foreign_keys=ON`, `busy_timeout=5s`, `quick_check` ao abrir.
- **Migrations:** `src/schema.rs` (`MIGRATIONS`); só para frente, backup `VACUUM INTO`, schema novo rejeitado.
- **Erros:** `StoreError` com `StoreErrorCode` (`PROJECT_CORRUPTED`, `UNSUPPORTED_SCHEMA_VERSION`, `STORE_BUSY`, ...).
- **Testes:** `tests/store.rs`, `schema.rs`, `crash.rs` (processo filho morto; feature `failpoints`, ativada só nos testes), `properties.rs` (`CAPIA_PROP_CASES`), `perf.rs` (`--release -- --ignored`).
- Fora do WASM (usa I/O e SQLite). Só esta camada pode depender de `rusqlite`/`getrandom`.
