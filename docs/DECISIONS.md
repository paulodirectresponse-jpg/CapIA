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
**Status:** Accepted · **Evidência (M03/S5):** canvas virtualizado manteve 60 fps com 10.000 clips visíveis (JS 2,1 ms p50) só com raster por CPU; DOM sem virtualização caiu a ~8 fps no zoom — `docs/spikes/S5-timeline-canvas.md`
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
**Status:** Accepted (M03, spike S4 — `docs/spikes/S4-wasm-core.md`), condicionada a: teste de paridade nativo×WASM por hash em CI sobre o modelo real; API WASM com f64 nos limites de tempo; medir cold-start do módulo e IPC real no S1/Fase 3
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
**Status:** Accepted (licenciamento fechado na ADR-032)
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

### ADR-029 — `operation_id` em todo comando (idempotência)
**Status:** Accepted (M03, decisão do Product Owner)
**Contexto:** AI Runs, REST/MCP e retries de rede podem reenviar uma transação. Só `base_revision` (ADR-014) protege contra conflito, não contra **duplicação** (ex.: Run retomado após crash reaplica 147 operações). O Timeline Studio (auditoria M02) usa ids de operação idempotentes; adotamos a ideia, reimplementada e estendida.
**Decisão:** todo comando carrega `operation_id` (≤128 chars, único no projeto). O engine registra `applied_operations(operation_id, payload_hash, history_entry_id)` na **mesma transação SQLite** do commit. Reenvio com todos os ids conhecidos e mesmo payload → resultado original (`replayed: true`), sem reaplicar; mesmo id com payload diferente → `OPERATION_ID_REUSED`; mistura de conhecidos/novos → `OPERATION_ID_CONFLICT`. Ids da IA são determinísticos (`run_id+stage+índice`). Undo **não** libera ids (idempotência da submissão, não do efeito). Retenção ≥ 30 dias e nunca enquanto houver Run retomável.
**Alternativas:** idempotência só por `transaction_id` (rejeitada: reenvio parcial/reordenado); deduplicar por hash do plano (rejeitada: mesmo plano legítimo reaplicado depois de undo seria bloqueado sem controle do chamador).
**Consequências:** nova tabela e verificação no commit; `COMMAND_SYSTEM.md` §4.1; Fase 2 testa replay/conflito/crash no meio do commit.

### ADR-030 — `preview → apply_plan` vinculado por token (plan token)
**Status:** Accepted (M03, decisão do Product Owner)
**Contexto:** a IA precisa revisar o diff antes de aplicar e **nada pode permitir aplicar silenciosamente um plano diferente do revisado** (bug, race, retry, manipulação). `dry_run` + `commit` com o plano reenviado não garante isso.
**Decisão:** `preview(tx)` normaliza a transação (JSON canônico RFC 8785), calcula `plan_digest` e `diff_digest` (ops primitivas resultantes), guarda o plano revisado num *preview store* em memória (TTL 15 min) e devolve `plan_token = plan_id ‖ HMAC-SHA256(K, plan_id‖plan_digest‖diff_digest‖base_revision‖actor‖scope‖expires_at)` com `K` aleatória por processo, só em memória. `apply_plan(plan_token)` recebe **só o token**, verifica HMAC (tempo constante), validade, uso único e ator, e aplica o plano **guardado**. Se a revisão mudou, só prossegue se o rebase produzir o **mesmo `diff_digest`**; senão `PLAN_STATE_CHANGED` (novo preview; aprovação humana se invalida). Atores `Agent` e `Api` só escrevem por este caminho (`PREVIEW_REQUIRED`); `User/System` podem usar `execute` direto.
**Alternativas:** assinar o plano e reenviá-lo no apply (rejeitada: o chamador ainda controla o corpo; maior superfície); chave persistida (rejeitada: vazamento de chave e tokens longevos); sem token, só `base_revision` (rejeitada: não prova que o plano aplicado é o revisado).
**Consequências:** estado em memória a limitar; tokens não sobrevivem a reinício (re-preview); `COMMAND_SYSTEM.md` §4.2; AI Tool System (`timeline.preview`/`apply_plan`); aprovação humana referencia `plan_id+diff_digest`. **Limite:** garante integridade preview→apply no mesmo processo; não impede um plano ruim submetido por quem tem permissão (isso é validação, permissões e undo).

