# PROVENANCE — Política de proveniência e reutilização de terceiros

Obrigatória para todo o repositório (ADR-031). Objetivo: o CapIA nasce **apto a distribuição comercial/closed-source** sem dívida de licença escondida. Este documento é a **única** fonte da política e o **registro** de tudo que veio de fora.

## 1. Classificação (de `OPEN_SOURCE_AUDIT.md`)

| Classe | Significado | O que é permitido |
|---|---|---|
| `SAFE_TO_REUSE` | Licença permissiva sem obrigações relevantes | Copiar/adaptar código; registrar |
| `SAFE_WITH_OBLIGATIONS` | MIT, BSD, Apache-2.0, ISC etc. (aviso, atribuição, NOTICE) | Copiar/adaptar **com** registro e cabeçalho de atribuição |
| `REFERENCE_ONLY` | Licença incompatível ou dependência/risco que impede uso do código | Ler para **entender comportamento/ideias**; **nada** de código é copiado ou traduzido |
| `DO_NOT_USE` | Não comercial, copyleft forte, licença desconhecida/ausente, termos proibitivos | Nem copiar nem basear implementação no código; ideias de produto genéricas apenas |

**Lista de bloqueio (nunca no código nem no caminho crítico do produto):** GPL/AGPL/SSPL/BUSL, PolyForm Noncommercial e outras "non-commercial", licenças *source-available* restritivas (ex.: Remotion), qualquer fonte **sem licença** ou com licença ambígua, pesos/modelos/mídia/fontes sem licença individual verificada. LGPL somente como **biblioteca dinâmica** (ver ADR-032).

## 2. Regras

1. **Código efetivamente reutilizado** (copiado, adaptado, portado linha a linha) exige, **no mesmo commit**: (a) entrada no registro (§4); (b) cabeçalho no arquivo (§3); (c) preservação do aviso de copyright/licença quando exigido; (d) a licença incluída em `THIRD_PARTY_LICENSES` na distribuição.
2. **Somente `SAFE_TO_REUSE`/`SAFE_WITH_OBLIGATIONS`** podem ter código copiado. Para `REFERENCE_ONLY`/`DO_NOT_USE`: **nenhuma implementação pode ser copiada**, nem "traduzida" de linguagem, nem reescrita olhando linha a linha.
3. **Ideias e comportamentos** podem ser reimplementados sem transportar código incompatível. Sempre que um comportamento *observado* num projeto de terceiros fundamentar uma decisão, registrar a fonte consultada no §5 (rastreabilidade), mesmo sem cópia.
4. **Clean-room** para `REFERENCE_ONLY`/`DO_NOT_USE` e para qualquer área sensível: (i) quem lê descreve o comportamento num documento/teste **próprio** em termos do CapIA (ex.: cenários de `tests/acceptance`); (ii) quem implementa parte da descrição e dos testes, não do código-fonte; (iii) registrar quem leu o quê no §5. Para `DO_NOT_USE`, ninguém que implemente a área correspondente deve ter lido o código.
5. **Testes de terceiros** valem como código: não copiar literalmente sem registro; preferir cenários próprios com `provenance`.
6. **Dependências** (crates/npm/binários/DLLs/modelos/fontes): licença verificada **antes** de entrar; `cargo-deny` (Rust) e verificador de licenças (JS) no CI da Fase 1/M04 com lista de licenças permitidas; manifesto de licenças gerado a cada release (`THIRD_PARTY_LICENSES`).
7. **Ativos** (fontes, sons, stickers, LUTs, modelos de IA, mídia de teste): licença individual registrada; nenhum ativo vem de repositórios `REFERENCE_ONLY`/`DO_NOT_USE`.
8. **Spikes** (`/spikes`) são descartáveis, mas seguem a regra 2: **não** copiam código de terceiros para dentro do repositório; dependência de um checkout externo por *path* é permitida só para medição e **não** é versionada.
9. **Proibições de marca:** não usar nome, logotipo ou identidade visual de projetos de origem (ex.: "OpenCut", "CapCut").
10. **Quando em dúvida, tratar como `REFERENCE_ONLY`** e perguntar ao Product Owner.

## 3. Cabeçalho obrigatório em arquivo com código reutilizado

