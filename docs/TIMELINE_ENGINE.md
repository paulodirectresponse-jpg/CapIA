# TIMELINE ENGINE — Tempo, tracks, clips, nested sequences

## 1. Modelo de tempo (decisão crítica — ADR-007)

### 1.1 Problema
Floats em segundos acumulam erro (0.1 + 0.2 ≠ 0.3), não representam exatamente 29,97 fps (30000/1001), quebram comparações de igualdade de cortes, divergem entre preview e export e tornam undo/redo não idempotente.

### 1.2 Solução: `Ticks` inteiros + `Rational`

```rust
/// Unidade canônica: 1 tick = 1/705_600_000 s  (o "flick")
pub const TICKS_PER_SECOND: i64 = 705_600_000;
pub struct Ticks(pub i64);

pub struct Rational { num: i64, den: i64 }   // sempre normalizado, den > 0
pub struct FrameRate(Rational);              // ex.: 30000/1001, 24/1, 60/1
pub struct TimeRange { start: Ticks, duration: Ticks }
```

Por que 705.600.000: é divisível exatamente pela duração de frame de **todas** as taxas usuais e pelos sample rates de áudio:

| Taxa | Ticks por frame/amostra |
|---|---|
| 23,976 (24000/1001) | 29.429.400 |
| 24 | 29.400.000 |
| 25 | 28.224.000 |
| 29,97 (30000/1001) | 23.543.520 |
| 30 | 23.520.000 |
| 50 | 14.112.000 |
| 59,94 (60000/1001) | 11.771.760 |
| 60 | 11.760.000 |
| 120 | 5.880.000 |
| 44.100 Hz | 16.000 |
| 48.000 Hz | 14.700 |
| 96.000 Hz | 7.350 |

Faixa: i64 cobre ~415 anos. **Em JavaScript**, `Number` representa inteiros exatos até 2^53 ≈ 9,0e15 ticks ≈ **147 dias** — por isso a validação limita qualquer posição/duração de timeline a **24 h** e o cliente TS pode usar `number` com segurança (o tipo gerado é `Ticks = number` com asserts de inteiro seguro). Fontes com timestamps absurdos são normalizadas para começar em 0.

### 1.3 Regras

1. **Toda posição e duração no modelo é `Ticks`.** Floats só existem na UI (pixels) e em parâmetros não temporais (opacidade, escala).
2. Conversões frame↔ticks e sample↔ticks são **exatas** quando a taxa divide o timebase; para taxas exóticas (ex.: 12,5 fps OK; 23,98 declarado de forma errada), o probe normaliza para a racional padrão mais próxima e registra `rate_normalized=true`.
3. Arredondamento único e documentado: `floor` para "qual frame contém t", `round-half-up` para snapping de valores vindos da UI.
4. **Invariante de alinhamento (V1):** `start` e `duration` de todo clip numa sequence são múltiplos da duração de frame da sequence. Áudio pode ter ajuste fino sub-frame via propriedade `audio_offset: Ticks` (alinhado a amostra), não via posição.
5. Cada sequence tem `frame_rate` próprio. Ao mudar o fps de uma sequence, um comando realinha todos os clips (round) — operação explícita e desfazível.

### 1.4 Tempo de origem (source time) vs tempo de timeline

```
timeline_t  ──(clip.start, clip.source_in, clip.speed)──►  source_t  ──(frame index)──►  frame/amostra físico
source_t = clip.source_in + (timeline_t − clip.start) × speed        // speed: Rational, > 0 (reverso = flag)
```

- `source_t` é medido a partir do **início normalizado da mídia** (primeiro PTS de apresentação = 0), em Ticks.
- O `MediaFile` guarda `time_base` original de cada stream e `start_pts`; a conversão para PTS nativo é feita só em `capia-media`.
- Speed V1: constante por clip (racional, ex.: 3/2), + reverse + freeze frame. Speed ramps (curvas de tempo) ficam para depois, mas o mapeamento já é uma função `TimeMap` substituível.

### 1.5 CFR, VFR e frame accuracy

