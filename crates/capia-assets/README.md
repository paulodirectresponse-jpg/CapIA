# capia-assets

Domínio de assets (ADR-046..049): identidade, hash em *streaming*, import, disponibilidade, relink e cache derivado. Sem SQLite e sem Command Engine (quem persiste é `capia-store`; quem orquestra é `capia-project`).

- **Identidade:** `AssetId` = `ast_` + 32 hex do `content_hash`; `content_hash` = `sha256:<64 hex>` do arquivo **inteiro**, em blocos de 1 MiB (nunca na memória). O caminho nunca identifica nada.
- **Import:** `prepare_import(path, project_dir, &dyn MediaProbe, now)` — verifica (arquivo regular, não vazio, caminho válido) → hash → probe → confere que o arquivo não mudou no meio → `AssetRecord`. **Não escreve nada.**
- **Localização:** `AssetLocation { path, path_bytes_hex?, relative? }` (caminho exato do SO + relativo ao projeto sempre com `/`); `resolve_candidates` ordena relativo → absoluto → aliases.
- **Disponibilidade:** `quick_status` (só `metadata`, O(1)) e `verify_content` (recalcula o hash): `online | offline | modified`. mtime nunca decide.
- **Relink:** `check_relink` aceita só o mesmo conteúdo; senão `ASSET_HASH_MISMATCH` com os dois hashes em `details`.
- **Cache:** `CacheKey` (SHA-256 de conteúdo+operação+parâmetros+produtor), `CacheDir` (escrita atômica, `clear()` seguro), `ensure_thumbnail`.
- **Testes:** `tests/assets.rs` (probe sintético `testing::StaticProbe`), `tests/real_media.rs` (ffprobe/ffmpeg reais).
