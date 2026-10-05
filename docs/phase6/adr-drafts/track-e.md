# ADR (rascunho) — Track E: documentação gerada, evidência externa honesta e processo de release

> Rascunho para o integrador incorporar a `docs/DECISIONS.md` com o próximo número livre. Não editar `DECISIONS.md` nesta frente.

## Contexto

A Fase 6 precisa de documentação de API que não divirja do código, de um pacote de aceitação que **nunca fabrique** resultados externos (certificado real, máquinas Windows limpas, usuários de beta, provedores reais) e de um processo de release repetível.

## Decisões

1. **Documentação de API derivada do catálogo único (ADR-102).** `docs/api/rest-reference.md`, `mcp-tools.md`, `openapi.json` e a matriz de scopes são **gerados** por `tools/docs/gen-api-docs.mjs` a partir do JSON do catálogo (`capia-server catalog`, ou o fixture `tools/docs/fixtures/catalog.json` transcrito mecanicamente do Rust). O gerador **não** interpreta Rust. `--check` falha se a documentação estiver defasada. Textos escritos à mão (convenções, auth, webhooks) ficam fora dos blocos gerados; só a matriz de scopes é injetada entre marcadores em `auth-and-scopes.md`.
2. **Vocabulário único de estado de aceitação:** `passed | failed | pending_external | not_available`. `not_available` = o passo não pôde rodar aqui (binário/teste de outra frente, variável, `CAPIA_P6_HEAVY`) e **nunca** conta como aprovado. O agregado é o **pior** estado; só “tudo `passed`” é `PHASE 6 COMPLETE`; suíte `not_available` impede declarar engenharia completa (`INCOMPLETE`).
3. **Evidência externa só por arquivo real validado.** Validadores (`clean-machine`, `beta-feedback`, `update`, `installer`, paridade) respondem `pending_external` sem arquivo ou com modelo (`"template": true`), `rejected` para malformado/inconsistente/que registra falha, `partial` para evidência bem formada mas incompleta (ex.: só uma versão do Windows; artefato não assinado; update assinado não executado por falta de certificado) e `accepted` só com todas as regras. Exigem atestados explícitos do executor e checagens de plausibilidade (datas não futuras, builds do Windows coerentes, hashes, igualdade recalculada). Um resultado negativo honesto é evidência válida que reprova o gate.
4. **Gate de beta:** “nenhum Blocker/Critical aberto para o RC”: aberto = `open`, `wontfix` ou `fixed` em versão posterior ao RC; `wontfix` nunca fecha Blocker/Critical. Mínimo de 5 usuários externos é **escolha de produto documentada**, parametrizável (`--min-users`); usuários internos não contam.
5. **Passos pesados** (compilam Rust, build do desktop) só rodam com `CAPIA_P6_HEAVY=1`; `steps.json` é declarativo e aponta para nomes **esperados** de testes de outras frentes, tratados como `not_available` enquanto não existirem.
6. **Exemplos não são produto:** vivem em `examples/`, sem dependência de crates/pacotes, com `.mjs` no lint/format; testados contra servidores falsos e com vetores HMAC independentes; passam a valer como integração real apenas quando executados no item `external-flow`.
7. **Documentos de release** (`RELEASE.md`, `CHANGELOG.md`, `KNOWN_ISSUES.md`, `MIGRATION_COMPAT.md`) são verificados por `installer/check-release.mjs` (versão única nas três fontes; seções e checklist presentes).

## Alternativas

- Escrever a referência da API à mão (diverge do código); parsear Rust por regex (frágil); `pending` genérico sem distinguir “não pôde rodar” de “depende de humano” (esconde lacunas de integração); tratar evidência faltante como “skipped/ok” (viola a regra de não fabricar).

## Consequências

- Mudar o catálogo exige regenerar `docs/api` (CI pode rodar `pnpm check:docs`).
- O estado final reportado pelo agregador é conservador por construção; fechar a Fase 6 exige as evidências externas listadas em `docs/phase6/IMPL_DOCS_ACCEPTANCE.md`.
- Os nomes esperados de testes de outras frentes em `steps.json` precisam ser ajustados pelo integrador quando divergirem.

## Evidência

`tools/docs/gen-api-docs.test.mjs`, `tools/phase6-acceptance/**/*.test.mjs`, `examples/**/*.test.mjs`, `tools/sample-project/make-sample.test.mjs` (todos em `pnpm test:tools`).