Celulares geram VFR com frequência — é o caso **normal** em UGC.

1. No ingest, um job constrói o **Frame Index** de cada stream de vídeo: lista de `(pts_ticks, is_keyframe, byte_pos)` — persistido no cache por fingerprint.
2. Classificação: `CFR` (desvio ≤ 0,5% do intervalo nominal), `VFR-leve`, `VFR-pesado`.
3. **Regra única de seleção de frame** (usada por preview e export — mesma função em `capia-media`):
   `frame_at(source_t) = último frame com pts ≤ source_t` (sample-and-hold).
4. Conformação para o fps da sequence: para cada frame de saída `n`, `t = n × frame_duration(seq)`, depois `frame_at(source_t(t))`. Isso dá cadência determinística independente de VFR.
5. VFR-pesado: o app oferece (e o AI pode solicitar) gerar um **intermediate CFR** como representação; a regra de seleção continua a mesma.
6. Seek frame-exato: seek ao keyframe anterior + decode até o alvo; nunca confiar em seek por timestamp aproximado do container.
7. Áudio: decodificado e reamostrado para o sample rate da sequence (padrão 48 kHz) com resampler de alta qualidade; posição de amostra derivada de Ticks exatamente. Sincronia A/V é garantida porque vídeo e áudio usam o mesmo `source_t` e o `start_pts` de cada stream (offsets de edit list / priming de AAC aplicados no probe).

### 1.6 Consistência preview/export
Preview e export chamam a **mesma** função de compilação timeline→render graph com o mesmo `t` em Ticks. Diferenças permitidas no preview: resolução, uso de proxy, qualidade de reamostragem — nunca timing. Ver `PREVIEW_RENDER.md` §4.

## 2. Tracks: família tipada + role (ADR-008)

### Alternativas
- **Tracks rigidamente tipadas** (VideoTrack, AudioTrack, TextTrack, CaptionTrack, NestedTrack...): simples para UI, mas ruins para composição (texto e vídeo precisam se intercalar em z-order; um nested é visual) e explodem em casos especiais.
- **Totalmente genérico** (qualquer clip em qualquer track): flexível, mas o mixer de áudio e o compositor de vídeo precisam de semânticas diferentes; validação fica vaga.

### Decisão
Duas **famílias** com semântica de render distinta, e um **role** opcional para UX/IA:

```rust
enum TrackKind { Visual, Audio }
enum TrackRole { Main, Overlay, Text, Captions, Voice, Music, Sfx, Custom(String) } // dica, não restrição dura
```

- `Visual` aceita: vídeo, imagem, texto, legenda, nested sequence, sólido (e futuramente adjustment). Z-order = ordem da track (V1 embaixo).
- `Audio` aceita: áudio de mídia, música, SFX, voz, componente de áudio de nested sequence.
- `role` orienta a UI (cores, agrupamento, onde soltar por padrão) e a IA ("coloque SFX na track de SFX"), e pode ter regras leves (ex.: `Captions` só aceita clips de legenda — validação por role configurável).
- **Sem limite artificial** de tracks (limite de sanidade: 1.000 por sequence).
- Flags por track: `locked`, `hidden` (visual), `muted` (áudio), `solo`, `magnetic`, `height` (UX), `volume_db` (áudio).

### Main track magnética (familiaridade CapCut)
Cada sequence tem no máximo uma track `Visual` com `role=Main` e `magnetic=true` por padrão: clips ficam encostados (sem gaps), deletar faz ripple, inserir empurra. Tracks de overlay são de posicionamento livre. O usuário pode desligar o magnetismo.

## 3. Clips

```rust
struct Clip {
  id, track_id, start: Ticks, duration: Ticks, name, enabled, color_label,
  content: ClipContent, transform/props: PropertySet, effects: Vec<EffectInstance>,
  group_id: Option<GroupId>, origin: Origin,
}
enum ClipContent {
  Media   { asset_ref: AssetRef, source_in: Ticks, speed: Rational, reversed: bool,
            video: Option<VideoComponent>, audio: Option<AudioComponent> },
  Image   { asset_ref: AssetRef },                      // duração livre
  Text    { text: RichText, style_ref: Option<StyleId>, style_overrides },
  Caption { words: Vec<CaptionWord>, style_ref, style_overrides, source_transcript: Option<TranscriptRef> },
  Nested  { sequence_id: SequenceId, source_in: Ticks, follow_length: bool, audio: Option<AudioComponent> },
  Solid   { color },
}
```