### ADR-031 — Estratégia de reutilização C e política de proveniência
**Status:** Accepted (M03, decisão do Product Owner; auditoria em `OPEN_SOURCE_AUDIT.md`)
**Decisão:** **core próprio + reutilização seletiva de comportamento, testes, padrões e componentes legalmente compatíveis**. Sem fork de OpenCut, Timeline Studio ou outro editor como base. Classes `SAFE_TO_REUSE / SAFE_WITH_OBLIGATIONS / REFERENCE_ONLY / DO_NOT_USE`. Proveniência **obrigatória** (repositório, commit/tag, arquivo original, licença, alterações, copyright) para qualquer código reutilizado; nada de `REFERENCE_ONLY`/`DO_NOT_USE` é copiado; ideias podem ser reimplementadas (clean-room). Política e registro: **`docs/PROVENANCE.md`**.
**Consequências:** nenhum código de terceiros incorporado até aqui; verificação de licenças no CI (M04); métricas de acompanhamento passam a ser marcos, critérios de aceitação, testes, riscos eliminados, blockers, retrabalho e custo de sessões (não pessoas-semanas).

### ADR-032 — FFmpeg: LGPL, dynamic linking, build própria mínima; sem GPL/nonfree (fecha OD-2)
**Status:** Accepted (M03, direção do Product Owner; evidência em `docs/spikes/S3-ffmpeg-lgpl.md`)
**Decisão:**
1. Build padrão **LGPL**, **bibliotecas compartilhadas** (DLLs), carregadas dinamicamente; **nenhum** componente GPL ou `--enable-nonfree`; **sem x264/x265**; sem `--enable-gpl`.
2. **Build própria e mínima**, **sem `--enable-version3`** (resulta em LGPL-2.1+), `--disable-autodetect --disable-network --disable-programs` (CLI só como sidecar opcional), componentes habilitados explicitamente. **Não** consumir a build BtbN "como está": ela é **LGPL-3.0** (`version3`), tem ~70 bibliotecas externas (manifesto/superfície), e o manifesto do OpenCut a rotula incorretamente como 2.1.
3. **Origem e build pinadas e reproduzíveis:** tarball/tag + SHA-256 + commit (ex.: `n8.1.3`, `1041abdc…`), receita versionada no repositório, toolchain fixa, CI gera artefatos; reprodutibilidade bit a bit é meta a verificar (não verificada no S3).
4. **Encoders de sistema/hardware atrás de abstração** (`VideoEncoder` trait): NVENC/AMF/QSV (via headers permissivos) e Media Foundation (`h264_mf`/`hevc_mf`) como caminho principal em Windows; **fallback por software**: H.264 via OpenH264 (Constrained Baseline) e HEVC via kvazaar (lento); ProRes via `prores_ks` quando permitido.
5. **Manifesto de licenças** (`THIRD_PARTY_LICENSES`) gerado a cada release, com texto da licença, versão, flags e fonte da build.
**Riscos registrados (não resolvidos por engenharia):** patentes/royalties de H.264/HEVC/AAC; OpenH264 compilado do fonte **não** tem a cobertura de patentes da Cisco (que vale para o binário distribuído por ela, obtido em runtime); obrigações LGPL (oferta do fonte da build, permitir substituir DLLs); `prores_ks` (implementação independente); dependência de drivers/SO para encoders HW.
**Pendente (M04/Fase 2):** build Windows própria, habilitação e teste dos encoders HW, teste de reprodutibilidade, política de download do binário OpenH264.

### ADR-033 — Baseline de hardware e plataforma V1 (fecha OD-3)
**Status:** Accepted (M03, decisão do Product Owner; **revisável por benchmark real nas próximas fases**, não é requisito imutável)
**Decisão:** suporte planejado **Windows 11 x64**. **Máquina de referência mínima** para testes: CPU moderna de 4+ cores, 16 GB RAM, GPU compatível com DirectX 12/wgpu (inclui iGPU moderna), SSD; edição alvo principal **1080p**. **Recomendada:** 8+ cores, 32 GB RAM, GPU dedicada com 6 GB+ de VRAM, NVMe. Metas de desempenho (`TEST_STRATEGY.md` §8) valem na máquina mínima, salvo indicação.
**Nota:** o CI em nuvem usa GPU de software (WARP/llvmpipe) para correção (golden frames), **não** para desempenho; benchmarks de desempenho exigem máquinas reais.

### ADR-034 — Compositor próprio (`REIMPLEMENT_WITH_REFERENCE`)
**Status:** Accepted (M03, spike S6 — `docs/spikes/S6-compositor.md`)
**Decisão:** `capia-render` escreve seu compositor wgpu conforme `PREVIEW_RENDER.md`; o compositor do OpenCut Classic **não** é semente de núcleo (8 bits não linear, passes full-canvas por camada ≈ 16 MB/camada a 1080p, sem YUV/texto/transições/bbox, API orientada ao bridge WASM). Pode servir de referência; `blend.wgsl` e `masks/` são candidatos a adaptação com atribuição (MIT) após testes dourados próprios, registrados em `PROVENANCE.md`.
**Achado útil:** wgpu 29 roda em llvmpipe (Vulkan por software) → golden frames no CI sem GPU.

