# STATUS

**Última atualização:** 2026-10-02 · **Fase atual:** FASE 1 — Fundação · **Fase 1: NÃO COMPLETA** (2 itens pendentes, ver "Blockers")

## Estado

| Item | Estado |
|---|---|
| Código de produto | Nenhum (intencional). Código descartável só em `/spikes` |
| Arquitetura e documentação | ✅ M01; emendada na M03 (command system, ADRs 029–036) |
| Auditoria open-source / estratégia | ✅ M02. **Estratégia C aprovada** (ADR-031); política de proveniência em `docs/PROVENANCE.md` |
| Spikes | ✅ S2–S7 concluídos · ⚠️ **S1 não mensurável no ambiente de nuvem** |
| Decisões abertas | **OD-1 aberta** (S1) · OD-2 fechada (ADR-032) · OD-3 fechada (ADR-033) |
| ADRs `Proposed` | Nenhum (ADR-016 aceito na M03) |
| Scaffold / CI | Não iniciado (M04) |
| Git | Commits locais em `claude/happy-bardeen-feyypa`; push: ver "Blockers" |

## Missões

| Missão | Resultado |
|---|---|
| M01 — Fundação arquitetural | ✅ `CLAUDE.md`, 15 docs, `.gitignore` |
| M02 — Auditoria open-source | ✅ `OPEN_SOURCE_AUDIT.md`; nenhum fork; reuso seletivo |
| **M03 — Fechamento da fundação e spikes** | ✅ spikes S2–S7, ADR-029..036, `PROVENANCE.md`, `tests/acceptance/timeline` (108 cenários), emendas ao `COMMAND_SYSTEM.md`. ⚠️ S1 não medido |

## Resultados dos spikes (resumo; detalhes em `docs/spikes/`)

| Spike | Resultado | Decisão |
|---|---|---|
| S1 preview surface | **NÃO MEDIDO.** `wry` só faz windowed hosting (verificado); proxy de upload no navegador é barato; protocolo + regra de decisão prontos | OD-1 **aberta** |
| S2 frame-exato/VFR | Lógica confirmada (sintético); `-ss` e filtro `fps` do ffmpeg **não** servem para timing; HW decode/HDR **não medidos** | ADR-035 |
| S3 FFmpeg LGPL | Viável; BtbN é LGPL-3.0; build mínima própria LGPL-2.1+ | ADR-032 |
| S4 WASM | Paridade exata nativo×WASM; 1,3 µs/ghost-move; 36 KB | ADR-016 aceito |
| S5 canvas | 60 fps com 10.000 clips visíveis sem GPU; DOM ~8 fps no zoom | ADR-005 confirmada |
| S6 compositor | `REIMPLEMENT_WITH_REFERENCE` (8 bits, ~16 MB/camada, sem YUV/texto) | ADR-034 |
| S7 aceitação | 108 cenários consistentes; mutação os valida | ADR-036 |

## Métricas da M03 (marcos, critérios, testes, riscos, retrabalho — sem pessoas-semanas)

- **Marcos:** 6 de 7 spikes com resultado medido; 8 ADRs novos (029–036) + 1 aceito (016); 1 documento novo de política (`PROVENANCE.md`); 3 documentos técnicos emendados com decisões fechadas.
- **Critérios de aceitação/testes executados:** 108 cenários de aceitação (108/108 no oráculo; 3/3 mutantes detectados); 9/9 checagens de correção do compositor OpenCut em wgpu nativo; seek por índice 240/240 exato (4 clipes) vs `-ss` 1/240; paridade nativo×WASM por hash em 200k operações; 4 benchmarks de timeline em canvas/DOM; **auditoria automatizada de consistência entre documentos: 215 verificações** (ADRs definidas, caminhos, seções citadas, estado das ODs, termos obsoletos, contagem de cenários) — 2 referências abreviadas em `PROVENANCE.md` corrigidas; 4 apontamentos restantes são falsos positivos conhecidos (3 caminhos de *outros* repositórios citados em `OPEN_SOURCE_AUDIT.md`; 1 contagem em prosa no S7).
- **Riscos eliminados:** timeline em canvas viável a 10.000 clips; WASM com paridade; frame-exatidão/VFR sem depender do ffmpeg para timing (e por quê); build LGPL sem libs externas existe; compositor do OpenCut **não** é base (evita fork com dívida); rótulo de licença incorreto do FFmpeg do OpenCut identificado (LGPL-3.0, não 2.1).
- **Riscos novos/registrados:** patentes H.264/HEVC/AAC e binário OpenH264 (ADR-032); reprodutibilidade da build não verificada; IPC/cold-start do WASM no Tauri não medidos; **S1 sem medição**; escopo do compositor próprio (RGBA16F, bbox, YUV, texto).
- **Retrabalho:** oráculo S7 — 2 iterações de ajuste de rigor + 1 lacuna de cobertura achada por mutação (GRP-014/015 adicionados); sonda S6 — 2 correções (API não reexportada; nome de uniforme); 1 comando de limpeza bloqueado pela proteção do ambiente (refeito sem remoção).
- **Custo aproximado:** ~0,26 M tokens de contexto consumidos nesta sessão (estimativa); custo em dinheiro da sessão **não informado**.

## Blockers (o que impede encerrar a Fase 1)

1. **S1 em Windows 11 (OD-1).** Exige máquina Windows com WebView2 e GPU (iGPU basta). Protocolo e regra de decisão: `docs/spikes/S1-preview-surface.md`. Dependem de OD-1: o presenter nativo de `capia-preview` e toda a Fase 3; **a Fase 2 headless não depende**.
2. **Scaffold + CI** (workspace vazio, CI Windows, secret scan, verificador de licenças): não feito na M03 (fora do escopo pedido). Precisa de acesso de escrita ao GitHub para validar o CI.
3. **Acesso de escrita ao GitHub:** `git push` retorna 403 ("Claude não tem acesso ao repositório"). Reconectar em https://claude.ai/connect-github e instalar o Claude GitHub App no repositório. As ferramentas MCP do GitHub conseguem *ler* o repositório (vazio, sem branches).

## Decisões do Product Owner pendentes (nenhuma bloqueia sozinha)

- Executar S1 antes de começar a Fase 2, ou **dispensar** o gate do S1 para a Fase 2 headless e executá-lo antes da Fase 3 (recomendação: executar antes do fim da Fase 2).
- Confirmar D-S7-1..8 (`docs/spikes/S7-timeline-acceptance.md`): faixa de velocidade, inserção magnética, ripple de sequência, arredondamentos, prioridade de snap, retenção de keyframes.
- Política de patentes/OpenH264 (ADR-032) e licença do repositório do produto antes do primeiro release.

## Próxima missão

**M04 — Fechar a Fase 1:** executar S1 em Windows (ou decidir dispensar o gate), scaffold do repositório + CI + verificação de licenças, corrigir o acesso de escrita. Só então a Fase 2 começa.

## Notas

- Leia `CLAUDE.md` e este arquivo primeiro; depois os docs da missão.
- Nenhum código de terceiros foi incorporado (`docs/PROVENANCE.md` §4 vazio). Qualquer reuso segue a política.
- Ao concluir uma missão, atualize esta página.
