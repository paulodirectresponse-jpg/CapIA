# DECISIONS — Architecture Decision Records

Formato: cada ADR tem **Status** (`Proposed` | `Accepted` | `Superseded by ADR-x`), contexto, decisão, alternativas e consequências. Não reescreva ADRs aceitos: crie um novo que o substitua.

Seções: **A. ADRs** · **B. Requisitos reformulados** · **C. Decisões abertas (bloqueantes para a Fase 2)**

---

## A. ADRs

### ADR-001 — Stack: Tauri 2 + React + TypeScript + Rust + SQLite + FFmpeg, com condições
**Status:** Accepted (condicionado ao spike S1 — ver OD-1)
**Contexto:** Desktop Windows primeiro, multiplataforma depois; mídia local pesada; GPU; integração com IA; manutenção por equipe pequena.
**Avaliação:**
| Critério | Tauri+React+Rust | Electron+React+Node/C++ | Qt/QML+C++ | UI 100% Rust (egui/iced/slint) |
|---|---|---|---|---|
| Performance de mídia | Nativo em Rust (excelente) | Nativo via addons C++ (bom, mais atrito) | Excelente | Excelente |
| UI profissional densa / velocidade de iteração | Excelente (ecossistema web) | Excelente | Boa, mais lenta | Fraca/imatura |
| Memória/distribuição | Pequena (WebView2 do sistema) | Grande (Chromium embutido) | Média | Pequena |
| GPU para preview | wgpu nativo (precisa resolver apresentação na janela) | Mesmo problema | Nativo e simples | Nativo e simples |
| Windows | WebView2 evergreen, maduro | Ótimo | Ótimo | Ok |
| Multiplataforma futura | Sim (WebKit em mac/Linux — por isso nada crítico depende da WebView) | Sim | Sim | Sim |
| Integração IA | Rust (HTTP simples; segredos fora do JS) | Node (SDKs prontos) | C++ (pior) | Rust |
| Licenciamento | MIT/Apache | MIT | LGPL/comercial | MIT/Apache |
**Decisão:** manter a stack candidata **com as condições C1–C5** de `ARCHITECTURE.md` §2 (core headless; preview/export nativos; timeline em canvas; IA+segredos em Rust; core compilável para WASM).
**Consequências:** o maior risco é a apresentação do preview na janela (airspace), mitigado por `PreviewPresenter` e spike. Electron não resolveria esse risco (mesmo problema) e custaria mais memória.

### ADR-002 — Core headless em Rust com Engine API única
**Status:** Accepted
**Decisão:** toda lógica vive em crates Rust sem dependência de Tauri; UI, CLI, REST, MCP e a IA são clientes da mesma Engine API com `Actor` identificado.
**Consequências:** testes sem UI; automação externa (Fase 6) sem reescrita; Tauri substituível.

### ADR-003 — Preview e export nativos (libav + wgpu) com compilador e compositor únicos
**Status:** Accepted
**Alternativas:** WebCodecs+WebGPU na WebView; MLT/GStreamer; frames por IPC — ver `PREVIEW_RENDER.md` §2.
**Decisão:** um compilador timeline→render graph e um compositor wgpu servem preview e export; FFmpeg/libav faz demux/decode/encode/mux, nunca a composição.
**Consequências:** equivalência semântica por construção; precisamos de um text engine e shaders próprios (escopo controlado de efeitos).

### ADR-004 — IA e credenciais executam no core Rust
**Status:** Accepted
**Decisão:** providers, router, tools, orquestração e acesso ao `SecretStore` ficam em `capia-ai`/`capia-secrets`. A WebView nunca recebe segredos.
**Alternativa rejeitada:** orquestração em TS na WebView ou sidecar Node (SDKs oficiais) — expõe chaves ao JS e duplica a fronteira de segurança; não roda headless de forma limpa.
**Consequências:** adapters HTTP próprios (APIs são simples e estáveis o suficiente); custo de manter formatos de cada provider.

### ADR-005 — Timeline UI própria em canvas
**Status:** Accepted
**Decisão:** `packages/ui-timeline` próprio (canvas + virtualização + camada de gestos), sem biblioteca de timeline de terceiros como base. Ver `TIMELINE_UX.md` §6.
**Consequências:** mais trabalho inicial; controle total sobre desempenho, track magnética, nested e integração com o estado autoritativo do core.

