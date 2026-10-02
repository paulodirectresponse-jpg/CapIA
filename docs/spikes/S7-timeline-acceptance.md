# S7 — Suíte de aceitação de comportamento da timeline

**Pergunta:** conseguimos especificar o comportamento CapCut-like (placement, snapping, ripple, retime, group move, keyframes) como suíte **nossa**, executável e independente do OpenCut?

**Resultado: CONFIRMADO.** **108 cenários** em `tests/acceptance/timeline/*.json`, **108/108 consistentes** com um oráculo descartável; a validação por mutação detecta regressões.

## O que foi produzido

| Artefato | Papel | Permanência |
|---|---|---|
| `tests/acceptance/timeline/{placement,snapping,ripple,retime,group_move,keyframes}.json` | Cenários dados→comando→resultado. **Critério de aceitação da Fase 2** para `capia-commands` | **Permanente** |
| `spikes/s7-oracle/oracle.py` | Oráculo de referência (≈350 linhas, Python) só para provar que os cenários são consistentes e sem ambiguidade. **Não é o engine** | Descartável |

| Área | Cenários |
|---|---|
| placement (inclui overlap, track magnética, estratégias, ticks a 29,97) | 19 |
| snapping (alvos, prioridade, empate, threshold px→frames, overlap) | 16 |
| ripple / trim / split | 20 |
| retime (velocidade, arredondamento, handles, split, reverso) | 17 |
| group move (clamp, obstáculos, trilhas, snap) | 15 |
| keyframes (interp., trim/split, retime, limites, bezier) | 21 |

**Formato:** cada cenário tem `given` (tracks/clips em **frames**; `fps` configurável), `when` (um comando ou lista), `then` (resultado completo). Posições em frames para legibilidade; o harness futuro converte para Ticks e **confere exatidão** (cenário PLC-019 afirma os ticks a 29,97 fps: frame 30 = 706.305.600). Resultados esperados foram **derivados à mão** da especificação do CapIA, não gerados pelo oráculo.

**Comandos exercitados:** `insert_clip`, `delete_clip`, `trim_clip`, `split_clip`, `set_clip_speed`, `add_keyframe`, `move_keyframe`; funções puras de UX/engine: `resolve_snap`, `resolve_group_move`, `resolve_placement`, `threshold_frames`, `eval_keyframes`.

## Validação

1. Primeira execução: 101/106 (as 5 falhas eram rigor do oráculo ao tratar mudança de keyframes como mudança de track — corrigido: "track não listada" = timing inalterado) e 4 cenários de split passaram a declarar o resultado completo.
2. **Mutação** (3 defeitos injetados no oráculo: borda de obstáculo inclusiva, arredondamento por truncamento, threshold exclusivo): **3/3 detectados** (RTM-004, SNP-003, GRP-014). A primeira versão da suíte **não** detectava o defeito de obstáculo encostado; por isso foram adicionados GRP-014/015.
3. Erros devem deixar o estado **intacto** (atomicidade) — verificado em todos os cenários de erro.

## Proveniência (política: `docs/PROVENANCE.md`)

Cada cenário traz `provenance`: `basis` (`capia-spec` ou `opencut-behavior+capia-spec`), referência à spec do CapIA e, quando o comportamento foi *observado* no OpenCut, `repo@commit` + arquivos consultados com a nota "comportamento observado, reimplementado do zero; nenhum código ou literal de teste copiado". **Nenhum teste do OpenCut foi copiado.** Cenários onde o CapIA **diverge deliberadamente** do OpenCut têm `diverges_from_opencut` com a razão (ex.: keyframes fora do clip são preservados; overlap validado no snap; ripple de sequência recusa em vez de criar overlaps; track magnética nunca é auto-selecionada).

## Comportamentos definidos por esta suíte que o PO pode querer confirmar (não bloqueiam a Fase 2)

| # | Regra proposta | Alternativa |
|---|---|---|
| D-S7-1 | Velocidade de clip em **[1/100, 5]** (igual ao OpenCut) | Faixa mais estreita (ex.: 0,1–10×) — decisão de produto |
| D-S7-2 | Inserção em track magnética **dentro** de um clip exige `split_at_insert`; senão `NOT_ON_BOUNDARY` (UI resolve intenção de drop) | Auto-split sempre |
| D-S7-3 | Ripple em escopo **sequence** recusa (`RIPPLE_CONFLICT`) se outro track tem clip sobreposto ao trecho removido | Cortar o trecho em todos os tracks (estilo Premiere) |
| D-S7-4 | Threshold de snap converte px→frames com **floor** | arredondar ao mais próximo |
| D-S7-5 | Prioridade de empate no snap: playhead > marcador > bordas de clip; depois menor tempo | outra ordem |
| D-S7-6 | Keyframes fora do trecho visível são **preservados** (trim não apaga) | descartar como o OpenCut |
| D-S7-7 | Comandos **rejeitam** valor fora do range; a UI faz clamp antes de enviar | clamp no comando |
| D-S7-8 | Arredondamento de duração após mudar velocidade: meio-frame para cima, mínimo 1 frame | outro modo |

**Fora da suíte (a especificar na Fase 3):** group move envolvendo track magnética (semântica de *reorder*), transições com handles, slip/roll/slide, nested (propagação/ciclos/`follow_length` — pertencem a testes de propriedade do `capia-commands`).
