# STATUS

**Última atualização:** 2026-10-02 · **Fase atual:** FASE 1 — Fundação (encerrada tecnicamente na M04; ver "Gates") · **Próxima:** FASE 2 — Motor

## Gates de fase (decisão do Product Owner, ADR-037)

```
Fase 2 — Motor (headless)     PODE iniciar com OD-1 aberto.
Fase 3 — Editor / Preview     NÃO pode iniciar sem OD-1 fechado (S1 medido em Windows).
```

OD-1 = presenter do preview na janela Tauri/WebView2. O pacote de medição está pronto: `tools/s1-preview-spike/` (`.\run.ps1`).

## Estado

| Item | Estado |
|---|---|
| Código de produto | Scaffold apenas (sem timeline/clips/comandos/render/IA). Código descartável em `/spikes` e `tools/s1-preview-spike` |
| Workspace | ✅ Cargo (4 crates do núcleo + shell Tauri) + pnpm (2 pacotes + app desktop); fronteiras verificadas por `tools/check-architecture.mjs` |
| CI | ✅ escrito (`.github/workflows/ci.yml`, validado com `actionlint`); **ainda não executado em runner real** (depende do push) |
| Licenças | ✅ `cargo-deny` + `tools/check-licenses.mjs` (detecta GPL/AGPL/non-commercial; não substitui auditoria humana) |
| Decisões abertas | **OD-1** (gate da Fase 3) · **OUTPUT-H264** (antes da entrega do Editor) · 8 decisões S7 `PROVISIONAL` |
| Spikes | S2–S7 concluídos (M03) · S1 = pacote pronto, aguardando execução em Windows |
| Git | Commits locais em `claude/happy-bardeen-feyypa`; ver "Blockers" |

## Decisões S7 — PROVISIONAL (aguardam aprovação do Product Owner)

A suíte `tests/acceptance/timeline` (ADR-036) já codifica estes comportamentos. **Nada na M04 depende deles.** Aprovar = manter; alterar = mudar cenários (dados), não código.

**D-S7-1 · Faixa de velocidade de clip** — `PROVISIONAL`
- *Proposto:* `set_clip_speed` aceita velocidade racional em **[1/100, 5]**; fora disso `OUT_OF_RANGE` (RTM-006..009).
- *Motivo:* limite finito evita durações absurdas e respeita o teto de 24 h de timeline.
- *OpenCut:* mesma faixa (0,01–5), mas ele **clampa** (inválida vira 1); o CapIA rejeita (ver D-S7-7).
- *Impacto:* validação do comando, slider da UI, erros acionáveis para a IA; mudar a faixa altera só cenários RTM.

**D-S7-2 · Inserção em track magnética no meio de um clip** — `PROVISIONAL`
- *Proposto:* `insert_clip` com `start` dentro de um clip → `NOT_ON_BOUNDARY`, salvo `split_at_insert=true` (PLC-009/010).
- *Motivo:* planos de IA/API precisam de semântica inequívoca; a intenção de drop (antes/depois/dividir) é decisão da UI.
- *OpenCut:* resolve na UI (placement/alvo de drop); não há erro equivalente no nível de comando.
- *Impacto:* contrato de `insert_clip`; a UI deve resolver a intenção antes de enviar; `generate_variants`.

**D-S7-3 · Ripple com escopo "sequence"** — `PROVISIONAL`
- *Proposto:* `RIPPLE_CONFLICT` se um clip de outra track destravada sobrepõe o trecho removido; tracks travadas ficam intactas (RPL-006..008).
- *Motivo:* não criar sobreposições nem cortar conteúdo de outras tracks implicitamente (V1 conservador).
- *OpenCut:* desloca só elementos a partir do corte (`rippleShiftElements`) e deixa os que atravessam — pode gerar overlap.
- *Impacto:* o usuário pode precisar dividir/travar antes; a alternativa estilo Premiere (cortar em todas as tracks) muda comando e UX.

**D-S7-4 · Conversão px→frames do limiar de snap** — `PROVISIONAL`
- *Proposto:* `floor(px / px_por_s × fps)` em frames inteiros (SNP-014..016).
- *Motivo:* inteiro em frames; o limiar nunca excede o orçamento em pixels.
- *OpenCut:* retorna ticks fracionários (`px / pps × TICKS_PER_SECOND`).
- *Impacto:* sensação do snap em zoom baixo (limiar 0 → só frame exato); trivial de ajustar.

**D-S7-5 · Desempate no snap** — `PROVISIONAL`
- *Proposto:* mesma distância → playhead > marcador > bordas de clip/início da sequence; depois o menor tempo (SNP-006..008).
- *Motivo:* resultado determinístico e previsível (testável).
- *OpenCut:* mantém o primeiro alvo encontrado (ordem de iteração).
- *Impacto:* qual alvo "ganha" em empates; afeta UX e testes de snapping.