### ADR-035 — O engine é dono da seleção de frame e da conformação de cadência
**Status:** Accepted (M03, spike S2 — `docs/spikes/S2-frame-exact.md`); reforça a ADR-007
**Contexto:** em mídia sintética VFR/CFR, o seek `-ss` do ffmpeg devolveu o frame **seguinte** em 99% dos casos (até 75 frames de erro com pausas) e o filtro `fps` divergiu de sample-and-hold em 13–38% dos frames ao conformar para 23,976/24/25/50 fps.
**Decisão:** preview e export usam **a mesma** função de seleção de frame (`frame_at` sample-and-hold sobre *frame index* próprio, decode por ordinal) e conformam a cadência no compositor; **nem `-ss` nem o filtro `fps`** do ffmpeg determinam timing. Conversão pts→Ticks por aritmética racional de 128 bits (`pts×num×705.600.000/den`, arredondamento definido; time bases como 1/15360 e 1/12288 não dividem o timebase). O demux/decode mantém o tratamento de *edit list* do libav (ignorá-la causa +21 ms de dessincronia). Rotação é aplicada pelo engine.
**Consequências:** suíte de conformidade de mídia (`/testdata`, gerada por script) em `capia-media`.

### ADR-036 — Suíte de aceitação de comportamento da timeline como ativo normativo
**Status:** Accepted (M03, spike S7 — `docs/spikes/S7-timeline-acceptance.md`)
**Decisão:** `tests/acceptance/timeline/*.json` (108 cenários em formato de dados, independentes de implementação) são o **critério de aceitação da Fase 2** de `capia-commands` e fonte de verdade de comportamento (placement, snapping, ripple, retime, group move, keyframes). Cada cenário tem `provenance` e, onde o CapIA diverge do OpenCut, `diverges_from_opencut`. As regras D-S7-1..8 (S7) são **propostas** a confirmar pelo Product Owner; mudanças nelas alteram cenários, não código. Nenhum teste de terceiros foi copiado.

### ADR-037 — Gates de fase: Fase 2 inicia com OD-1 aberto; OD-1 é hard gate da Fase 3
**Status:** Accepted (M04, decisão do Product Owner)
**Contexto:** o S1 (presenter do preview em Tauri/WebView2) exige Windows 11 com WebView2 e GPU; a sessão de nuvem não consegue executá-lo (`docs/spikes/S1-preview-surface.md`). O motor headless da Fase 2 (modelo, comandos, store, mídia, render, export, CLI) não depende do presenter: o `PreviewPresenter` é um trait e o compositor produz frames independentemente do destino.
**Decisão:**
```
Fase 2 — Motor (headless)   pode iniciar com OD-1 aberto.
Fase 3 — Editor/Preview     NÃO pode iniciar sem OD-1 fechado.
```
Fechar OD-1 = relatório do S1 executado em Windows (`tools/s1-preview-spike`), analisado pela regra de decisão de `S1-preview-surface.md` §4, e um ADR de presenter. `OUTPUT-H264` (caminho confiável de exportação MP4/H.264 no Windows) deve estar definido **antes da entrega do Editor** (saída da Fase 3).
**Consequências:** na Fase 2 não se constrói preview embutido nem UI de editor (só o trait e um *frame sink* headless); se o S1 mostrar que P1 e P2 são inviáveis, a revisão da ADR-001 afeta **apenas** a Fase 3; a Fase 2 não é retrabalhada.

