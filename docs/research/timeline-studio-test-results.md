# Timeline Studio — resultados de teste (evidência executável)

> Fase 1 da nova estratégia. Candidato: `MartinDelophy/ai-video-editor` ("Timeline Studio").
> **SHA testado: `36cf7b9`** (branch `main`, clone separado fora do repositório CapIA; nenhum código de terceiros foi importado).
> Ambiente: Linux container, Node 22 + npm, Chromium do Playwright (**sem H.264**), ffmpeg do sistema para render headless.
> Legenda: PASS / PARTIAL / FAIL / UNVERIFIED_ON_CURRENT_ENVIRONMENT. Nada aqui é PASS por leitura de código.
> Scripts de teste (Playwright, `playwright-core` contra `vite preview`; polyfill de `document.modelContext` para chamar as tools WebMCP) ficaram no scratchpad da sessão, não no repositório.

## 1. Instalação / build

| Passo | Resultado | Evidência |
|---|---|---|
| `npm ci` | PASS **com contorno** | O `package-lock.json` aponta para `registry.npmmirror.com` (bloqueado pelo proxy) → travou >30 min. Funcionou com `npm ci --ignore-scripts --no-audit --no-fund --replace-registry-host=always` (32 s). |
| `npm run lint` | PASS | exit 0 |
| `npm run typecheck` | PASS | exit 0 |
| `npm run build` | PASS | Vite build ok |
| Testes automatizados do repo | **NÃO EXISTEM** | `package.json` não tem script `test`; o CI só roda guarda de tamanho + lint + typecheck + build. |

## 2. Teste funcional do editor (UI real, Playwright)

| Item | Resultado | Detalhe |
|---|---|---|
| A–D criar projeto / importar vídeo, imagem, áudio | PASS | projeto padrão + 4 assets via `input[type=file]` |
| E inserir mídia (arrastar da biblioteca) | PASS | clips 0–6,01 s e 6,01–12,02 s |
| F inserir áudio em faixa de áudio por drag | FAIL (automação) | drag HTML5 expirou; o áudio **entra** por drop na área `.tracks` e pelo agente (`asset.insert`). Tratar como PARTIAL. |
| Multi-track | PARTIAL | faixas **fixas por tipo** (visuals, overlays, áudio/voiceover, música, legendas, stickers, marcadores). Não há faixas livres. |
| I split no playhead | PASS | 0–2,00 / 2,00–6,01 |
| H trim direita / esquerda (handles) | PASS | 2,00→1,78 ; sourceIn 2,00→2,18 |
| O duplicar | PASS | 3→4 |
| G mover (reordenar na faixa principal) | PASS | ordem alterada |
| J apagar | PASS | 4→3; na faixa principal **sempre** fecha o buraco |
| M/N undo / redo (teclado) | PASS | 3→4→3 |
| K ripple | PARTIAL | toggle existe; na faixa principal é implícito (contígua). |
| Copiar/colar (Ctrl+C/V) | FAIL/UNSUPPORTED | 3→3 |
| Q transform (posição, escala, rotação, opacidade) | PASS | valores confirmados no projeto salvo |
| T keyframe (escala no playhead) | PASS | confirmado no projeto salvo |
| U animação de clip "In: Fade" | PASS | |
| R velocidade (editor de curva) | PASS (UI) | valor conferido no projeto salvo |
| S mudo no clip | PASS | |
| W separar áudio da fonte | PARTIAL | resultado inconsistente na automação (contadores de faixa não mudaram) |
| P legenda manual pela faixa de legendas | FAIL (automação) | `+Segment` não criou legenda; legenda **via agente** (`caption.add`) = PASS. |
| Texto / stickers / filtros / efeitos | PARTIAL | painéis existem (capturas `ui_c-*`, `uif-filters`); aplicação ponta a ponta não confirmada. |
| Transições | PARTIAL | existem no modelo; UI não exercitada de ponta a ponta |
| Volume | PASS (agente) | `clip.set_property volume` |
| Snapping | UNVERIFIED | existe no código; não medido |
| X salvar | PASS | pacote `.timeline` de 474 522 bytes |
| Y fechar/reabrir | PASS | contexto novo do navegador, storage vazio, import do pacote restaurou timeline, keyframes, animação e transform |
| Y continuar editando após reabrir | PARTIAL | duplicar manual: ver nota; split via agente: **FAIL** `BROWSER_EDIT_COMPLEX_TIMING` ("Edit clips with unsupported speed curves or transitions in the editor") — o agente se recusa a editar clips com curva/transição/keyframes. |
| Z exportar | PASS (WebM/VP9, ~1,9 MB em 13 s); **MP4/H.264 UNVERIFIED_ON_CURRENT_ENVIRONMENT** (Chromium do Playwright não tem H.264). |

## 3. Persistência

- Formato: **zip `.timeline`** com `project.json` (`format: "timeline-studio-archive"`, **versão 3**) + mídia embutida + `commandState{revision, appliedOperationIds}`.
- Portabilidade: PASS (autocontido; reaberto em navegador limpo).
- Versionamento: há campo de versão; **não há cadeia de migrações testada** (sem testes).
- Mídia **dentro** do projeto (viola o princípio local-first / "mídia nunca entra no arquivo de projeto" do CapIA; projetos grandes ficam enormes).
- Risco de corrupção: escrita do zip inteira em memória/download, sem journal, sem transação, sem checksum de mídia. IDs de clip incluem timestamp+aleatório; asset ids são UUID por importação (não por conteúdo → sem dedup).

## 4. Command engine / camada de agente (executado)

