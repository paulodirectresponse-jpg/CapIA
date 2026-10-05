# Migração e compatibilidade

Política única para **projeto** (`.capia`), **AppDb** (preferências/configuração de IA do usuário, `app.db`) e **banco do servidor** (`server.db`): migrations **explícitas, numeradas e só para frente**; cada uma numa transação; **backup antes de migrar**; schema **mais novo** que o software é **recusado sem tocar no arquivo** (`UNSUPPORTED_SCHEMA_VERSION`). Erros do store são sempre `StoreError` estruturado (nunca SQLite bruto). Registro das decisões: ADR-042..044 e ADRs das Fases 4–6 em `docs/DECISIONS.md`.

## Versões

### Projeto `.capia` (`PRAGMA user_version` = schema; `crates/capia-store/src/schema.rs`)

| Schema | Migration | Conteúdo | Aditiva? |
|---|---|---|---|
| 1 | initial schema | meta, `schema_migrations`, histórico, eventos, operações aplicadas, resultados de commit, snapshots (ADR-042) | — |
| 2 | media catalog | catálogo de assets (hash, caminho, metadados) e `asset_events` (M07) | sim |
| 3 | media jobs | `jobs`, `import_tickets`, impressão rápida de asset (M08) | sim |
| 4 | intelligence | `ai_records`, `ai_usage` (Fase 4) | sim |
| 5 | autonomy | runs, stages, eventos, efeitos colaterais, proveniência, memória, livro de orçamento (Fase 5) | sim (projetos v4 migram sem perda) |

Versão corrente do projeto: **5** (`CURRENT_SCHEMA_VERSION`).

### AppDb (`app.db`)
Schema **1** (`APP_SCHEMA_VERSION`, migration “app kv”): armazenamento chave-valor do app (configuração global, incluindo a de IA). **Nunca** guarda segredos — só referências de credencial; o valor fica no cofre do SO.

### Banco do servidor (`server.db`, assinatura “CAPS”)
Schema **1** (`SERVER_SCHEMA_VERSION`; `crates/capia-store/src/serverdb.rs`): `api_tokens` (só hash SHA-256), `projects`, `idempotency`, `audit`, `events`, `webhooks` (só `secret_ref`), `deliveries`, `exports`. Mesma disciplina de migrations do AppDb.

## Regras

1. **Forward-only.** Não existe migração para trás. Migration que invalida o histórico declara isso e reinicia o undo stack arquivando o journal (ADR-013).
2. **Backup automático.** Antes de migrar um arquivo existente com migrations pendentes o store cria `<arquivo>.capia.v<N>.bak` (`N` = schema antigo) por `VACUUM INTO` (consistente, inclusive com WAL). Se o nome existir, acrescenta `-<timestamp>`.
3. **Falha = arquivo preservado.** `MIGRATION_FAILED` deixa o arquivo na última versão concluída; cada migration é uma transação `IMMEDIATE`.
4. **Schema futuro recusado** sem modificar o arquivo. Não há modo “somente leitura” (exigiria garantia de compatibilidade de leitura que não temos).
5. **Cache descartável.** `<proj>.capia-cache/` nunca é migrado (é recriado; as chaves de cache incluem a versão do formato, ex.: `capia-cache-v2`). `<proj>.capia-media/` é durável e não tem schema próprio (arquivos endereçados por hash).
6. **Mudar o schema ⇒** nova migration + teste de migração a partir da versão anterior + ADR.
7. **Compatibilidade entre versões do app:** um projeto aberto e migrado por uma versão nova **não** abre numa versão antiga. Troque de versão restaurando o `.bak`/backup.

## Matriz de compatibilidade a manter (Fase 6 §31)

| Projeto de origem | Esperado | Verificação |
|---|---|---|
| Era Fase 3 (schema ≤ 3) | abre, migra até 5 com `.bak`, sem perda | testes de schema/migração em `capia-store/tests/` (CI) |
| Schema 4 (Fase 4) | migra para 5 sem perda | `autonomy_store.rs::a_schema_4_project_migrates_and_keeps_its_content` (CI) |
| Schema 5 (atual) | abre sem migração | testes de store (CI) |
| Schema > 5 | recusa sem tocar | testes de schema (CI) |
| Projetos reais “antigos” do usuário | **externo**: abrir projetos representativos de versões anteriores numa máquina limpa | executar manualmente no beta; reportar via `beta-feedback` |

## Rollback e backup — estratégia

- **Antes de atualizar:** backup dos projetos (arquivo `.capia` + `-media/` + mídia original; ver [`docs/user/06-backup-e-recuperacao.md`](user/06-backup-e-recuperacao.md)).
- **Release (equipe):** fixe os schemas suportados no changelog; teste a migração do RC anterior para o atual (`tools/phase6-acceptance/update/`); não publique RC cujo schema seja mais novo sem migration e teste.
- **Rollback do app:** reinstalar a versão anterior **e** restaurar `.bak`/backup do projeto. O atualizador (Fase 6) mantém a versão anterior recuperável em falha de download/assinatura/saúde; ele **não** desfaz migrações de projeto — por isso o backup.
- **Servidor:** `server.db` v1 é novo no 0.6.0-rc.1; tokens e webhooks são criados após a atualização. Em caso de dúvida, pare o servidor (`capia-server stop`), copie a pasta de dados e revogue/rotacione tokens ao restaurar.