### ADR-006 — Projeto = Demanda; pastas organizam; Deliverables exportam
**Status:** Accepted
**Decisão:** hierarquia de edição `Workspace → Project → Sequence → Track → Clip`. A árvore "AD 1 → Hook 1/2/3, Body" é **organização** (Folders) + composição (nested), não um nível de edição. `Deliverable` = sequence + preset de export + regra de nome.

### ADR-007 — Tempo em Ticks inteiros (1/705.600.000 s) + Rational
**Status:** Accepted
**Alternativas:** float seconds (rejeitado: erro acumulado, NTSC inexato); rational por valor (`value/rate` como no OTIO com floats) — comparações e somas exigem normalização constante; frames da sequence como unidade (quebra com nested de fps diferente e áudio).
**Decisão:** `Ticks(i64)` com timebase divisível por todas as taxas usuais de vídeo e áudio; `FrameRate` racional; clips alinhados a frame na V1; regra única `frame_at` (sample-and-hold) para CFR/VFR. Ver `TIMELINE_ENGINE.md` §1.

### ADR-008 — Tracks: duas famílias (Visual/Audio) + role opcional
**Status:** Accepted
**Decisão e justificativa:** `TIMELINE_ENGINE.md` §2. Texto, legendas e nested são clips visuais; role guia UX/IA sem criar tipos rígidos.

### ADR-009 — Áudio embutido no clip de mídia, com detach
**Status:** Accepted
**Decisão:** clip de mídia carrega componentes vídeo+áudio (modelo mental do CapCut, zero dessincronia); `detach_audio` separa. Linked clips estilo Premiere foram rejeitados para V1 (mais propagação, mais risco de dessincronia, menos familiar ao público-alvo).

### ADR-010 — Nested sequences como referência viva
**Status:** Accepted
**Decisão:** referência compartilhada por padrão; `make_unique` (cópia profunda), `flatten_nested`, `follow_length`; DAG validado; profundidade ≤ 16; sem "pinned revision" na V1 (campo reservado). Ver `TIMELINE_ENGINE.md` §4.

### ADR-011 — Histórico de undo linear por projeto
**Status:** Accepted
**Contexto:** edições num master propagam para várias sequences; pilhas por sequence tornariam undo inconsistente.
**Decisão:** uma pilha por projeto; filtro visual por sequence; undo seletivo por ator só na Fase 5 e só sem interseção de entidades.

### ADR-012 — UX multi-sequence: árvore de projeto + abas abertas + `+`
**Status:** Accepted
**Decisão:** árvore = organização persistente; abas = navegação. `+` cria instantaneamente com formato herdado e renomeação inline. Ver `TIMELINE_UX.md` §2.

### ADR-013 — Command Engine: comandos semânticos → primitive ops com inversas; documento imutável; single writer
**Status:** Accepted
**Decisão:** comandos expandem para ops primitivas; histórico guarda ops e inversas; documento com compartilhamento estrutural e revisões; um actor por projeto serializa escritas.
**Consequências:** undo independente da lógica futura dos comandos; snapshots baratos para preview/render/IA; após migration de schema o histórico anterior é arquivado (undo não atravessa migrations) — trade-off aceito.

### ADR-014 — Transações com `base_revision`, rebase automático e conflito explícito
**Status:** Accepted
**Decisão:** ver `COMMAND_SYSTEM.md` §7. Necessário porque IA e usuário editam concorrentemente.