CLI headless (`npm run agent` = `scripts/timeline-command.mjs`), motor puro `applyCommandPlan`:

| Capacidade | Resultado |
|---|---|
| validate → preview → diff → apply | PASS |
| revisão base obsoleta (`baseRevision` antigo) | PASS — rejeitado |
| idempotência (mesmo `operationId` reaplicado) | PASS — ignorado |
| atomicidade (plano com op inválida no meio) | PASS — nada aplicado |
| erros estruturados com código | PASS |
| render headless por plano | PASS **sem legendas**; com legendas → `UNSUPPORTED_RENDER_FEATURE` |
| render usa `libx264`/`aac` do ffmpeg do sistema | CONFIRMADO em `projectRenderPlan.js` — **conflita com ADR-032 do CapIA** |
| MCP stdio (`skills/edit-timeline-studio/mcp/server.mjs`) | PASS — `initialize`, `tools/list` = 8 tools, `timeline_project_inspect` retorna projeto válido |
| WebMCP no navegador (21 tools sobre o estado vivo) | PASS — inspect projeto/faixa/clip, `visual.insert` (vídeo e imagem), `asset.insert` (áudio), split, trim, `visual.reorder`, `visual.delete`, `caption.add/update`, `clip.set_property volume`, `project.set_ratio`, `marker.add`; todas com `previewId`/`transactionId`/`stateToken` |
| Humano e IA operam **a mesma** timeline? | **SIM** (ponto forte): WebMCP altera o estado React vivo e o histórico de undo é compartilhado (verificado). |
| Lacunas | transformar clip da faixa principal pelo agente: não disponível; ops recusadas em clips com tempo complexo; cobertura de 26 operações, só reducers para as faixas fixas. |

## 5. Desempenho (sem otimização; nós = clips)

Motor puro (Node, `bench-engine`):

| n | split (ms) | 5 ops (ms) | JSON (MB) | heap (MB) |
|---|---|---|---|---|
| 100 | 2,4 | 1,7 | 0,02 | 79 |
| 500 | 3,7 | 4,4 | 0,10 | 81 |
| 1000 | 6,9 | 6,9 | 0,20 | 85 |
| 5000 | 36,7 | 42,5 | 1,00 | 89 |
| 10000 | 89,0 | 90,0 | 2,00 | 109 |

O motor é bom (≈ linear). **A UI não é**: timeline sem virtualização (um nó DOM por clip).

| n | load (ms) | nós DOM | heap após (MB) | scroll p50 / p95 (ms/quadro) |
|---|---|---|---|---|
| 100 | 774 | 2 458 | 46 | 16,7 / 66,7 |
| 500 | 2 290 | 10 498 | 63 | 133 / 433 |
| 1000 | 3 393 | 20 498 | 84 | 117 / 383 |
| 5000 | 15 935 | 70 498 | 195 | 100 / 167 |
| 10000 | 29 361 | 140 498 | 229 | — ; split via agente 19,99 s |

Conclusão: UI inutilizável acima de ~500 clips sem reescrever a timeline (canvas/virtualização) — **corrigível, mas é reescrita da camada de timeline**.

## 6. Multi-sequence

`grep` por `sequences|nestedSequence|compoundClip|activeSequence` em `src/` e `scripts/`: **zero ocorrências** relevantes (apenas `output.sequences` de tensores ASR). O projeto é **um único documento com um único conjunto fixo de faixas**. Classificação: **ARQUITETURAL**.

## 7. Pipeline de mídia

- Decode/encode: WebCodecs + mediabunny; libav.js (build LGPL compat em `src/vendor/libav-timeline-compat`, `BUILD.md`); `@ffmpeg/core` **GPL** como fallback; export determinístico offline + `MediaRecorder` como fallback.
- IA local: modelos WebGPU/ONNX (Whisper, matting, depth, etc.) com cache PWA. `MODEL_LICENSES.md` lista modelos com licenças que exigem triagem (alguns não comerciais) → **precisam ser removidos/substituídos** para uso comercial.
- Vídeo longo/VFR/memória: UNVERIFIED (não testado nesta fase).
- Desktop: somente navegador/PWA. Sem Tauri/Electron.

## 8. Edição por referência

- IMPLEMENTADO (executável): `scripts/analyze_replication.py` (cv2/numpy: detecção de cortes/repetições, **só diagnóstico**).
- INSTRUÇÃO DE AGENTE/SKILL (texto, não código): fluxos `replication-workflow`, `auto-edit-workflow`, `professional-editing-workflow`, `highlight-tension-workflow` etc. em `skills/edit-timeline-studio/SKILL.md`.
- ROADMAP: restante (ver `ROADMAP.md` do upstream).
- Requer Python + OpenCV local; não foi executado ponta a ponta com um vídeo de referência real nesta fase (UNVERIFIED).

## 9. Licenças (triagem técnica, não é parecer jurídico)

| Item | Licença | Veredito |
|---|---|---|
| Código do projeto | MIT | GREEN |
| `@ffmpeg/core` (fallback) | GPL (inclui x264) | RED se distribuído → remover |
| render headless `libx264` | GPL (via ffmpeg do usuário) | RED para o produto (ADR-032) |
| libav.js compat | LGPL-2.1+ | YELLOW (cumprir relink/atribuição) |
| Modelos de IA locais | variadas (`MODEL_LICENSES.md`) | YELLOW/RED por modelo |
| Mídia/fontes/sons de exemplo | não auditado | UNVERIFIED |
Veredito do repositório: **LEGAL_YELLOW** (MIT, mas com dependências/binários/modelos que exigem remoção ou substituição).
