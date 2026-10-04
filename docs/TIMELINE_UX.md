# TIMELINE UX — Experiência de edição

> Objetivo: quem usa CapCut Desktop deve se sentir em casa em minutos — **mesma lógica de interação e velocidade**, com design system, marca, ícones e assets **próprios**. Nada de código, assets ou identidade visual do CapCut.

## 1. Layout

```
┌───────────────────────────────────────────────────────────────────────────────────────┐
│ Top bar: projeto ▸ pasta ▸ sequence | Undo/Redo | AI Run status | Export              │
├──────────────┬──────────────────────────────────────────────┬─────────────────────────┤
│ Left rail    │                                              │ Inspector               │
│ (abas):      │              PREVIEW (centro)                │ (contexto da seleção:   │
│ Project      │      superfície nativa + controles           │  Clip/Texto/Áudio/      │
│ Media        │      safe-areas 9:16, grid, zoom             │  Transição/Sequence)    │
│ Audio/Music  │                                              │  abas: Básico · Anim ·  │
│ SFX · Text   │                                              │  Áudio · Velocidade ·   │
│ Captions     │                                              │  Efeitos                │
│ Transitions  │                                              │                         │
│ Effects · AI ├──────────────────────────────────────────────┴─────────────────────────┤
│              │ Sequence tabs: [ AD1 · Hook 1 ][ AD1 · Body ][ BODY_MASTER ][ + ]       │
│              │ Toolbar: select/blade · split · delete · snapping · link · zoom ─●──    │
│              │ Ruler ───┬──────────────────────────────────────────────────────────    │
│              │ T  Text  │      [ título ]            [ CTA ]                            │
│              │ V2 Over  │   [ broll ][ broll ]    [ produto ]                           │
│              │ V1 MAIN  │ [ talking head ▓▓ thumbnails ▓▓ ][ talking head ][ nested ]  │
│              │ A1 Music │ ~~~~~~~~~~~~~~~~ waveform ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~  │
│              │ A2 SFX   │     ▪whoosh        ▪pop                                      │
└──────────────┴──────────────────────────────────────────────────────────────────────────┘
```

Painéis redimensionáveis e recolhíveis; layout salvo por workspace. Timeline ocupa a região inferior (padrão ~40% da altura).

## 2. Multi-sequence: UX recomendada

Dois mecanismos complementares (ADR-012):

1. **Painel Project (árvore)** — fonte da organização: pastas (`AD 1/`, `AD 2/`, `Masters/`), sequences, contador de usos de nested, badge de formato (9:16 · 30 fps), status de export. Drag-and-drop para reorganizar; arrastar uma sequence para a timeline cria um clip nested.
2. **Abas de sequences abertas** acima da timeline — navegação rápida estilo navegador. Abas não são a organização; fechar uma aba não apaga nada.

Botão **`+`** (e `Ctrl+N` dentro da timeline):
- Cria sequence imediatamente com o formato da sequence ativa (ou preset padrão), nome provisório `Sequence N` em modo de renomeação inline, na mesma pasta da ativa.
- Menu secundário (seta do `+`): "Nova a partir de preset (9:16 / 1:1 / 4:5 / 16:9)", "Duplicar atual", "Nova variação (Hook + Master)".

Menu de contexto da aba/árvore: renomear, duplicar, duplicar como instância independente, mover para pasta, revelar no projeto, exportar, adicionar a deliverables, fechar.

Navegar para dentro de um nested: duplo clique no clip abre a sequence filha em nova aba com breadcrumb `HOOK2 ▸ BODY_MASTER` e aviso "Editando master compartilhado (3 usos)".

## 3. Elementos da timeline

| Elemento | Comportamento |
|---|---|
| Ruler | Timecode (HH:MM:SS:FF), marcas adaptadas ao zoom; clicar/arrastar move o playhead |
| Playhead | Linha vertical, arrastável; scrub com áudio opcional |
| Cabeçalho de track | Nome, role (ícone/cor), lock, hide (visual), mute (áudio), solo, altura ajustável |
| Clip visual | Faixa de thumbnails (tiles), nome, badges (velocidade, efeitos, keyframes, offline, gerado por IA) |
| Clip de áudio | Waveform; linha de volume opcional; fades por "alças" nos cantos |
| Clip de texto/legenda | Texto renderizado resumido; cor de role |
| Nested | Cor própria, ícone, nome da sequence; mini-thumbnails |
| Transição | Marcador sobre o corte; arrastar as bordas muda duração |
| Keyframes | Diamantes no clip selecionado (por propriedade ativa no inspector) |
| Marcadores | Na ruler (sequence) e nos clips |
| Itens criados por IA | Marca sutil + tooltip com a origem (Run/plan step). Visual idêntico em todo o resto: são clips comuns |

## 4. Interações (paridade funcional com editores populares)