### ADR-015 — Formato de projeto: arquivo SQLite único `.capia`
**Status:** Accepted
**Alternativas:** pasta com JSONs (diffável, mas sem atomicidade multi-arquivo e lenta em projetos grandes); JSON único (reescrita completa a cada save; ruim para autosave contínuo); bundle de pasta com SQLite dentro (menos portátil para o usuário comum).
**Decisão:** `.capia` (SQLite WAL) com commit durável por transação; mídia gerenciada em `Nome.capia-media\`; cache regenerável em AppData; snapshots JSON canônico internos; backups rotativos externos. Ver `DATA_MODEL.md` §2 e §6.

### ADR-016 — Crates puros do core compilados para WASM e usados pela UI
**Status:** Proposed (validação no spike S4)
**Decisão:** `capia-time`, `capia-model`, `capia-commands` sem IO, compilados para WASM; a UI usa-os para ghost previews de gestos e snapping, aplicando a mesma lógica do core.
**Alternativa:** reimplementar lógica em TS (rejeitada: divergência inevitável); round-trip IPC a cada movimento do mouse (latência).

### ADR-017 — Identidade de assets: Asset lógico → AssetVersion → MediaFile por fingerprint
**Status:** Accepted
**Decisão:** ver `ASSET_SYSTEM.md` §1 e §4. Clips nunca referenciam caminhos.

### ADR-018 — Asset Gateway com adapters substituíveis, preferencialmente fora do processo
**Status:** Accepted
**Decisão:** ver `ASSET_SYSTEM.md` §6. O editor só conhece a API do Gateway.

### ADR-019 — Provider abstraction, Brain Profile e Capability Router
**Status:** Accepted
**Decisão:** formato canônico de chat/tools; adapters por família de API; Brain Profile com overrides e fallbacks; router com regra "Brain primeiro em Auto"; requisitos mínimos para ser Brain. Ver `AI_PROVIDERS.md`.

### ADR-020 — Credenciais: SecretStore com Windows Credential Manager e vínculo de host
**Status:** Accepted
**Decisão:** ver `SECURITY.md` §2–§3.

### ADR-021 — Tool system seguro por construção
**Status:** Accepted
**Decisão:** tools declaradas com schema/permissão; nenhuma tool de shell, filesystem arbitrário, HTTP genérico ou settings. Escritas na timeline apenas via transações.

### ADR-022 — Agentes como papéis do Brain + pipeline em state machine com gate de escrita
**Status:** Accepted
**Decisão:** ver `AI_SYSTEM.md` §3–§4. Papéis podem usar modelos diferentes via Brain Profile, mas não precisam.

### ADR-023 — Memória em 4 escopos sem promoção automática
**Status:** Accepted
**Decisão:** ver `AI_SYSTEM.md` §9.

### ADR-024 — Uso do FFmpeg
**Status:** Accepted (licenciamento em OD-2)
**Decisão:** libav in-process para decode/preview/export; CLI como sidecar para jobs batch isolados; build única versionada; nunca como engine de composição.

### ADR-025 — Keyframes ancorados no tempo de conteúdo do clip
**Status:** Accepted
**Decisão:** ver `TIMELINE_ENGINE.md` §3 (mover/trimar não desloca a animação em relação ao conteúdo).

### ADR-026 — Transições consomem handles, não alteram a duração
**Status:** Accepted
**Decisão:** ver `TIMELINE_ENGINE.md` §3. Sem handles: duração limitada; se zero, freeze de borda com warning.

### ADR-027 — Job system persistente unificado
**Status:** Accepted
**Decisão:** ver `ARCHITECTURE.md` §8.

### ADR-028 — Text engine em Rust compartilhado por preview e export
**Status:** Accepted (biblioteca concreta escolhida no início da Fase 2: cosmic-text/swash vs Skia)
**Decisão:** nenhum texto é rasterizado pela WebView para fins de render.

---

## B. Requisitos reformulados (o pedido original tinha ambiguidade ou problema técnico)

| # | Requisito original | Problema | Solução adotada (intenção preservada) |
|---|---|---|---|
| R-1 | Projeto com árvore `AD → Hook/Body` e hierarquia `Project → Sequence` | A árvore mistura organização com composição; se "AD" fosse um nível de edição, o modelo ficaria rígido | Pastas para organizar + nested para compor + Deliverables para exportar (ADR-006) |
| R-2 | Tracks "Text, Captions, Nested Sequences" ao lado de V/A | Tipos rígidos impedem intercalar z-order e multiplicam casos especiais | Famílias Visual/Audio + `role` (ADR-008) |
| R-3 | "Preview e final render podem usar tecnologias diferentes" | Tecnologias diferentes de composição tornam equivalência semântica praticamente impossível de garantir | Mesmo compilador/compositor; só presenter/encoder diferem (ADR-003) |
| R-4 | Usuário cadastra capabilities, context limits, custo | Declarações manuais podem estar erradas e quebrar o pipeline em runtime | Presets + probe automático + marca "não verificado" |
| R-5 | "Escolher qual modelo será o cérebro" (qualquer um) | Modelos sem tool calling/contexto suficiente não conseguem orquestrar | Requisitos mínimos para Brain, validados na seleção |
| R-6 | IA aplica "centenas de operações atomicamente" | Atomicidade sozinha não resolve edição concorrente do usuário durante o Run | `base_revision` + rebase automático + `CONFLICT` (ADR-014); limites de ops |
| R-7 | Nested com "versionamento" | Referência fixada a revisões exige manter todas as revisões vivas de uma sequence — alto custo, baixo valor na V1 | `make_unique` + snapshots; campo `pinned_revision` reservado |
| R-8 | Windows Credential Manager para as chaves | Protege em repouso/entre usuários, não contra malware do mesmo usuário; limite de tamanho | Aceito com abstração `SecretStore`, vínculo de host, chaves só no core; limite documentado |
| R-9 | "Nunca enviar a chave a outro provider" | Sem mecanismo, depende de disciplina de código; `base_url` editável pode redirecionar a chave | Vínculo credencial↔host aplicado no cliente HTTP, inclusive em redirects |
| R-10 | Lock/mute/solo/hide como lista única | Nem todos se aplicam a todas as tracks | Hide = visual; mute = áudio; solo e lock = ambas |
| R-11 | "Registrar correções e padrões" | Aprendizado implícito contamina preferências globais | Só propostas; promoção com aprovação (ADR-023) |
| R-12 | Reference Analyzer "requisito da V1" | "V1" ambíguo com a fase | V1 = produto ao fim da Fase 6; analyzer entregue na Fase 4 |
| R-13 | `media.fetch(url)` para TikTok/Instagram/YouTube | Downloaders violam ToS de várias plataformas e são frágeis; risco legal | Adapters opcionais e substituíveis, fora do processo, com proveniência e flag de licença (ADR-018) |
| R-14 | "Autosave" | Autosave periódico perde trabalho entre intervalos | Commit durável por transação; `Ctrl+S` = checkpoint + snapshot |
| R-15 | "Experiência muito próxima ao CapCut" | Proximidade excessiva de trade dress/nomenclatura é risco legal; o nome "CapIA" também se parece com "CapCut" | Paridade **funcional** com design próprio; nome é provisório e precisa de verificação de marca antes de lançamento |
| R-16 | Transições "entre clips" (sem semântica de duração) | Editores diferem: sobreposição que encurta a timeline vs. uso de handles | Handles, duração inalterada (ADR-026) |
| R-17 | Undo/redo com IA + usuário | Undo "da IA" após edições manuais pode desfazer trabalho do usuário | Linear na V1; seletivo por ator só sem interseção (Fase 5) |

---

## C. Decisões abertas (precisam ser fechadas antes da Fase 2)

### OD-1 — Presenter do preview na janela Tauri/WebView2
Opções: P1 superfície nativa filha (preferida), P2 WebView2 SharedBuffer + canvas. **Como decidir:** spike S1 com os critérios de `PREVIEW_RENDER.md` §5. Se nenhuma atender, reavaliar ADR-001 (ex.: shell nativo com UI web embutida em região, ou Qt).

### OD-2 — Modelo de licença do produto e build do FFmpeg
Se o produto for **comercial e de código fechado** (recomendação implícita do contexto): FFmpeg LGPL com linkagem dinâmica, sem x264/x265; H.264/HEVC via encoders de hardware (NVENC/QSV/AMF) com fallback OpenH264 (qualidade inferior) ou licenciamento comercial de encoder; avaliar royalties de patentes (AAC, HEVC). Se **open-source GPL**: x264/x265 liberados. Também define a licença do repositório. **Quem decide:** o dono do produto (decisão de negócio), com aconselhamento jurídico.

### OD-3 — Baseline de plataforma e hardware de referência
Proposta: Windows 10 22H2+ e Windows 11, x64 (ARM64 depois); GPU com D3D12 (feature level 11_0+); 16 GB RAM; hardware de referência para metas de desempenho: notebook com CPU de 8 núcleos (≈2021+) e GPU integrada Intel Iris Xe **e** uma máquina com GPU NVIDIA dedicada. Afeta backend wgpu, decode/encode HW e metas de `TEST_STRATEGY.md` §8.