### Áudio vinculado (embedded) vs desvinculado (ADR-009)
Seguindo o modelo mental do CapCut, um clip de mídia com vídeo+áudio carrega **ambos os componentes** e corta/move como unidade (zero risco de dessincronia). `detach_audio` cria um clip de áudio separado numa track `Audio`, com o mesmo `source_in`/range, e desliga o componente de áudio do clip original. Para reagrupar, usa-se `group_clips`. Um clip de mídia em track `Audio` só tem componente de áudio.

### Keyframes
- Toda propriedade animável é `AnimatableValue<T> = Static(T) | Animated(Vec<Keyframe<T>>)`.
- `Keyframe { time: Ticks (relativo ao conteúdo), value, interp: Hold|Linear|Bezier{..}|Ease(preset) }`.
- **Tempo do keyframe é relativo ao conteúdo do clip** (para mídia: tempo de origem; para texto/sólido: tempo local desde a criação). Assim, mover o clip preserva a animação e trimar a entrada não desloca a animação em relação ao conteúdo (comportamento de editores profissionais).
- Avaliação determinística e idêntica em preview/export (função pura em `capia-model`).

### Transições
- Entidade própria ligada a um corte: `Transition { id, track_id, left_clip, right_clip, kind, duration, alignment: Center|StartAtCut|EndAtCut, params }`.
- Exige clips **adjacentes** na mesma track. Usa **handles** (mídia além do in/out). Duração da timeline **não muda**.
- Sem handles suficientes: a duração é limitada ao disponível; se zero, usa freeze do frame de borda (sinalizado na UI e no `validate` como warning).
- Ao mover/remover um dos clips de modo a desfazer a adjacência, a transição é removida no mesmo comando (registrado no histórico).

### Grupos
`Group { id, members: Vec<ClipId> }` — seleção e movimento conjunto, entre tracks. Não aninha render (para isso: nested sequence / "compound").

### Legendas
Um clip `Caption` representa uma frase/bloco; contém palavras com tempos relativos ao clip e estilo por palavra (highlight típico de DR). Geradas a partir de um `Transcript` (ver `AI_SYSTEM.md`), mas são clips comuns e editáveis. Regerar legendas é um comando que substitui os clips de uma track `Captions` (um passo de undo).

## 4. Nested Sequences / Compositions (ADR-010)

### Semântica
| Conceito | Definição |
|---|---|
| **Referência compartilhada** (padrão) | Clip `Nested{sequence_id}` aponta para uma sequence viva do projeto. Editar `BODY_MASTER` altera **todas** as instâncias imediatamente. |
| **Instância independente** (`make_unique`) | Duplica a sequence referenciada (cópia profunda, novos IDs, nome `BODY_MASTER (HOOK2)`) e repointa só aquele clip. Alterações futuras no master não propagam. |
| **Detach / Flatten** (`flatten_nested`) | Substitui o clip nested pelos clips internos, colocados em tracks do pai (criando tracks se necessário), respeitando range, transform e tempo. |
| **Propagação** | Automática para referências; nenhum "push" manual. A UI mostra o contador de usos ("usado em 3 sequences") e alerta ao editar um master compartilhado. |
| **follow_length** | Se `true`, o clip nested estende/encolhe quando o master muda de duração (útil para "Hook + BODY"). Na main track magnética, a mudança faz ripple no pai. Se `false`, o clip mantém sua duração e mostra área vazia/cortada. |

### Prevenção de ciclos
O projeto mantém um **grafo de dependência de sequences** (DAG). `insert_clip(Nested)` e `set_nested_target` validam que o alvo não alcança a sequence pai (DFS; custo trivial). Profundidade máxima de aninhamento: 16. Violação → erro `NESTED_CYCLE` / `NESTED_DEPTH`.