- **Drag-and-drop** da biblioteca para a timeline: soltar na main track insere com ripple; soltar acima cria/usa track de overlay; soltar sobre área ocupada de overlay cria nova track automaticamente.
- **Mover**: arrastar clip; entre tracks; `Alt`+arrastar duplica. Mover na main track reordena (magnético).
- **Trim pelas extremidades**: cursor muda nas bordas; na main track faz ripple; em overlay limita ao vizinho; limites de handle indicados visualmente.
- **Split** no playhead (`Ctrl+B` / `S`), **delete** (`Del`), **ripple delete** (`Shift+Del`), trim-to-playhead esquerdo/direito (`Q`/`W`).
- **Snapping** liga/desliga (`N`); **link de áudio** (`Ctrl+L`), **detach audio**.
- **Copy/paste/duplicate** (`Ctrl+C/V/D`) — cola no playhead, na track selecionada; funciona entre sequences e projetos (assets importados conforme necessário).
- **Grupos** (`Ctrl+G` / `Ctrl+Shift+G`).
- **Zoom horizontal**: `Ctrl+scroll`, `+`/`-`, slider; ancorado no cursor ou playhead; "fit" (`Shift+Z`). Scroll vertical entre tracks.
- **Seleção**: clique, `Shift` (adicional), marquee; selecionar à frente na track (`A`)...
- **Playback**: `Space`, J/K/L shuttle, ←/→ frame a frame, `Shift+←/→` 10 frames, `Home/End`.
- **Keymap configurável** com preset padrão próprio e presets "familiar" inspirados em convenções comuns de editores (sem prometer identidade com nenhum produto).

Todas as interações acima resultam em **um comando** (ou uma transação) do Command Engine; nenhuma altera o documento diretamente.

## 5. Snapping

- Alvos: playhead, bordas de clips (todas as tracks visíveis), marcadores, início da sequence, bordas de keyframes, e (quando houver transcript) **limites de palavras** — muito útil em DR.
- Limiar em pixels (padrão 8 px) convertido para **Ticks** no zoom atual (`px ÷ px_por_segundo × 705.600.000`), **sem floor para frames inteiros** (D-S7-4); o destino final respeita o alinhamento a frame em tracks visuais (half-up). Empate de distância: playhead > marcador > borda de clip > menor timestamp (D-S7-5). Implementado em `capia-commands::{threshold_ticks, resolve_snap, resolve_group_move}`.
- Indicador visual da linha de snap e do alvo.
- Função pura, testável, rodando na UI (WASM do core para os alvos e alinhamento).

## 6. Como atingir a fluidez (requisitos de engenharia)

| Requisito | Meta | Como |
|---|---|---|
| Scroll/zoom da timeline | 60 fps com 5.000 clips visíveis-na-sequence | Canvas (2D ou WebGL) + virtualização por range de tempo e track; nada de DOM por clip |
| Latência de arrasto | < 16 ms por frame | Ghost calculado localmente pelo WASM do core sobre a read-replica; IPC só no drop |
| Commit de comando | < 30 ms do drop até a réplica atualizada | Single-writer em Rust + patches incrementais |
| Thumbnails | Tiles em várias densidades; aparecem em < 200 ms após scroll | Gerados por job, cache em disco, servidos via `capia://`; placeholder imediato |
| Waveforms | Sem recálculo ao dar zoom | Pirâmide de picos min/max pré-calculada (vários níveis) |
| Scrub do preview | Resposta < 100 ms com proxies | Preview engine com cache de frames e decode HW (`PREVIEW_RENDER.md`) |
| Undo/redo | < 50 ms | Patches inversos |

### Avaliação de bibliotecas de timeline
Bibliotecas prontas de timeline web (ex.: componentes React de "timeline editor" e de animação) foram avaliadas conceitualmente e **rejeitadas como base** (ADR-005): são DOM-based ou pensadas para animação/keyframes genéricos, não suportam track magnética, nested, transições com handles, invariantes do nosso modelo, virtualização para milhares de clips nem a integração com um estado autoritativo externo (core Rust) que também é alterado pela IA. Construiremos `packages/ui-timeline` próprio: renderer em canvas + camada de interação + adaptadores para o read-model. Primitivas genéricas (gestos, scroll virtual) podem vir de bibliotecas pequenas.

## 7. IA na UX

- Painel **AI** (aba lateral) para conversa, brief e acompanhamento de Runs (stages, custo, plano).
- O **Edit Plan** é visualizável/aprovável antes da edição.
- Durante um Run, a timeline mostra as alterações por transação; cada transação de IA é um item de histórico ("AI · Editor: montar body da AD 2") desfazível como unidade.
- Edição manual concorrente é permitida em outras sequences; na sequence alvo, a UI avisa e o core detecta conflitos (`COMMAND_SYSTEM.md` §7).

## 8. Acessibilidade e internacionalização

Contraste AA, foco visível, atalhos em todas as ações principais, UI em pt-BR e en desde o início (strings externalizadas).

## Estado de implementação e medição (Fase 3)

Implementado: renderer em canvas virtualizado (sem DOM por clip), ghost/snap/grupo/colocação pelo WASM do core, seleção/marquee/move/reordenar/trim/blade/split/delete/ripple/copiar-colar/duplicar/grupos, faixas, nested, zoom/fit, marcadores, miniaturas e waveform. As metas de §6 foram **medidas** (5.000 clips; Chromium headless sem GPU + engine release) em `docs/STATUS.md` ("Metas de UX medidas") e em `target/perf/phase3-ui-perf.json`: pintura e arrasto passam com folga; commit/undo passam no engine e ficam **na margem** quando medidos pela UI neste ambiente — o benchmark estrito em hardware de referência é `CAPIA_PERF_STRICT=1` (pacote de aceitação).
