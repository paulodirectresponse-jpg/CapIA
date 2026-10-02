# S6 — Compositor wgpu do OpenCut Classic

**Pergunta:** os crates MIT `gpu/compositor/effects/masks` do OpenCut Classic reduzem trabalho real do `capia-render`?

## Veredito: **`REIMPLEMENT_WITH_REFERENCE`**

Rejeitado como semente de *núcleo*; arquivos pequenos e específicos são candidatos a adaptação **com atribuição** (decisão arquivo a arquivo na Fase 2, sob `docs/PROVENANCE.md`).

Revisão: `OpenCut-app/opencut-classic@cf5e79e9` (MIT, arquivado em 2026-05). Isolado: nada copiado ou integrado; sonda em `spikes/s6-compositor-probe/` usa **somente a API pública** via *path dependency* para um checkout externo (não versionado aqui).

## Evidência

**Tamanho e dependências:** 4 crates de render (+ `time`), ~2,8 k linhas Rust + ~0,5 k linhas em 9 shaders WGSL (maiores arquivos: `compositor.rs` 870, `gpu/context.rs` 695, `effects/pipeline.rs` 330, `masks/sdf.rs` 332, `masks/feather.rs` 285). Dependências mínimas: `wgpu 29.0.1`, `bytemuck`, `thiserror`, `serde` (+ `bridge`, proc-macro próprio, só no crate `time`). Edition 2024. Compila nativamente do zero em ~61 s.

**Qualidade:** código limpo, erros tipados (`thiserror`), **0 `unwrap`/`panic`**, 3 `expect` (no contexto GPU); **0 testes nos crates GPU** (os poucos testes Rust do repo estão em `time`).

**Execução real** (llvmpipe, Vulkan por software, formato `Bgra8Unorm`): **9/9 checagens de correção passaram** — blend normal 50% (127,0,128 esperado 128,0,128 ±3), recorte fora do quad, rotação 45° (cantos e ponta), `flip_x`, blend multiply (valor exato), máscara por alpha, feather da máscara (borda suave), blur gaussiano (borda difusa).

**Escala (1080p, quads 400×300, software — tempos absolutos sem significado, o *escalonamento* sim):**

| Camadas | Memória de pico (RSS) |
|---|---|
| 1 | 136 MB |
| 10 | 279 MB |
| 50 | 914 MB |
| **200** | **3.299 MB** |

≈ **16 MB por camada a 1080p** (crescimento linear). Causa lida no código: cada camada é renderizada numa textura **do tamanho do canvas inteiro** e depois mesclada em outro passe full-frame (`render_layer` + `blend_texture`), sem *bounding box*/scissor, e o pool só recicla por frame.

## Por que não é semente

| Requisito CapIA (`PREVIEW_RENDER.md`) | OpenCut Classic compositor |
|---|---|
| Pipeline **RGBA16F linear** | **8 bits** (`Bgra8Unorm`/`Rgba8Unorm`), não linear |
| Passes limitados ao *bounding box* da camada; centenas de camadas | passes full-canvas por camada (memória/tempo O(camadas × pixels)) |
| Import de YUV com matriz/range, HDR→SDR, rotação | nenhum (recebe `wgpu::Texture` pronta) |
| Texto (engine Rust, ADR-028) | nenhum — no OpenCut o texto vem do Canvas2D do navegador |
| Transições, crop, âncora, qualidade de redução (mipmaps) | ausentes; amostragem bilinear sem mipmap (aliasing na redução) |
| Efeitos: registry com parâmetros tipados | só `gaussian-blur` (laço fixo de 61 taps); uniformes por **nome em string** (`HashMap<String,…>`); `EffectUniformValueDescriptor` **não é reexportado** (a API pública só monta efeitos via serde — API desenhada para a ponte WASM) |
| Máscara em espaço do layer/clip | máscara amostrada em **coordenadas do canvas**, canal alpha |
| API orientada a nativo, `PreviewPresenter`, compositor único preview/export | API `FrameDescriptor` serializável com `texture_id: String`, pensada para o bridge JS↔WASM |

Reaproveitar exigiria **reescrever o laço central** (`compositor.rs`, 870 linhas), trocar formato/cor, adicionar bbox, texto, YUV e transições — ficando com shaders de mistura e pouco mais. Um fork interno também seria totalmente nosso (upstream arquivado; sem atualizações de wgpu).

## O que pode ser aproveitado (com proveniência, após testes dourados próprios)

- `shaders/blend.wgsl` (142 linhas, 17 modos de mistura) — conferir fórmula/alpha (reta × pré-multiplicada) contra nossa referência;
- `masks/` (feather por SDF via *jump flooding*, ~600 linhas + 3 shaders) — técnica reutilizável;
- API de `gpu/context.rs` (init nativo/superfície) como referência de uso do wgpu 29.

Cada uso exige registro em `docs/PROVENANCE.md` e testes de paridade; o padrão é **reimplementar usando como referência**.

## Impacto

`capia-render` é escrito do zero conforme `PREVIEW_RENDER.md` (RGBA16F linear, bbox, YUV, texto). Custo de manutenção evitado: fork de código com 0 testes GPU e upstream arquivado. wgpu 29 + llvmpipe funcionam como **banco de testes de golden frames no CI sem GPU** (achado útil: `mesa-vulkan-drivers` instalável via apt).