### Undo/redo
O histórico é **por projeto**, não por sequence (ADR-011). Editar o master é um passo único que, ao desfazer, reverte o efeito em todas as instâncias. A UI pode filtrar a visualização do histórico por sequence, mas a pilha é linear e global.

### Render
O compilador expande nested como **subgrafo** com seu próprio formato: o subgrafo é renderizado no tamanho da sequence filha e depois transformado (fit/fill) no pai. Diferença de fps: conformação sample-and-hold igual à de mídia. O cache de render de uma sequence é chaveado por `(sequence_id, sequence_content_hash, t, qualidade)` — editar o master invalida exatamente as regiões necessárias.

### Versionamento
- `Sequence.revision` incrementa a cada alteração (cache e auditoria).
- Para "congelar" uma versão, o usuário usa `make_unique` ou snapshot de projeto (`DATA_MODEL.md` §6). V1 não tem "referência fixada a revisão" (complexidade alta, baixo valor); a porta fica aberta via campo `pinned_revision: Option` reservado no schema.

### Variações (caso central de DR)
`HOOK1 + BODY_MASTER`, `HOOK2 + BODY_MASTER`... são sequences cuja main track contém `[Nested(HOOK_n), Nested(BODY_MASTER)]`. O comando composto `generate_variants(hooks[], bodies[], format)` cria a matriz (uma transação).

## 5. Operações de edição (semântica)

| Operação | Semântica |
|---|---|
| insert | Insere no tempo `t`. Em track magnética: empurra posteriores (ripple). Em overlay: exige espaço livre; se ocupado, a UI cria/usa outra track (o comando recebe track explícita). |
| overwrite | Substitui conteúdo no range (corta/remove o que estiver por baixo). |
| move | Muda `start` e/ou `track`. Não pode sobrepor na track destino. |
| trim (in/out) | Ajusta borda; limitado por handles da mídia (imagem/texto/sólido: ilimitado). Em track magnética: ripple. |
| ripple trim / ripple delete | Remove o tempo e fecha o gap na track (opcionalmente em todas as tracks não travadas — `ripple_scope: Track|Sequence`). |
| split | Divide em `t` em dois clips com IDs novos para a metade direita; keyframes e efeitos copiados; transições preservadas nas bordas externas. |
| slip / roll / slide | Fase 3 (opcionais para V1; o modelo já suporta). |
| lock | Track travada rejeita qualquer comando que a altere (`TRACK_LOCKED`), inclusive de IA. |

Snapping é **UX** (calcula o alvo); o comando recebe valores já alinhados e o core revalida o alinhamento a frame. Ver `TIMELINE_UX.md` §5.

## 6. Invariantes do documento (validadas após toda transação)

1. Sem sobreposição de clips na mesma track.
2. Track magnética sem gaps (se `magnetic=true`).
3. `start`, `duration` alinhados a frame da sequence; `duration > 0`.
4. `source_in + duration × speed ≤ duração da mídia` (exceto imagem, texto, sólido; mídia offline usa a última duração conhecida).
5. Clip compatível com a família da track (e com o role se a regra estiver ativa).
6. Grafo de nested acíclico, profundidade ≤ 16.
7. Transições apenas entre clips adjacentes existentes, duração ≤ handles disponíveis.
8. Referências válidas: `track_id`, `sequence_id`, `group_id`, `style_ref`, `asset_ref` (asset pode estar offline, mas deve existir).
9. IDs únicos no projeto.
10. Limites: posições ∈ [0, 24 h]; tracks ≤ 1.000; clips por sequence ≤ 100.000 (sanidade).

## 7. Desempenho do modelo

- Índice por track: estrutura ordenada por `start` (B-tree/`im::OrdMap`) → busca de clip em `t` em O(log n).
- Consultas de range (`clips_in(range)`) para preview/UI/virtualização.
- Alvo: aplicar um comando típico em < 1 ms; transação de 500 operações validada e commitada em < 200 ms num projeto de 10.000 clips.
