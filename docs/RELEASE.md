# Processo de release

Checklist para cortar um release candidate (RC) ou release. **Regra de ouro:** nada é marcado como feito sem evidência; o que depende de certificado, máquina limpa física, usuários reais ou decisão jurídica fica **externo** (`pending_external`) e é declarado como tal no changelog e no `KNOWN_ISSUES`.

Estado final possível (`node tools/phase6-acceptance/run-all.mjs`): `FAILURES` · `INCOMPLETE — …` · `PHASE 6 ENGINEERING COMPLETE — EXTERNAL RELEASE ACCEPTANCE PENDING` · `PHASE 6 COMPLETE` (só com todas as evidências externas reais).

## 1. Versão (fonte única)

- [ ] Defina `X.Y.Z[-rc.N]` e atualize **juntas** as três fontes: `[workspace.package].version` do `Cargo.toml`, `version` do `package.json` raiz e `version` de `apps/desktop/src-tauri/tauri.conf.json` (e os `package.json` dos pacotes, se versionados).
- [ ] `node tools/phase6-acceptance/installer/check-release.mjs --versions --expect X.Y.Z-rc.N` passa (a versão aparece igual em `server.info`, sobre/about e diagnóstico).

## 2. Changelog e documentação

- [ ] `CHANGELOG.md`: seção `## [X.Y.Z-rc.N] - AAAA-MM-DD` honesta (o que existe, o que é externo); `[Unreleased]` esvaziado/atualizado.
- [ ] `docs/KNOWN_ISSUES.md` revisado (pendências externas e limitações reais; sem “resolvido” sem evidência).
- [ ] `docs/api/` regenerado do catálogo real: `capia-server catalog | node tools/docs/gen-api-docs.mjs --catalog -` e `node tools/docs/gen-api-docs.mjs --check` verde.
- [ ] `node tools/phase6-acceptance/installer/check-release.mjs --docs --version X.Y.Z-rc.N` passa.
- [ ] `docs/STATUS.md` e `docs/ROADMAP.md` atualizados pelo integrador (sem marcar caixas externas).

## 3. Migração e compatibilidade

- [ ] `docs/MIGRATION_COMPAT.md` reflete os schemas (projeto, AppDb, `server.db`); se mudou schema: migration + teste + ADR.
- [ ] Migração a partir do RC anterior e dos schemas 3/4/5 testada (CI) com backup `.bak` conferido.
- [ ] Projetos representativos de versões anteriores abertos manualmente (externo; registrar no feedback do beta).

## 4. CI (mesmo HEAD, todos os jobs)

- [ ] `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- [ ] `pnpm lint && pnpm format:check && pnpm typecheck && pnpm test && pnpm test:tools && pnpm build`
- [ ] `pnpm check:arch && pnpm check:licenses && cargo deny check licenses bans sources`
- [ ] E2E Linux e Windows (incl. autonomia no app Tauri), gitleaks no histórico.
- [ ] `node tools/phase6-acceptance/run-all.mjs` sem `FAILURES`; resumo (`target/phase6-acceptance/summary.json`) anexado ao release.

## 5. Instalador

- [ ] `pnpm desktop:build` + bundle Windows (NSIS/MSI) gerado em CI; FFmpeg **LGPL** embutido (build própria, dinâmica, sem GPL/nonfree — ADR-032); tratamento do WebView2 documentado.
- [ ] Atalhos e desinstalador; **desinstalar não apaga projetos**; primeira execução sem chave de IA.
- [ ] Artefatos de desenvolvimento/teste **rotulados** como não assinados.

## 6. Assinatura (externo até haver certificado)

- [ ] Certificado de assinatura de código **real** disponível apenas como **segredo do CI** (nunca no repositório, em logs ou em artefatos); pipeline assina instalador, executáveis e artefato do atualizador.
- [ ] `signtool verify /pa /v` / `Get-AuthenticodeSignature` = `Valid`, com **carimbo de tempo**; evidência em `target/phase6-acceptance/evidence/installer-results.json` validada por `installer/validate.mjs`.
- [ ] Sem certificado: o release é **RC não assinado**; o gate fica `pending_external` e o changelog diz isso. Nunca “simular” assinatura.

## 7. Hashes, SBOM e licenças

- [ ] `SHA256SUMS` dos artefatos; `inspect-installer.mjs` confere tamanho/hash/formato; hashes publicados na página do release.
- [ ] **SBOM** (CycloneDX ou SPDX) e relatório de licenças gerados; `cargo deny` e `pnpm check:licenses` verdes; avisos de terceiros e informações do FFmpeg no instalador (`docs/PROVENANCE.md`).
- [ ] Nenhum `REFERENCE_ONLY`/`DO_NOT_USE` copiado (PROVENANCE).

## 8. Atualização e rollback

- [ ] Manifesto e artefato de atualização **assinados**; cliente verifica a assinatura antes de instalar; sem downgrade acidental; sem atualizar no meio de uma Run ativa (adia ou faz checkpoint).
- [ ] Atualização RC→RC e os cenários de rollback (download interrompido, artefato corrompido, assinatura inválida, falha de saúde, Run ativa) com evidência validada por `update/validate.mjs` (externo: Windows real).
- [ ] **Estratégia de rollback do release:** manter o instalador do RC anterior publicado; o atualizador mantém a versão anterior recuperável; migrações são só para frente ⇒ instruir **backup dos projetos antes de atualizar** (`docs/user/06-backup-e-recuperacao.md`); para retirar um release ruim, despublicar o manifesto de atualização (os clientes ficam na versão atual) e publicar um RC corretivo — nunca “re-assinar por cima” do mesmo número de versão.

## 9. Segurança e privacidade

- [ ] Suíte de segurança da API (`tools/phase6-acceptance/security/`) sem falhas; sem vazamento de canário em REST, MCP, webhook, diagnóstico e relatório de falhas.
- [ ] Relatório de falhas **opt-in e desligado por padrão** (verificado); pacote de diagnóstico sem chaves/mídia.
- [ ] Segredos de release (assinatura, webhooks, providers) só no secret store do CI.

## 10. Aceitação

- [ ] **Máquina limpa** Windows 10 22H2 e 11 (`clean-machine/validate.mjs` = `accepted`) — externo.
- [ ] **Beta**: feedback real validado; gate “nenhum Blocker/Critical aberto para o RC” (`beta-feedback/validate.mjs`) — externo.
- [ ] **Paridade** UI×REST×MCP e fluxo canônico (`external-flow`) — externo/servidor real.
- [ ] Desempenho em projeto grande (≥ 30 sequences, ≥ 5.000 clips) sem regressão material (suíte da frente de desempenho).
- [ ] Pendências herdadas das Fases 3/4/5 e a decisão H.264/AAC listadas em `KNOWN_ISSUES` com seu estado.

## 11. Publicar

- [ ] Tag `vX.Y.Z-rc.N` no commit verde; notas de versão = seção do changelog; anexos: instaladores, `SHA256SUMS`, SBOM, relatório de licenças, `summary.json` da aceitação, `KNOWN_ISSUES.md`.
- [ ] Anunciar **apenas** o que está verificado; itens externos sempre como pendentes.
- [ ] Pós-release: coletar feedback (`beta-feedback`), triagem por severidade (Blocker, Critical, Major, Minor, Cosmetic); Blocker/Critical abertos bloqueiam a promoção do RC a release.