```
// Provenance: adapted from <repository> @ <commit/tag>, file <path/original>
// Original license: <SPDX id>. Copyright (c) <ano> <titular>  [aviso preservado quando exigido]
// Modifications: <resumo curto das alterações>
// Registry: docs/PROVENANCE.md §4 (#<id>)
```

## 4. Registro de código/ativos de terceiros efetivamente incorporados

| # | Repositório | Commit/tag | Arquivo original | Arquivo no CapIA | Licença | Alterações | Aviso de copyright preservado | Data |
|---|---|---|---|---|---|---|---|---|
| — | *(nenhum código de terceiros foi incorporado até a M03)* | | | | | | | |

Candidatos já avaliados (decisão arquivo a arquivo **na Fase 2**, só após testes dourados próprios; **não** copiados): `OpenCut-app/opencut-classic@cf5e79e9` — `rust/crates/compositor/src/shaders/blend.wgsl` e `rust/crates/masks/**` (MIT). Ver `docs/spikes/S6-compositor.md`.

## 5. Registro de fontes consultadas (rastreabilidade; **sem cópia**)

| Fonte | Commit | Classe | Para quê foi consultada | Resultado / onde se reflete |
|---|---|---|---|---|
| `OpenCut-app/opencut-classic` | `cf5e79e9` | SAFE_WITH_OBLIGATIONS | Comportamento de placement, snapping, ripple, retime, group move, keyframes; crates `time`, `gpu`, `compositor`, `effects`, `masks` | `tests/acceptance/timeline/*` (cenários próprios, campo `provenance`); `docs/spikes/S6-compositor.md`; nada copiado |
| `OpenCut-app/OpenCut` | `e668010` | REFERENCE_ONLY (scripts FFmpeg: SAFE_WITH_OBLIGATIONS) | Pipeline de build FFmpeg LGPL (`crates/media/setup`, `media-deps.yml`) | `docs/spikes/S3-ffmpeg-lgpl.md`; **rótulo de licença "LGPL-2.1" do manifesto não foi adotado** (a build é LGPL-3.0). Nada copiado |
| `MartinDelophy/ai-video-editor` | `36cf7b9` | SAFE_WITH_OBLIGATIONS (código) / DO_NOT_USE (pesos/modelos) | Contrato de comandos (`baseRevision`, ids de operação idempotentes, preview→apply, diff), MCP/WebMCP | ADR-029/ADR-030 (reimplementados do zero; ideias); nada copiado |
| `tjameswilliams/ai-video-editor` | `93f79bb` | **DO_NOT_USE** (PolyForm NC) | Auditoria M02 (arquitetura em alto nível) | **Nenhuma** implementação baseada nele; clean-room para Brain/Tool System |
| `itsjwill/vanta`, `abekyo/abekyo-editor` | `350b053`, `d442894` | DO_NOT_USE / REFERENCE_ONLY | Auditoria M02 | Nenhum código; padrão de schema+validate+render headless como inspiração de contrato (Fase 6) |
| Remotion | — | **DO_NOT_USE** | Licença verificada na auditoria | Fora do caminho do produto |
| BtbN FFmpeg-Builds `n8.1.3 lgpl-shared` | `autobuild-2026-09-23-14-55` | LGPL-3.0 (binário) | Inventário de codecs/licença (S3) | **Somente inspeção**; não é dependência do produto (ADR-032: build própria) |
| FFmpeg | tag `n8.1.3` (`1041abdc…`) | LGPL-2.1+ (config mínima) | Build mínima de prova (S3) | Origem da build própria; manifesto na release |

## 6. Procedimento de revisão (checklist por PR que traga código/ativo externo)

- [ ] Origem, commit/tag, arquivo original identificados
- [ ] Classe do repositório permite cópia (§1)
- [ ] Licença compatível com distribuição closed-source; obrigações listadas
- [ ] Cabeçalho (§3) no arquivo; entrada no registro (§4)
- [ ] Aviso de copyright/NOTICE preservado e incluído em `THIRD_PARTY_LICENSES`
- [ ] Dependências transitivas verificadas (`cargo-deny`/verificador JS)
- [ ] Testes próprios cobrem o comportamento (não só os do terceiro)
- [ ] Nenhum ativo/modelo/fonte sem licença individual