**D-S7-6 · Keyframes fora do trecho visível** — `PROVISIONAL`
- *Proposto:* trim **não apaga** keyframes; reestender restaura a animação; split cria keyframe de fronteira interpolado nas duas metades (KF-007..013).
- *Motivo:* edição não destrutiva; coerente com ADR-025 (keyframes no tempo do conteúdo).
- *OpenCut:* `clampAnimationsToDuration` descarta keyframes além da nova duração (mantém só a fronteira). O split com keyframe de fronteira é equivalente.
- *Impacto:* tamanho dos dados e UI de "keyframes inativos"; alternativa = descartar como o OpenCut.

**D-S7-7 · Valor fora do range numa propriedade** — `PROVISIONAL`
- *Proposto:* comandos **rejeitam** (`OUT_OF_RANGE`); a UI faz clamp antes de enviar (KF-005/006, RTM-006/007).
- *Motivo:* um plano de IA nunca é alterado silenciosamente.
- *OpenCut:* clampa parâmetros numéricos.
- *Impacto:* mensagens de erro das tools de IA; registro de propriedades com faixas.

**D-S7-8 · Arredondamento de duração ao mudar velocidade** — `PROVISIONAL`
- *Proposto:* `round-half-up` para frame inteiro, **mínimo 1 frame** (RTM-003..005).
- *Motivo:* invariante de alinhamento a frame (ADR-007).
- *OpenCut:* arredonda a ticks inteiros (não a frames).
- *Impacto:* duração final ±0,5 frame; consistência preview/export.

## OUTPUT-H264 — risco técnico explícito (NÃO resolvido na M04)

Antes da **entrega do Editor** (saída da Fase 3) é preciso um caminho **confiável** de exportação **MP4/H.264 no Windows**:

- **Encoder de hardware** quando disponível (NVENC/AMF/QSV);
- **Media Foundation / FFmpeg** quando adequado (`h264_mf`, builds LGPL — ADR-032);
- **Fallback por software legal e de qualidade aceitável**. Sem x264/x265 GPL no build padrão.

*Sabido (S3):* as builds LGPL trazem NVENC/AMF/QSV/MF; o fallback por software disponível é OpenH264 (**só Constrained Baseline**; PSNR 46,9 dB vs 53,0 dB do x264 no mesmo bitrate em conteúdo **sintético**); o binário OpenH264 compilado do fonte não tem a cobertura de patentes da Cisco.
*Desconhecido:* qualidade/velocidade em vídeo real; disponibilidade de encoder de hardware na máquina mínima (iGPU); comportamento do Media Foundation; política de download/licença do binário OpenH264; patentes H.264/HEVC/AAC (decisão jurídica/comercial).
*Responsável:* Product Owner (com apoio jurídico para patentes). *Sugestão:* spike dedicado em Windows (mesma sessão do S1). *Estado:* **ABERTO**.

## Missões

| Missão | Resultado |
|---|---|
| M01 — Fundação arquitetural | ✅ docs + `.gitignore` |
| M02 — Auditoria open-source | ✅ `OPEN_SOURCE_AUDIT.md`; estratégia C |
| M03 — Fechamento e spikes S1–S7 | ✅ S2–S7 medidos; ADR-029..036; `PROVENANCE.md`; `tests/acceptance` (108 cenários) |
| **M04 — Scaffold, CI e preparação** | ✅ workspace Rust+TS, shell Tauri, CI, verificação de licenças/arquitetura, pacote S1 para Windows, ADR-037/038 |

## Validação M04 (saída real dos comandos, container Linux; toolchain Rust 1.97.0, Node 22, pnpm 10.28)

