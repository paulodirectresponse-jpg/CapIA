# S4 — Core Rust compilado para WASM

**Pergunta:** crates sem IO (Ticks/modelo/comandos) compilam para WASM com **paridade exata** com o nativo e latência adequada para *ghost preview* de gestos? (ADR-016)

**Resultado: CONFIRMADO para o núcleo de tempo/índice/ghost-move.** Latência de IPC do Tauri **NÃO medida** (proxy apenas).

## Método

`spikes/s4-wasm-core/` — protótipo descartável (~150 linhas, sem dependências, C ABI cru, sem wasm-bindgen): `Ticks(i64)`, índice por track (`BTreeMap`), índice global de bordas (alvos de snap), `ghost_move` com snapping e detecção de overlap, e um workload determinístico de **200.000 operações** sobre **10.000 clips** (50 tracks × 200). O mesmo `lib.rs` compila para nativo e `wasm32-unknown-unknown`; `run.mjs` carrega o `.wasm` no Node 22 (V8, mesma família do WebView2).

## Resultados

| Medida | Resultado |
|---|---|
| **Paridade nativo × WASM** | **hash idêntico** (`4670097539939199739`) após 200k operações; 4.187 commits simulados idênticos |
| Tamanho do binário (release, `opt-level=s`, LTO, sem `wasm-opt`) | **36,5 KB** |
| Latência de `ghost_move` chamada do JS (10k clips) | p50 **1,3 µs**, p95 2,4 µs, p99 4,5 µs; média em lote 0,83 µs/chamada (um outlier de ~1 ms = chamada fria/GC) |
| WASM vs nativo (mesmo workload) | ~0,84 µs/op vs ~0,48–0,68 µs/op (≈1,2–1,7×) |
| Construção do índice (10k clips) | 8,9 ms |
| Fronteira i64 | i64 vira `BigInt` no JS. Decisão: a API WASM usa **f64 nos limites**; com o teto de 24 h (6,1×10¹³ ticks) o valor é inteiro seguro em JS (limite seguro ≈ 3.546 h) |
| Proxy de IPC (JSON, **não é Tauri**) | 147 patches: 15,8 KB, stringify 0,04 ms / parse 0,06 ms; 2.000 patches: 220 KB, 0,8 / 1,4 ms; 20.000 patches: 2,2 MB, 11 / 16 ms |

## Limites da evidência

- Núcleo de brinquedo: o modelo real (UUIDs, `im`/`rpds`, serde, propriedades, keyframes) aumentará tamanho e custo; a paridade precisa ser **mantida por teste de CI**, não assumida.
- Sem `wasm-opt`; sem medir inicialização/compilação do módulo em WebView2 (cold start).
- **Throughput real do IPC Tauri não foi medido** (sem Tauri/Windows aqui); só a serialização em Node.
- Sem ponto flutuante no núcleo medido → a paridade bit a bit é esperada; propriedades animadas (f32/f64) exigem teste próprio (funções puras, evitar `fma`/`fast-math`).

## Decisão

**ADR-016 → Accepted**, condicionada a: (1) teste de paridade nativo×WASM por hash em CI (como este, sobre o modelo real); (2) interface WASM com f64 nos limites de tempo; (3) medir cold-start do módulo e IPC real no spike S1/Fase 3.