### ADR-038 — Estrutura do scaffold: crates, pacotes e ferramentas
**Status:** Accepted (M04)
**Decisão:**
- **Crates Rust** (workspace Cargo, direção única): `capia-time → capia-model → capia-commands → capia-project`; o shell `apps/desktop/src-tauri` (`capia-desktop`) depende **só** de `capia-project`. `capia-time/model/commands` não têm dependências de terceiros, não fazem IO e compilam para WASM (ADR-016, verificado no CI).
- **`capia-project`** assume, no scaffold, o papel de fachada do núcleo que a M01 chamou de `capia-store` + `capia-engine` (persistência `.capia`/SQLite, sessão de projeto, Engine API). Se crescer, divide-se por novo ADR. Os demais crates da M01 (`capia-media`, `-assets`, `-render`, `-preview`, `-jobs`, `-secrets`, `-ai`, `-gateway`, `-cli`, `-server`) **não** são criados vazios: nascem quando sua fase começa e entram na matriz de arquitetura.
- **Pacotes TS** (pnpm): `packages/engine-bindings` (= `engine-client` da M01; contrato tipado, **sem** Tauri/React), `packages/editor-ui` (React; depende apenas da interface `EngineClient`), `apps/desktop` (Vite + Tauri; único ponto que conhece o Tauri). `ui-timeline`, `design-system` e `core-wasm` nascem na Fase 3.
- **Fronteiras verificadas por máquina:** `tools/check-architecture.mjs` (matriz de dependências permitidas por crate/pacote, ciclos, imports proibidos de Tauri/wgpu/ffmpeg/IA/providers no núcleo e de `@tauri-apps` na UI) + `deny.toml` + job WASM do CI. Adicionar crate/pacote exige atualizar a matriz **de propósito**.
- **Contrato Rust ↔ TypeScript:** fixture JSON compartilhada (`packages/engine-bindings/fixtures/engine_info.json`) testada nos dois lados, até haver geração de tipos.
- **Toolchain pinada:** Rust `1.97.0` (`rust-toolchain.toml`, com `wasm32`), Node ≥ 22, pnpm 10 (workspace + *catalog*), **TypeScript 6.0.3** (o `typescript-eslint` ainda não suporta TS 7; revisar quando suportar), Vitest, ESLint flat `strictTypeChecked`, Prettier. Lints do workspace: `unsafe_code = forbid`, `unwrap_used = warn`.
- **Shell Tauri:** capabilities mínimas (`core:default`), CSP estrita, `bundle.active = false` (sem instalador até a Fase 6); ícone é **placeholder** gerado por script, não identidade visual.
- **Licenças:** `cargo-deny` (allow-list; GPL/AGPL/non-commercial falham) + `tools/check-licenses.mjs` (JS). MPL-2.0 é *permitido para revisão* (5 crates Rust do Tauri e `lightningcss` no build do Vite, hoje); `exceptions` humanas ficam vazias.
**Consequências:** o grafo é pequeno e auditável; o preço é manter a matriz atualizada a cada crate novo.

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
**Estado (M04): ABERTA — hard gate da Fase 3, não da Fase 2 (ADR-037).** Pacote de medição pronto em `tools/s1-preview-spike/` (`.\run.ps1` em Windows 11). *(M03:)* S1 **não pôde ser medido** no ambiente da sessão (sem Windows/GPU/WebView2). Evidência documental e de proxy, protocolo de medição e **regra de decisão** definidos em `docs/spikes/S1-preview-surface.md`. Achado verificado: o `wry` 0.57 (Tauri 2.12) só faz *windowed hosting* do WebView2 (sem visual hosting/DirectComposition).
Opções: P1 superfície nativa filha (preferida), P2 WebView2 SharedBuffer + canvas. **Como decidir:** spike S1 com os critérios de `PREVIEW_RENDER.md` §5. Se nenhuma atender, reavaliar ADR-001 (ex.: shell nativo com UI web embutida em região, ou Qt).

### OD-2 — Modelo de licença do produto e build do FFmpeg
**Estado (M03): FECHADA pela ADR-032** (direção do PO: LGPL, dynamic linking, sem GPL/nonfree). Riscos jurídicos/patentes registrados na ADR; modelo de licença **do repositório** do produto continua a definir pelo PO antes do primeiro release.
Se o produto for **comercial e de código fechado** (recomendação implícita do contexto): FFmpeg LGPL com linkagem dinâmica, sem x264/x265; H.264/HEVC via encoders de hardware (NVENC/QSV/AMF) com fallback OpenH264 (qualidade inferior) ou licenciamento comercial de encoder; avaliar royalties de patentes (AAC, HEVC). Se **open-source GPL**: x264/x265 liberados. Também define a licença do repositório. **Quem decide:** o dono do produto (decisão de negócio), com aconselhamento jurídico.

### OD-3 — Baseline de plataforma e hardware de referência
**Estado (M03): FECHADA pela ADR-033.**
Proposta: Windows 10 22H2+ e Windows 11, x64 (ARM64 depois); GPU com D3D12 (feature level 11_0+); 16 GB RAM; hardware de referência para metas de desempenho: notebook com CPU de 8 núcleos (≈2021+) e GPU integrada Intel Iris Xe **e** uma máquina com GPU NVIDIA dedicada. Afeta backend wgpu, decode/encode HW e metas de `TEST_STRATEGY.md` §8.

### OD-4 / OUTPUT-H264 — Caminho confiável de exportação MP4/H.264 no Windows
**Estado (M04): ABERTA — não resolvida de propósito.** Precisa estar definida **antes da entrega do Editor** (saída da Fase 3). Requisitos: encoder de hardware quando disponível (NVENC/AMF/QSV); Media Foundation/FFmpeg quando adequado; fallback por software **legal e de qualidade aceitável**; **sem x264/x265 GPL** no build padrão (ADR-032). Fatos e incógnitas em `docs/STATUS.md` ("OUTPUT-H264"). Decide: Product Owner, com apoio jurídico para patentes (H.264/HEVC/AAC e binário OpenH264).