| Verificação | Resultado |
|---|---|
| `cargo fmt --all -- --check` | ✅ |
| `cargo check --workspace` (inclui o shell Tauri) | ✅ |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ sem avisos |
| `cargo test --workspace` | ✅ **12 testes** (time 5 · model 2 · commands 2 · project 2 · desktop 1) |
| WASM: `cargo check --target wasm32-unknown-unknown -p capia-time -p capia-model -p capia-commands` | ✅ (ADR-016) |
| `pnpm typecheck` · `lint` · `format:check` | ✅ ✅ ✅ |
| `pnpm test` | ✅ **13 testes** (engine-bindings 10 · editor-ui 2 · desktop 1) |
| `pnpm test:tools` | ✅ **12 testes** (arquitetura 7 · licenças 5) |
| `pnpm build` (Vite) | ✅ |
| `pnpm desktop:build` (`tauri build --no-bundle`, release) | ✅ binário de 7,5 MB |
| **App aberto de verdade** (Xvfb, Linux/WebKitGTK) | ✅ processo vivo, janela "CapIA" 1280×720; screenshot mostra o título e a linha "Engine capia-engine v0.0.0 · API 1 · schema documento 1 / comandos 1", que atravessa IPC Tauri → `capia-project` → `engine-bindings` → React |
| `pnpm check:arch` | ✅ 5 crates Rust e 3 pacotes JS dentro das fronteiras, sem ciclos |
| `pnpm check:licenses` | ✅ 227 pacotes JS: 224 permitidos, **0 bloqueados**, 3 para revisão (`lightningcss`, MPL-2.0, só no build do Vite) |
| `cargo deny check licenses bans sources` | ✅ grafo Windows: 246 crates de terceiros, todos permissivos; **5 MPL-2.0** para revisão (`cssparser`, `cssparser-macros`, `dtoa-short`, `option-ext`, `selectors`, via Tauri) |
| `cargo deny check advisories` (informativo no CI) | ⚠️ 1 aviso `unmaintained`: `proc-macro-error` (via `glib-macros`/GTK, **só Linux**; ausente no grafo Windows) |
| Detector de licenças prova que barra | ✅ workspace descartável com crates GPL-3.0, AGPL-3.0 e PolyForm-Noncommercial → `rejected`, exit 4 |
| `actionlint` no `ci.yml` | ✅ |
| Pacote S1: cross-compile **e link** para `x86_64-pc-windows-gnu` | ✅ PE32+ de 32 MB; `clippy` sem avisos |
| Pacote S1: parser real do PowerShell 7.6 em `run.ps1` + `selftest.ps1` | ✅ 0 erros de sintaxe; 8 verificações de lógica |

### O que NÃO foi validado

- **Build/execução no Windows** (MSVC + WebView2): o app só rodou em Linux. O CI do Windows existe mas **nunca rodou em runner real**; `gitleaks-action` e `cargo-deny-action` idem.
- O harness S1 **nunca foi executado** (nem em Windows, nem em Wine); só compilado/cross-linkado e com a lógica de correlação testada. Não compilado com MSVC.
- `tauri build` gerou apenas o binário (sem instalador, de propósito).
- O ícone é placeholder.

## Métricas da M04

- **Marcos:** scaffold (4 crates + shell + 3 pacotes TS) · CI (4 jobs) · 2 ferramentas de política com 12 testes · pacote S1 completo · 2 ADRs · gates de fase e decisões S7 documentados.
- **Testes:** 12 Rust + 13 TS + 12 de ferramentas + 8 do selftest PowerShell = **45 automatizados**, todos verdes.
- **Auditoria de consistência entre documentos (script):** 257 verificações (ADRs citadas existem, caminhos, seções, estado de OD-1..4, termos obsoletos, as 8 decisões S7, texto dos gates); 0 problemas reais; 9 apontamentos esperados (caminhos de *outros* repositórios citados no audit e `packages/ui-timeline`, que nasce na Fase 3).
- **Riscos eliminados:** workspace compila/linka e roda ponta a ponta no Linux; Tauri 2 + wgpu 29 + webview2-com compilam para Windows; núcleo comprovadamente livre de Tauri/IA/render e compilável para WASM; detector de licenças provado; sintaxe e lógica do runner S1 validadas.
- **Retrabalho:** typescript 7 → fixado em 6.0.3 (peer do typescript-eslint); `cargo fmt`/clippy ajustados 2×; erros de config do `cargo-deny` (2); `node --test` com diretório; 2 defeitos no `run.ps1` achados em revisão (`$args` automático, aspas em caminhos com espaço); 1 comando de limpeza bloqueado pela proteção do ambiente e 1 `pkill` que matou o próprio shell (refeitos).
- **Custo aproximado:** ~0,22 M tokens de contexto nesta missão (estimativa); custo em dinheiro **não informado**.

## Blockers

1. **Push ao GitHub (403)** — persistente desde a M01; testado de novo no início da M04 (antes de qualquer alteração): `Claude doesn't have GitHub access to paulodirectresponse-jpg/CapIA`. Correção: reconectar em https://claude.ai/connect-github e instalar o Claude GitHub App no repositório. O resultado do push final desta missão está no relatório da sessão.
2. **S1 em Windows** — gate da Fase 3 (não bloqueia a Fase 2).
3. **Primeira execução do CI** — depende do push.

## Notas

- Leia `CLAUDE.md` e este arquivo primeiro; depois os docs da missão.
- Nenhum código de terceiros incorporado (`docs/PROVENANCE.md` §4 vazio).
- Ao concluir uma missão, atualize esta página.
