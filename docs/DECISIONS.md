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
**Decisão:** `tests/acceptance/timeline/*.json` (108 cenários em formato de dados, independentes de implementação) são o **critério de aceitação da Fase 2** de `capia-commands` e fonte de verdade de comportamento (placement, snapping, ripple, retime, group move, keyframes). Cada cenário tem `provenance` e, onde o CapIA diverge do OpenCut, `diverges_from_opencut`. As regras D-S7-1..8 eram **propostas**; foram **fechadas pelo PO na M05 (ADR-039)** e a suíte tem hoje 120 cenários. Nenhum teste de terceiros foi copiado.

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

### ADR-039 — Decisões S7 definitivas (D-S7-1..8), substituindo as versões PROVISIONAL
**Estado:** Aceita (Product Owner, M05) · **Substitui** o caráter provisório descrito na ADR-036.
**Contexto:** a suíte de aceitação (ADR-036) nasceu com 8 regras propostas. O PO as fechou na M05; algumas mudaram.
**Decisão:**

| # | Regra definitiva | Mudou? |
|---|---|---|
| D-S7-1 | O core aceita velocidade de clip de **0,01× a 100×**; fora disso `OUT_OF_RANGE`. A UI pode expor faixa menor. Sem time-stretch de áudio avançado agora. | **Sim** (era [1/100, 5]) |
| D-S7-2 | Inserção no interior de um clip em track magnética → `NOT_ON_BOUNDARY`, salvo `split_at_insert = true`. | Não |
| D-S7-3 | **Sem conflito global simplista.** Ripple tem `ripple_scope` (`Track`, `Tracks[...]`, `Group`, `Sequence`) e `sync_lock` por track (e rótulo de grupo). Só tracks **participantes** são deslocadas; tracks travadas ou fora do escopo ficam intactas. Se as invariantes não puderem ser preservadas → `RIPPLE_CONFLICT` **estruturado** (clips, tracks, faixa, sugestão). | **Sim** |
| D-S7-4 | O threshold de snap nasce em **pixels** da UI e é convertido para **ticks** conforme o zoom (`px / pps × 705.600.000`, racional, half-up em ticks). **Sem floor** para frames inteiros. O destino final respeita o alinhamento aplicável. | **Sim** (era floor em frames) |
| D-S7-5 | Empate de distância: `playhead > marcador > borda de clip > menor timestamp`. | Não |
| D-S7-6 | Trim **nunca** destrói keyframes fora da região visível (reestender os traz de volta). Split calcula o valor interpolado no ponto de divisão e cria o estado de fronteira em **ambas** as metades, preservando a animação. | Não (reforçada: split exato inclusive em Bézier) |
| D-S7-7 | O core **rejeita** valor inválido; a UI pode fazer clamp preventivo; o core nunca depende da UI. | Não |
| D-S7-8 | Bordas de **vídeo** que precisam de alinhamento usam **half-up** e duração mínima de 1 frame. Áudio, mapeamento de fonte e tempo interno ficam em ticks/subframe. **Não** arredondar a timeline inteira para frames. | **Sim** (escopo do arredondamento) |

**Consequências:** cenários da suíte atualizados (RTM-006, SNP-014..016) e **12 cenários novos** (RTM-018/019, RPL-021..025, SNP-017..019, KF-022/023) → **120 cenários**. O oráculo Python (`spikes/s7-oracle`) fica **congelado** (serviu para provar consistência na M03); a verdade executável é o harness Rust em `crates/capia-commands/tests/acceptance.rs`.

### ADR-040 — Modelo de escrita e invariantes da Fase 2 (escolhas de implementação)
**Estado:** Aceita (M05).
**Decisão:**
1. **Ops primitivas com entidade inteira:** `Clip/Track/Marker/Sequence/Asset { old: Option<E>, new: Option<E> }`. A inversa é trocar `old`/`new`; `apply` verifica que o estado bate com `old` (determinismo de undo/replay). O conjunto `affected` e os conflitos são em nível de **entidade** (mais conservador que "entidade+campo" de `COMMAND_SYSTEM.md` §7; refinar só se a prática mostrar falsos conflitos).
2. **Alinhamento (invariante 3 refinada):** clips em tracks **visuais** têm `start`/`duration` múltiplos do frame da sequence. Em tracks **de áudio** só se exige tick inteiro: 44,1/48 kHz não dividem a duração de frame NTSC (ex.: 23.543.520 ticks ÷ 14.700 não é inteiro), então forçar frame no áudio criaria erro de timing — coerente com D-S7-8.
3. **Tempo de conteúdo:** todo clip tem `source_in` + `speed` + `reversed` (`content_t = source_in + (t − start)·speed`; reverso inverte). Keyframes vivem em tempo de conteúdo. Conteúdo **sem fonte externa** (texto, imagem, sólido) tem "tempo local desde a criação": o `split` re-basa a metade direita em 0; `Media` e `Nested` não.
4. **Ids determinísticos:** ids não informados derivam de `SHA-256(operation_id|tipo|slot)`, então replay, re-preview e `apply_plan` produzem os mesmos ids (e o mesmo `diff_digest`).
5. **Plano:** `preview` de transação já aplicada devolve `already_applied` (sem token). Reenvio de `apply_plan` consumido devolve o **resultado original** (idempotente); `PLAN_CONSUMED` fica reservado. `apply_plan` **recomputa** a transação sobre o estado atual e exige o mesmo `diff_digest` (rebase seguro); qualquer falha de recomputação vira `PLAN_STATE_CHANGED` com a causa no `hint`.
6. **Revisão:** `Document.revision` é monotônica (commit, undo e redo incrementam). O histórico guarda só o ramo ativo; a auditoria guarda commit/undo/redo. `Sequence.revision` por sequence fica para quando houver cache de render que precise dela.
7. **JSON canônico dos digests:** `serde_json` com chaves ordenadas, compacto, `f64` no formato mais curto que faz round-trip (`float_roundtrip`) — subconjunto determinístico e **interno** de RFC 8785 (digests não são interoperáveis com terceiros).
8. **Fora desta missão (documentado em STATUS):** `permissions` por ator (Fase 4), caminhos de ref (`$seq.tracks.main`), transições, efeitos, legendas, grupos de clips, nested commands (o **modelo** e as invariantes de nested/ciclo/profundidade existem), `move` para/de track magnética (reorder, Fase 3), persistência `.capia`.

### ADR-041 — Dependências do núcleo, criptografia local e paridade nativo × WASM
**Estado:** Aceita (M05) · atualiza ADR-038 (matriz de dependências).
**Decisão:** `capia-time`/`capia-model` podem depender de `serde`; `capia-commands` de `serde` + `serde_json` (matriz de `tools/check-architecture.mjs` atualizada de propósito). **SHA-256 e HMAC-SHA-256 são implementados no próprio crate** (≈100 linhas, vetores FIPS 180-4 e RFC 4231 nos testes): nenhuma dependência de criptografia no núcleo, comportamento idêntico em WASM, superfície auditável. A chave do token de plano é **injetada** (aleatória por processo, só em memória; `Debug` do engine nunca a imprime). Testes de propriedade usam gerador xorshift **com semente** (sem `proptest`; reprodutível por semente). **Paridade nativo × WASM** (ADR-016) = `examples/parity` executado nativamente e em `wasm32-wasip1` (WASI do Node) com saídas idênticas por digest do documento (`tools/check-wasm-parity.mjs`, no CI).
**Alternativas:** `sha2`/`hmac` (RustCrypto — permissivas, mas mais dependências no núcleo); `proptest`/`wasm-bindgen-test` (mais ferramentas pesadas); só `cargo check` para WASM (não prova paridade de comportamento).
**Consequências:** o núcleo continua pequeno (2 dependências externas diretas); a corretude de SHA/HMAC depende dos vetores de teste (revisar se o uso crescer além de digests/tokens internos).

### ADR-042 — Formato `.capia` v1: SQLite com *journal* de eventos + snapshots
**Estado:** Aceita (M06) · concretiza ADR-015 · diverge de `DATA_MODEL.md` §6 em dois pontos conscientes (abaixo).
**Contexto:** a M05 entregou um engine puro com histórico, `operation_id` e plano **em memória**. A M06 precisa de durabilidade sem mover lógica de edição para a camada de persistência.
**Decisão:**
1. **Um arquivo SQLite** (`*.capia`), identificado por `PRAGMA application_id = 0x43415049` ("CAPI") e `PRAGMA user_version = <schema>`; o usuário nunca vê SQLite. Crate `capia-store` (IO, **não** vai para WASM); o núcleo puro não muda de natureza.
2. **O documento não é espalhado em dezenas de tabelas.** Estado = **snapshot** (documento em JSON canônico + cursor/ids do histórico + digest SHA-256) **+ journal de eventos** posteriores. Motivos: (a) o engine já produz ops primitivas invertíveis determinísticas; reaplicá-las é a forma mais fiel de reconstruir o estado; (b) cada commit grava poucos bytes (a entrada de histórico), não o documento inteiro (10.000 clips ≈ 2,3 MiB de JSON); (c) migrations de documento ficam em Rust sobre JSON versionado (`DATA_MODEL.md` §6 já prevê isso); (d) entidades como colunas continuam possíveis depois **sem** mudar o contrato do engine (migration 2 + projeção).
3. **Schema 1** (todas as tabelas `STRICT`, `foreign_keys=ON`):
   `meta(key,value)` · `schema_migrations(version,name,applied_at_ms)` · `history_entries(id, revision_before, revision_after, label, actor_json, transaction_id, plan_id, timestamp_ms, affected_json, entry_json)` · `events(seq, kind∈{commit,undo,redo}, entry_id, revision_after, timestamp_ms, actor_json)` · `applied_operations(operation_id PK, payload_hash, entry_id, applied_at_ms, actor_json)` · `commit_results(entry_id PK, result_json)` · `snapshots(seq PK, revision, cursor, history_ids_json, document_json, digest)`.
4. **Snapshot** na criação (seq 0) e a cada `SNAPSHOT_EVERY` eventos (256) **na mesma transação** do commit; só o último é mantido. Abrir = último snapshot + replay dos eventos seguintes + `validate_document` + conferência do digest.
5. **Migrations explícitas** numeradas (`schema_migrations` + `user_version`), forward-only, cada uma numa transação; versão **mais nova** que o software → `UNSUPPORTED_SCHEMA_VERSION` sem tocar no arquivo (diverge do rascunho "abre somente leitura": somente leitura exige uma garantia de compatibilidade de leitura que ainda não temos). Migration que invalida o histórico declara isso e reinicia o undo stack arquivando o journal (ADR-013).
6. **Metadados voláteis** (`created_at_ms`, `app_version`, timestamps de eventos) vivem em `meta`/`events`, **nunca** no documento: o digest semântico do documento é idêntico antes e depois de salvar/reabrir.
**Alternativas:** documento inteiro regravado a cada commit (simples, mas O(n) por transação e WAL inchado); tabelas por entidade já no schema 1 (mais SQL e migrations antes de haver necessidade; o engine não consulta o documento via SQL); formato próprio/binário (perde `integrity_check`, tooling e transacionalidade).
**Consequências:** o histórico completo é carregado em memória na abertura (O(#entradas); poda/carga preguiçosa ficam para quando houver projetos reais longos — registrado em STATUS); `sqlite3` consegue inspecionar o arquivo, mas o conteúdo do documento é JSON versionado, não colunas.

### ADR-043 — Persistência do histórico, undo/redo e idempotência (journal por commit)
**Estado:** Aceita (M06) · responde ao item "Parte 11" da M06 · relaciona ADR-011/029/030.
**Contexto:** o contrato do histórico (ADR-011) é uma pilha linear por projeto cujo undo aplica `inverse_ops` **gravadas**; a idempotência (ADR-029) exige `operation_id` ↔ `payload_hash` ↔ resultado original. Nada disso pode desaparecer ao fechar o projeto.
**Decisão — estratégia A (persistir o histórico completo)**, não B: toda `HistoryEntry` (ops + `inverse_ops` + resumo + `affected`) é persistida **integralmente** em `history_entries`, e a pilha (`cursor`) é reconstruída dobrando `events` (`commit` trunca o ramo de redo e empilha; `undo`/`redo` movem o cursor). Reconstruir o histórico "a partir de log mínimo" (B) exigiria recomputar comandos — o que a ADR-011 proíbe (undo não recalcula comandos).
1. **Porta única:** o `Engine` ganha o trait `Journal` (`capia-commands`, puro): antes de publicar qualquer mudança em memória ele chama `Journal::append(record)`; se falhar, **nada** muda em memória (`PERSISTENCE_FAILED`). Sem journal, o engine funciona como na M05 (testes, WASM, ghost previews).
2. `applied_operations`/`commit_results` são gravados na **mesma transação SQLite** da entrada de histórico e do evento (invariante: documento ⇔ journal ⇔ operation log nunca divergem). Replay após reopen devolve `replayed: true` com o resultado original.
3. `change_log` (detecção de `CONFLICT` por `base_revision`) e a auditoria são **derivados** de `events` + `history_entries.affected_json` — sem tabela própria.
4. O ramo de redo descartado fica no banco (auditoria) mas não volta à pilha; seus `operation_id` continuam registrados (undo não libera ids — ADR-029).
5. **Planos (`preview`) não são persistidos:** tokens não sobrevivem a reinício por desenho (ADR-030). A chave HMAC é regerada a cada abertura (`getrandom`).
6. **Retenção:** nada é podado nesta missão (ADR-029 exige ≥ 30 dias e nunca podar com Run ativo; poda por idade vem com o AI Run).
**Consequências:** `Engine::restore(EngineState)` valida consistência (cursor, ids, invariantes); undo após reopen usa as mesmas `inverse_ops` gravadas — testado.

### ADR-044 — Semântica transacional, *crash safety* e escritor único
**Estado:** Aceita (M06).
**Decisão:**
1. **Configuração SQLite (consciente, não por conveniência):** `journal_mode=WAL` (leitores — `capia inspect/validate` — não bloqueiam o escritor; recuperação por WAL após kill); **`synchronous=FULL`** por padrão (um commit reportado como sucesso sobrevive a queda de energia — necessário para idempotência durável; `NORMAL` do rascunho de `DATA_MODEL.md` pode perder o último commit em queda de energia, então só é opção explícita de testes de propriedade); `foreign_keys=ON`; `busy_timeout=5000 ms`; `trusted_schema=OFF`; `cell_size_check=ON` (detecção extra de corrupção); `wal_autocheckpoint` padrão; checkpoint `TRUNCATE` no fechamento limpo (sem `-wal`/`-shm` em repouso → "arquivo único" no disco). Enquanto aberto existem `-wal`/`-shm`: copiar o projeto exige fechá-lo (documentado).
2. **Escrita = `BEGIN IMMEDIATE` … `COMMIT`:** (a) engine valida e aplica na *working copy*; (b) `Journal::append` abre a transação, confere **head** (último `revision_after` do banco == `revision_before` do engine), grava entrada, evento, `applied_operations`, resultado e, se for o caso, o snapshot; (c) `COMMIT`; (d) só então o engine publica o novo estado. Qualquer falha em (b) → `ROLLBACK` e estado em memória intacto. Nunca há "documento salvo, operation log não".
3. **Escritor único determinístico:** o lock de escrita do SQLite serializa escritores; o segundo espera até `busy_timeout` e então recebe **`STORE_BUSY`**; um engine **obsoleto** (outro processo/handle já avançou o head) recebe **`STALE_HEAD`** (`STORE_CONFLICT`) e precisa reabrir — nada é gravado. Sem lock exclusivo do arquivo (o `inspect` precisa ler em paralelo). Colaboração real é Fase 6.
4. **Falhas injetáveis:** feature `failpoints` (apenas testes) permite abortar o processo antes do `BEGIN`, no meio da transação, antes do `COMMIT` e logo após o `COMMIT`; os testes de crash matam o **processo filho de verdade** (`TerminateProcess`/`SIGKILL`/`abort`) e reabrem o arquivo.
5. **Erros estruturados** (`capia-store`): `PROJECT_NOT_FOUND`, `PROJECT_CORRUPTED`, `NOT_A_CAPIA_PROJECT`, `UNSUPPORTED_SCHEMA_VERSION`, `MIGRATION_FAILED`, `STORE_IO_ERROR`, `STORE_BUSY`, `STORE_CONFLICT`, `TRANSACTION_FAILED`; erro cru do SQLite nunca sobe (fica em `cause` para diagnóstico). Entrada externa nunca causa `panic`.
**Consequências:** durabilidade custa um `fsync` por commit (medido; ver STATUS); `quick_check` na abertura e `integrity_check` completo em `capia validate`.

### ADR-045 — Comandos de nested sequences e `follow_length`
**Estado:** Aceita (M06) · concretiza ADR-010.
**Decisão:** novos comandos `delete_sequence` (só se nenhum `Nested` de **outra** sequence a referencia; senão `IN_USE` com a lista de clips; apaga tracks/clips/marcadores da própria sequence na mesma transação), `rename_sequence`, `insert_nested` (conveniência: duração derivada da sequence filha quando omitida), `set_nested_target` (retarget com as mesmas validações) e `set_follow_length`. Mover/remover/recortar um nested usa os comandos de clip já existentes. **Ciclo e profundidade** são checados **antes** de aplicar (erro com índice do comando, caminho do ciclo no `hint`) e continuam revalidados no fim da transação pelo grafo global (rede de segurança): `A→A`, `A→B→A` → `NESTED_CYCLE`; > 16 saltos → `NESTED_DEPTH`; alvo inexistente → `DANGLING_REFERENCE`/`NOT_FOUND`.
`follow_length` (campo novo de `ClipContent::Nested`, padrão `false`, serde-compatível) é uma **reconciliação** executada após cada comando quando existe algum clip com a flag: a duração do clip passa a `duração(filha) − source_in` (alinhada ao frame do pai, half-up, mínimo 1 frame); em track magnética os posteriores sofrem ripple; em track livre a duração é **limitada ao espaço livre** com aviso (editar o master nunca falha por causa de um overlay alheio; a reconciliação tenta de novo no próximo comando); track travada → não toca e avisa. Idempotente, determinística e atômica com o comando que a causou (um passo de undo).
**Alternativas:** propagar só no momento do `set_follow_length` (a flag mentiria); falhar a transação do master quando o pai não comporta (bloquearia edição legítima).
**Consequências:** editar um master altera clips em sequences pai **na mesma transação** (já previsto na ADR-011); fora do escopo: `make_unique`, `flatten_nested`, `create_nested_from_selection`, `duplicate_sequence`, `generate_variants` (M07).


### ADR-046 — Identidade, hash e deduplicação de assets
**Estado:** Aceita (M07) · concretiza ADR-017 / `ASSET_SYSTEM.md` §1/§4 com **uma divergência consciente** (abaixo).
**Contexto:** clips já referenciam `AssetId` (M05), mas o `Asset` do documento só tem nome/duração/flags. Falta ligar o asset a um arquivo físico sem usar o **caminho** como identidade.
**Decisão:**
1. **Três camadas, três responsabilidades.** (a) `AssetId` = identidade **interna e opaca** do asset no projeto; (b) `content_hash` = identidade do **conteúdo**; (c) `location` = onde o arquivo está **agora** (mutável). O caminho nunca identifica nada.
2. **`content_hash` = SHA-256 do arquivo inteiro, em *streaming*** (blocos de 1 MiB, nunca o arquivo na memória), formato textual `sha256:<64 hex>`. Diverge de `ASSET_SYSTEM.md` §4 (fingerprint BLAKE3 amostrado + hash completo em background): na M07 não há jobs, o hash completo é calculado **no import** (custo medido em STATUS) e o algoritmo é SHA-256 porque o projeto já o possui e verifica por vetores oficiais (ADR-029) e para não adicionar um segundo algoritmo ao núcleo; o prefixo `sha256:` deixa a troca para BLAKE3 (fingerprint amostrado) como migração futura sem ambiguidade.
3. **`AssetId` determinístico na criação:** `ast_` + 32 primeiros hex do `content_hash`. Mesmo conteúdo ⇒ mesmo id em qualquer projeto/máquina (idempotência natural, `operation_id` reproduzível). Depois de criado o id é **opaco**: nada pode inferir o hash dele (o hash vive no catálogo e pode mudar via force-relink futuro).
4. **Deduplicação: um asset por conteúdo por projeto.** Importar bytes já conhecidos **não cria asset novo**: devolve o existente (`outcome = existing`). Se o caminho for novo, ele entra em `known_paths` (alias); se o asset estava `offline`, o import de conteúdo idêntico é um **relink automático**. Mesmo nome com conteúdo diferente ⇒ assets diferentes. Metadados/`MediaInfo` não são duplicados (uma linha por hash).
5. **Arquivo alterado depois do import** não é detectado por mtime: `verify_asset` recalcula o hash; diferente ⇒ estado `modified` (o asset **não** muda de conteúdo silenciosamente; ver ADR-048).
6. **Catálogo ≠ documento.** O documento (timeline, undo) continua com o `Asset` lógico (`id`, nome, duração, flags) — o que a edição precisa. Hash, caminho, metadados de mídia e estado ficam no **catálogo do projeto** (tabelas do `.capia`, ADR-048), porque são fatos do ambiente, não edições: não entram no digest do documento nem no histórico, e `online/offline` nunca polui o undo.
**Alternativas:** hash amostrado (rápido, mas falsos positivos em vídeos que diferem só no meio — inaceitável para relink/modified); `AssetId` aleatório (dedup exigiria consulta e `operation_id` não seria reproduzível); guardar tudo no documento (digest/golden M05 quebrariam e o ambiente entraria no undo).
**Consequências:** import de arquivos grandes custa um passe de leitura (medido); trocar o algoritmo exige migration de `content_hash` (prefixado).

### ADR-047 — Backend de mídia: `capia-media`, ffprobe e execução de processos
**Estado:** Aceita (M07) · relaciona ADR-032/035.
**Decisão:**
1. **Crate `capia-media`** (IO, fora do WASM) expõe o trait `MediaProbe { fn probe(&self, path) -> Result<MediaInfo, MediaError> }`. Nenhum outro crate vê JSON/texto do ffprobe: `MediaInfo` é o modelo **normalizado** (container, streams ordenados por índice, tempo em `Ticks`, taxas em `Rational`).
2. **Implementação `FfprobeBackend`:** `ffprobe -v error -print_format json -show_format -show_streams -protocol_whitelist file -i <path>`; argumentos estruturados (sem shell), caminho como **um** argumento (`-i`), cwd neutro, stdin fechado. Saída estruturada (JSON), nunca texto humano.
3. **Limites de processo:** stdout e stderr capturados com **teto** (8 MiB / 64 KiB; excedeu ⇒ processo morto e `MEDIA_PROBE_OUTPUT_TOO_LARGE`), **timeout** (30 s padrão; excedeu ⇒ kill + `MEDIA_PROBE_TIMEOUT`), `-probesize`/`-analyzeduration` limitados. Demuxers de playlist/rede (`hls`, `concat`, `dash`, `sdp`, `rtsp`…) são **rejeitados** pelo nome do formato e o protocolo é restrito a `file`: um arquivo hostil não faz o probe tocar rede nem outros arquivos.
4. **Localização do backend (`MediaToolchain::locate`)**, em ordem: (1) caminho **configurado pelo app** (`MediaConfig`); (2) variáveis `CAPIA_FFPROBE`/`CAPIA_FFMPEG`; (3) diretório **bundled** (`<exe>/ffmpeg/` — pronto para o instalador, nada a refatorar); (4) `PATH`. Nada encontrado ⇒ `MEDIA_BACKEND_NOT_FOUND` estruturado (sem pânico). A versão detectada é registrada (`-version`) para diagnóstico e CI.
5. **Normalização determinística:** streams por `index`; seleção de stream padrão **explícita** (`default_video` = primeiro stream de vídeo que não é capa/`attached_pic`; `default_audio` = primeiro de áudio); maps ordenados; nenhum campo volátil (caminho, mtime) em `MediaInfo`. Durações decimais (`"1.234000"`) viram `Ticks` por **aritmética inteira** (sem float); frame rates `"30000/1001"` viram `Rational` reduzido; denominador 0, NaN/∞, valores negativos ou acima dos limites (duração > 1.000 h, dimensão > 65.536 px, fps > 1.000, canais > 64, sample rate > 768 kHz) ⇒ campo `None`/erro `MEDIA_METADATA_INVALID`, nunca pânico nem overflow.
6. **Imagens:** probe pelo mesmo ffprobe (`image2`/`png_pipe`/`jpeg_pipe`…); `kind=image` quando o único stream de vídeo tem formato de imagem estática (codec `png/mjpeg/webp/bmp/gif(1 frame)`, container `image2`/`*_pipe`); alpha só é afirmado quando o `pix_fmt` o declara (`rgba`, `bgra`, `ya8`, `pal8` ⇒ desconhecido). Duração de imagem **não** pertence ao arquivo (`duration = None`).
**Alternativas:** parsear texto do `ffmpeg -i` (frágil); ligar libav* (FFI/`unsafe`, licenciamento — adiado, o trait permite trocar); depender só do PATH (instalador futuro sem controle de versão).
**Consequências:** testes de lógica usam um `MediaProbe` falso; um subconjunto roda o ffprobe real (CI instala FFmpeg explicitamente — ADR-051).

### ADR-048 — Caminhos, disponibilidade (online/offline/modified), relink e catálogo no `.capia` (schema 2)
**Estado:** Aceita (M07) · concretiza `ASSET_SYSTEM.md` §5.
**Decisão:**
1. **Catálogo de mídia no `.capia`** — migration **v1→v2** (aditiva; projetos v1 migram com backup, ADR-042): tabela `media_assets(asset_id PK, kind, content_hash, size_bytes, display_name, location_json, known_paths_json, media_info_json, status, status_checked_ms, imported_ms)` (STRICT, `json_valid`), `asset_events(seq, asset_id, kind import|relink|verify|alias, detail_json, at_ms)` (append-only, auditoria). O **arquivo de mídia nunca entra** no `.capia`.
2. **Importação atômica:** a linha do catálogo e o `register_asset` do documento vão **na mesma transação SQLite** do commit do engine (a inserção do catálogo é um *efeito lateral transacional* do `Journal`): ou o asset existe nas duas camadas ou em nenhuma. Hash e probe acontecem **antes** de qualquer escrita; falha de probe ⇒ nada gravado.
3. **Caminho persistido com consciência de plataforma:** `location_json = { path: <UTF-8, exatamente como o SO o entrega, sem trocar separadores>, path_bytes_hex?: <bytes originais quando não é UTF-8>, relative?: <relativo ao diretório do projeto, **sempre com "/"**> }`. Resolução ao abrir: `relative` (join com o diretório do projeto, componentes convertidos para o SO atual) → `path` absoluto → `known_paths`. Nunca `canonicalize` (no Windows produz `\\?\`); usa-se `std::path::absolute`. Linux e Windows guardam o que têm; um `.capia` aberto no outro SO resolve pelo `relative` e cai para `offline` se o absoluto não existir.
4. **Disponibilidade é derivada, nunca editada à mão:** `online` (arquivo existe, é regular, tamanho confere) · `offline` (nenhum candidato acessível) · `modified` (acessível, mas `verify` calculou hash diferente). Abrir o projeto **não** falha por mídia ausente e **não** faz hash (O(1) por asset: `metadata` do arquivo); `verify_asset` faz o hash completo e atualiza `status`. mtime nunca decide sozinho; tamanho diferente já marca `modified` sem hash.
5. **Relink:** `relink_asset(asset, novo_arquivo)` calcula o hash do candidato (streaming): **igual** ⇒ troca a localização (o caminho antigo vira alias e o evento é registrado); **diferente** ⇒ **rejeita** com `ASSET_HASH_MISMATCH` estruturado (hash esperado × encontrado, tamanhos, caminho) — nunca equivalência silenciosa. O *force-relink* explícito (substituir o conteúdo conhecido) **não existe na M07**: exigiria reconciliar a duração/clips do documento com a nova mídia e fica para uma missão própria.
6. **Relink e undo:** relink e verify **não** são comandos do engine nem entram na pilha de undo: mudam **onde** está o byte, não a edição; desfazer uma edição não deve "desreligar" a mídia. A trilha de auditoria (`asset_events`) guarda o caminho anterior (reversível manualmente). `register_asset`/`delete_asset` (documento) continuam comandos desfazíveis.
7. **Asset em uso:** novo comando `delete_asset` ⇒ `IN_USE` (lista de clips) se algum clip o referencia; a linha do catálogo **permanece** (biblioteca) — reimportar o mesmo conteúdo reaproveita o `AssetId`. Referência a asset inexistente continua rejeitada pela invariante 8 (`validate_document`).
**Alternativas:** `offline` como campo do documento (poluiria o undo e o digest); `relink_asset` como comando (undo de relink surpreende; ambiente ≠ edição); caminhos só relativos (quebra mídia em outro disco).
**Consequências:** um projeto aberto em outra máquina mostra `offline` até o relink; o catálogo ocupa ~1 KiB/asset (medido com 10.000).

### ADR-049 — Cache derivado: `CacheKey`, diretório separado e descartabilidade
**Estado:** Aceita (M07) · §2 (layout de diretórios) e §3 (escrita atômica) **superados em parte pela ADR-057** (M08) · relaciona ADR-017 (representações regeneráveis).
**Decisão:**
1. **O cache é descartável por definição:** nada necessário para **abrir** ou **editar** o projeto existe só no cache; apagar o diretório de cache não corrompe nem altera o `.capia` (testado). O cache nunca é tratado como dado permanente (sem referência do documento/catálogo ao arquivo do cache além da chave).
2. **`CacheKey` determinística** = SHA-256 de uma codificação canônica de `(content_hash, operation, params canônicos, produtor+versão)` → `<op>/<2 hex>/<64 hex>.<ext>`; inclui a **versão do produtor** (ex.: `thumbnail/1` + versão do ffmpeg) para invalidação seletiva. Depende do **conteúdo**, não do caminho: dois projetos/paths com o mesmo arquivo compartilham a entrada.
3. **Diretório separado do `.capia`:** `CacheDir` configurável (padrão: `<projeto>.capia-cache/` ao lado do projeto; o app poderá apontar para `%LOCALAPPDATA%`); escrita atômica (arquivo temporário + rename); entrada corrompida/ausente ⇒ regenerar.
4. **Primeira operação derivada: thumbnail** (`generate_thumbnail(asset, timestamp)` via `ffmpeg`, um frame, PNG, escala limitada) — entra no cache; sem UI. Offline/arquivo alterado ⇒ erro estruturado, nunca entrada obsoleta (a chave inclui o hash vigente).
**Alternativas:** guardar thumbs no `.capia` (infla, quebra "projeto = fonte da verdade"); chave por caminho (invalida ao mover/relink).
**Consequências:** proxies, waveforms e frame index (M08+) usam o mesmo `CacheKey`.

### ADR-050 — Comandos de composição de sequences: `duplicate_sequence`, `make_unique`, `flatten_nested`, `create_nested_from_selection`, `generate_variants`
**Estado:** Aceita (M07) · completa ADR-010/045 (que adiaram estes cinco comandos).
**Contexto:** as decisões anteriores definiam o modelo (DAG de nested, profundidade ≤ 16, `follow_length`) mas não a semântica destes comandos. Todos são **composições de ops primitivas** na working copy da transação ⇒ atômicos, com undo/redo, idempotência (`operation_id`) e persistência herdados do engine; a validação de DAG/profundidade (`guard_edge`) roda antes de cada aresta nova **e** `validate_nested_graph` continua como rede de segurança (defesa em profundidade: remover só uma camada não quebra o invariante; remover as duas é detectado pelos testes).
**Decisão — ids determinísticos (nada aleatório):** o elemento `X` (track, clip, marcador) copiado para a sequence `S'` chama-se **`S'.X`**; uma sequence descendente `D` copiada em profundidade para a raiz `R'` chama-se **`R'~D`**. Ids derivados passam de 256 caracteres ⇒ `LIMIT_EXCEEDED`; colisão com id existente ⇒ erro estruturado, sem estado parcial. Quando `new_sequence`/`id` não é informado, o id deriva do `operation_id` (como `create_sequence`).
1. **`duplicate_sequence{source, new_sequence?, name?, deep=false}`** — copia cabeçalho, tracks (mesma ordem e flags), clips (com keyframes) e marcadores. Nested internos **continuam compartilhando as filhas** (cópia rasa); com `deep` copia também toda a subárvore de nested (cada descendente **uma única vez**, preservando losangos do DAG; ≤ 64 cópias).
2. **`make_unique{clip, new_sequence?, name?, deep=false}`** — o clip nested ganha uma **cópia independente** da filha (e, com `deep`, da subárvore) e passa a apontá-la (`set_nested_target` valida a aresta sem a antiga). Nenhum id é reaproveitado; os outros pais continuam apontando para a original. Track travada ⇒ `TRACK_LOCKED`.
3. **`flatten_nested{clip, prefix?}`** — substitui o clip nested pelos clips da filha, **recortados à janela visível** `[source_in, source_in+duração)`, em **novas tracks livres** (uma por track da filha, no lugar da track do clip na ordem de empilhamento; ids `prefix.<id>`, prefixo padrão = id do clip). Recorte exato como `split_clip` (fonte externa preserva keyframes em tempo de conteúdo; sem fonte rebaseia o tempo local). **Falha em vez de perder informação** (`UNSUPPORTED_COMMAND`): track magnética (ripple indefinido), `speed ≠ 1` ou reverso, propriedades/keyframes próprios do nested (não há "bake"), frame rate diferente do pai, `source_in` fora da grade. Marcadores da filha **não** são copiados (aviso). A filha continua existindo. Netos viram nested apontando direto para o neto.
4. **`create_nested_from_selection{clips, new_sequence?, name?, clip_id?, track?, follow_length=false}`** — move os clips (**mesmos `ClipId`**) para uma nova sequence (mesmo frame rate; uma track filha por track de origem, na ordem do pai) com o **tempo relativo preservado** (origem = primeiro início alinhado ao frame) e insere um nested cobrindo `[origem, último fim)` na track escolhida (padrão: a de cima entre as dos clips). Rejeita: seleção vazia/duplicada/de sequences diferentes, track travada ou magnética, id existente, sobreposição do nested com clips **não** selecionados da track destino — tudo atômico. `flatten ∘ create_nested` devolve os tempos absolutos originais (testado).
5. **`generate_variants{template, variants[{sequence?, name?, deep?, swaps[]}]}`** — operação **estrutural, sem IA**: cada variante é uma cópia (`duplicate_sequence`) da template com trocas `set_nested{clip, sequence}` (reaponta um nested) e/ou `replace_media{clip, asset}` (troca a mídia mantendo in/out; o asset precisa ter os streams que o clip usa e comportar o trecho). Os `clip` das trocas são ids da **template**. ≤ 100 variantes e ≤ 256 trocas por variante; qualquer falha desfaz **todas** as variantes. Uma variante nunca fecha ciclo (A→A rejeitado com `NESTED_CYCLE`).
**Alternativas:** ids aleatórios (replay/idempotência quebram); `flatten` assando propriedades do nested nos filhos (perde fidelidade silenciosamente); `make_unique` sempre profundo (copia mais do que o usuário pediu).
**Consequências:** o gerador de propriedade (`tests/common/nested.rs`) inclui os cinco comandos (≈ 4,7 mil aceitos em 3.000 casos) e cobre undo/redo exatos, DAG válido e reabertura do `.capia`.

### ADR-051 — Estratégia de CI: FFmpeg reprodutível e tempo do job Windows
**Estado:** Aceita (M07).
**Decisão:**
1. **FFmpeg no CI é instalado explicitamente** (Linux: `apt-get install ffmpeg`; Windows: `choco install ffmpeg`), e a versão efetiva é **impressa e registrada** no log do job; os testes que exigem o binário usam `CAPIA_REQUIRE_FFMPEG=1` (no CI a ausência **falha**, localmente os testes que dependem dele são ignorados com aviso). Fixtures de mídia são **versionadas** (poucos KiB, geradas por `tools/gen-media-fixtures.sh`) para não depender do encoder da máquina; os *golden tests* comparam só campos estáveis entre versões do FFmpeg.
2. **Tempo do job Windows:** os testes de propriedade com salvar/reabrir (fsync por commit, NTFS) passam a rodar em **release** no Linux (orçamento completo) e com orçamento menor no job Windows (*smoke*: 100 sequências por gerador), mantendo o orçamento completo (5.000 por gerador, release) no Linux. As propriedades **com IO** leem `CAPIA_IO_PROP_CASES` (as de lógica pura continuam em `CAPIA_PROP_CASES`); o `cargo test --workspace` do Windows usa `CAPIA_IO_PROP_CASES=100` (um passo release no Windows custaria mais em compilação do que economiza). Cobertura cross-platform preservada: os crash tests reais, migrations e o *smoke* de propriedade continuam no Windows.

### ADR-052 — Sistema de jobs: `capia-jobs`, prioridades sem starvation, cancelamento real, persistência e recuperação
**Estado:** Aceita (M08) · relaciona ADR-002 (core headless), ADR-029/030 (única porta de escrita).
**Contexto:** hash de arquivos grandes, índice de quadros, waveform e proxy levam segundos a minutos; não podem bloquear a thread do projeto nem a edição, precisam poder ser cancelados de verdade (matar o FFmpeg) e sobreviver a fechar/crash sem virar lixo.
**Decisão:**
1. **Crate genérico `capia-jobs`** (só `serde`/`serde_json`; não conhece SQLite, FFmpeg nem documento). `Executor` com **N workers fixos** (padrão 2–4) e **fila limitada por categoria**; estourar a fila é erro estruturado `JOB_QUEUE_FULL`, nunca thread por job.
2. **Prioridade por créditos, não por fila estrita:** categorias `interactive` > `normal` > `background` com créditos 6/3/1 por ciclo ⇒ o background **sempre** avança (teste de justiça com enxurrada de interativos). Estados: `queued/running/completed/failed/cancelled/interrupted`; progresso monotônico; resultado JSON e erro estruturado (`code/message/details`); *panic* num job vira falha `JOB_PANICKED`, nunca derruba o worker.
3. **Cancelamento cooperativo real:** `CancelToken` chega ao job; os jobs de mídia o repassam ao runner de processos (`run_streaming`), que **mata o processo filho** (teste: PID existe → cancela → PID some → temporário removido → nada publicado). Hash de arquivo checa o token a cada bloco de 1 MiB. Cancelar job na fila o remove na hora.
4. **Deduplicação:** `dedup_key` ativa (ex.: `index:<hash>`) devolve o job existente — o mesmo trabalho não roda duas vezes.
5. **Persistência (schema 3):** tabela `jobs` (estado, progresso com limite de taxa, params/resultado/erro, `cancel_requested`) escrita pelo `JobSink` do store em **conexão própria**. Nada disto é documento nem undo.
6. **Recuperação:** reabrir marca `queued/running` como **`interrupted`** (nunca `completed`) e tickets de import pendentes idem; repetir é sempre permitido. `shutdown` (fechar o projeto) faz o mesmo de forma limpa.
7. **Dono único do executor:** lock exclusivo de arquivo do SO (`<proj>.jobs-lock`, `File::try_lock`; liberado se o processo morrer). Outro processo (CLI) cancela pela flag `cancel_requested` no banco; o dono a observa em ≤ 250 ms. A recuperação só roda quando nenhum dono está vivo.
8. **Os workers só calculam.** Quem grava no documento/catálogo é a thread do projeto em `Project::pump` (via engine/catálogo) — a regra "uma única porta de escrita" fica intacta.
**Alternativas:** `tokio` (assíncrono não ajuda em trabalho de CPU/processo; peso); fila única FIFO (starvation e interativo preso atrás de proxy); persistir o job completo para retomar de onde parou (artefatos parciais são inválidos por desenho — ADR-057); thread por job (sem limite).
**Consequências:** `capia-jobs` fora do WASM; o executor é desligado (e os jobs interrompidos) ao soltar o `Project`.

### ADR-053 — Identidade de asset assíncrona: impressão rápida × SHA-256, `ImportTicket`, mudança durante o processamento
**Estado:** Aceita (M08) · estende ADR-046.
**Decisão:**
1. **Impressão rápida (`Fingerprint`)** = SHA-256 de `versão ‖ tamanho ‖ regiões amostradas` (início, fim e 14 janelas de 64 KiB; arquivo pequeno lê tudo), texto `fp1:<tamanho>:<digest>`; a **versão do algoritmo** faz parte do valor (impressões de versões diferentes são incomparáveis). Custo ≲ 1 MiB de leitura. **Nunca é identidade:** serve para triagem (dica de duplicata, candidatos de relink, checagem barata antes de derivar). Teste de **colisão deliberada**: dois arquivos de 32 MiB iguais nas regiões amostradas e diferentes fora delas ⇒ mesma impressão, SHA-256 diferente ⇒ `AssetId` diferente.
2. **A identidade continua sendo o SHA-256 completo** (ADR-046): `AssetId = ast_ + 32 hex`. Como o id só existe após o hash, o import não bloqueante devolve um **`ImportTicket`** (`ticket_id`, `job_id`, caminho, tamanho, impressão, estado `pending`, dica `possible_duplicate_of`) — **não** um id provisório (um id que muda depois contaminaria o documento). O ticket é persistido (`import_tickets`); o asset só passa a existir quando `Project::pump` finaliza o ticket (hash + probe feitos pelo job; registro pelo mesmo caminho atômico do import síncrono: documento + catálogo na MESMA transação). Estados: `pending → finalized|failed|cancelled|interrupted`; `interrupted` pode ser **retomado** (`resume_ticket`).
3. **Arquivo alterado durante o processamento:** o job compara `(tamanho, mtime)` do handle e do caminho antes/depois do hash e o total lido; qualquer diferença ⇒ `ASSET_CHANGED_DURING_PROCESSING` e **nada** é registrado nem cacheado. O mesmo vale para índice/waveform/proxy (resultado descartado antes de publicar).
4. **Derivados exigem fonte coerente:** antes de gerar, `check_source` compara tamanho + impressão rápida com o registro; divergência ⇒ `ASSET_HASH_MISMATCH` ("rode `verify`"). Limite conhecido: troca de conteúdo fora das regiões amostradas e com mesmo tamanho só é pega pelo `verify` (SHA-256).
**Alternativas:** id provisório/`PendingAsset` com troca posterior (identidade instável); aceitar a impressão como id (colisão silenciosa); hash completo antes de cada derivado (custo de leitura total por job).
**Consequências:** `AssetRecord` ganha `fingerprint` (coluna anulável, schema 3); nenhuma edição depende do ticket.

### ADR-054 — Índice de quadros `CIDX` e decode preciso
**Estado:** Aceita (M08).
**Decisão:**
1. **Índice por pacote real** (`ffprobe -show_entries packet=pts,dts,duration,flags`, lido em **streaming** com teto de linhas e de memória): cada quadro lógico tem `pts/dts/duration/keyframe` reais em unidades do *time base*; ordem de apresentação `(pts, dts)`; PTS duplicado/não monotônico é preservado (quadros distintos); PTS ausente é derivado e contado. **Nada de `quadro/fps`**: CFR, **VFR**, B-frames e GOP longo usam o mesmo caminho.
2. **Tempo relativo ao primeiro quadro apresentado**, em `Ticks` (half-up só na conversão; comparações em `i128` exatas). APIs: `frame_at_or_before`, `frame_at_or_after`, `nearest_frame` (empate ⇒ anterior), `keyframe_before`, `frame_by_index`.
3. **Formato binário versionado `CIDX` v1:** cabeçalho de 64 B (magic, versão, tamanho do cabeçalho, nº de quadros, time base, stream, PTS derivados, `start_pts`), `n × 24` B de entradas e rodapé **SHA-256**. Decodificador com limites e checagens (truncado, byte alterado, contagem absurda, time base inválido ⇒ `MEDIA_INDEX_INVALID`, sem pânico); entrada inválida no cache ⇒ regenerar. Teto 10 M quadros.
4. **Cache** chaveado por `(hash do conteúdo, stream, `INDEX_PRODUCER`+versão do ffprobe)`.
5. **Decode exato:** keyframe anterior (índice) → `ffmpeg -seek_timestamp 1 -ss <keyframe> -copyts` → filtro `select=eq(pts,N)` entrega **exatamente** o quadro com aquele PTS (se não aparecer, tenta o keyframe anterior e, por fim, o início). Saída `RawFrame{width,height,stride,RGBA8,pts,índice,tempo,bytes}` com aritmética checked e teto (256 MiB) conferidos **antes** de iniciar o processo. A rotação de exibição é metadado (não aplicada: `-noautorotate`).
**Alternativas:** seek por tempo em segundos (impreciso em VFR/B-frames); contar quadros a partir do keyframe (desalinha com PTS duplicado/descartes); índice em JSON (gigante); decoder in-process (dependência/licença — ADR-032).
**Consequências:** fixtures com quadros identificáveis (luma = função de N) provam o quadro certo em CFR/GOP longo/VFR.

### ADR-055 — Áudio PCM normalizado e waveform multirresolução
**Estado:** Aceita (M08).
**Decisão:**
1. **Decode de áudio** para **f32 intercalado** na taxa/canais pedidos (padrão: os do stream). Posições em **amostras** (inteiros) convertidas de `Ticks` por aritmética inteira (início = floor, fim = ceil); 705.600.000 é múltiplo de 44.100 e 48.000 ⇒ exato. O decode parte do início do stream, descarta até `start`, entrega `duration` e **mata o ffmpeg** ao terminar (exato por construção; custo O(start) aceito: áudio decodifica ≫ tempo real). Pedir além do fim devolve só o que existe (nunca preenche). Teto de bytes e checagem de forma (taxa/canais) antes de iniciar.
2. **Waveform `CWFM` v1:** pirâmide de buckets `(min, max, rms)` sobre o áudio **mixado em mono** na taxa nativa; nível 0 = 64 amostras/bucket, cada nível agrega 4 do anterior até 1 bucket; construção em streaming (memória O(buckets)); NaN/inf viram silêncio e amostras são limitadas a [-1, 1]; rms ponderado pelo nº real de amostras. Cabeçalho com taxa, total de amostras e nº de níveis, **rodapé SHA-256**; o leitor confere contagem de buckets por nível, finitude e `min ≤ max`. Consulta `query(início, fim, buckets)` escolhe o nível mais fino compatível. Falha do decode ⇒ nenhum waveform (nunca parcial).
**Alternativas:** waveform por canal (dobra o custo sem uso na V1); PCM i16 (perde headroom/normalização); `-ss` no ffmpeg (preciso só ao milissegundo, não à amostra).

### ADR-056 — Proxy de edição: `ProxyProfileV1` e política de encoders
**Estado:** Aceita (M08) · aplica ADR-032 (LGPL, sem GPL/nonfree).
**Decisão:**
1. **`ProxyProfileV1`** = `{max_width, max_height, fps (keep|fixed), codec, jpeg_quality, h264_kbps, gop, audio}`; validado; seu JSON canônico entra na `CacheKey`.
2. **Codec padrão: MJPEG** (encoder **nativo** do FFmpeg, LGPL, intra-only ⇒ scrub/seek quadro a quadro baratos) em `.mov`, áudio AAC nativo; escala sem ampliar, dimensões pares, `yuv420p`. `fps = keep` usa `-fps_mode passthrough`: **os quadros do proxy coincidem com os do original** (teste VFR: mesma contagem e mesmos instantes).
3. **H.264 só por encoder de hardware/SO** (`h264_nvenc|qsv|amf|mf` no Windows, `videotoolbox` no macOS, `nvenc|qsv|vaapi` no Linux), escolhido por `select_encoder` a partir de `ffmpeg -encoders`; ausente ⇒ `MEDIA_ENCODER_UNAVAILABLE` estruturado. **`libx264`/`libx265` nunca são usados** (teste).
4. **O proxy nunca é autoritativo:** a timeline e o export referenciam o original; o proxy é cache descartável. Progresso por `-progress pipe:1`; cancelar mata o ffmpeg e apaga o parcial; o resultado é validado com o probe (dimensões ≤ perfil) antes de publicar.
**Alternativas:** H.264 por libx264 (GPL, ADR-032); ProRes/DNxHD (licença/patente de encoder); proxy como substituto no export (viola "timeline = fonte da verdade").

### ADR-057 — Cache derivado v2: produção atômica, lock por chave, validação e GC
**Estado:** Aceita (M08) · **substitui** a organização de diretórios do ADR-049 §2 (o restante do ADR-049 segue).
**Decisão:**
1. **Layout** `<cache>/<op>/<16 hex do conteúdo>/<chave>.<ext>`: agrupa tudo de um asset (GC e invalidação por conteúdo sem listar o banco); `.tmp/` e `.locks/` na raiz do cache (mesmo volume ⇒ `rename` atômico).
2. **`produce(key, validate, write)`:** toma o **lock da chave** → se há entrada **válida**, *hit* → senão escreve num **temporário**, **valida** (checksum/formato/probe), `fsync` e **`rename`** para o final. Falha, cancelamento ou validação ruim ⇒ temporário apagado e **nada publicado**; um final bom anterior nunca é tocado antes do `rename`. Crash deixa só lixo em `.tmp/` (varrido por `sweep_temp`/`cache clean`), nunca um final parcial.
3. **Lock por chave:** exclusão em-processo (conjunto + condvar, acorda quem espera, cancelável) + **lock consultivo do SO** (`File::try_lock`: `flock`/`LockFileEx`) no arquivo `.locks/<chave>.lock` entre processos. O lock pertence ao descritor, **não** ao arquivo: um processo morto o libera sozinho. *(Bug achado pelos crash tests: a 1ª versão usava um arquivo `create_new` com expiração de 10 min — um kill durante o índice deixava a chave presa e o retry travava; o lock do SO elimina a classe de problema.)* Dez produtores simultâneos da mesma chave geram **uma** vez (teste de concorrência); chaves diferentes não se bloqueiam.
4. **Validação na leitura:** `get_valid` apaga entrada corrompida e trata como *miss* (checksum do `CIDX`/`CWFM`, probe do proxy).
5. **GC:** `usage` (arquivos/bytes por operação + sobras temporárias), `remove_unused(conteúdos vivos)`, `invalidate_content(hash)` (force relink/arquivo modificado), `clear`. Nenhum desses toca o `.capia`; o projeto continua válido (testado).
**Alternativas:** escrever direto no final (crash deixa parcial válido-aparente); só lock em-processo (CLI × app concorrentes duplicam trabalho); GC por idade (apaga o que ainda é usado).

### ADR-058 — Force relink, relink em lote e `update_asset`
**Estado:** Aceita (M08) · estende ADR-048 §5 (relink só por conteúdo) sem alterá-lo.
**Decisão:**
1. **Force relink** (explícito, nunca por import): analisa o arquivo novo (probe + SHA-256 + impressão) e troca o conteúdo do asset **mantendo o `AssetId`**. O lado do documento é o **novo comando `update_asset`** (desfazível, `operation_id`), que **valida todos os clips dependentes**: o trecho de fonte consumido tem de caber na nova duração e os streams que o clip usa têm de existir; qualquer violação ⇒ `CONFLICT` com a lista estruturada (um item por clip, motivos `SOURCE_RANGE_EXCEEDS_NEW_MEDIA`/`MISSING_VIDEO_STREAM`/`MISSING_AUDIO_STREAM`) e **nada muda — nunca trim silencioso**. Documento + catálogo na MESMA transação SQLite; derivados do conteúdo antigo são invalidados; os aliases antigos são descartados (descreviam outro conteúdo); evento `force_relink` na trilha. `--dry-run` roda a mesma validação sem gravar. Conteúdo já pertencente a **outro** asset ⇒ `FORCE_RELINK_DUPLICATE_CONTENT` (um conteúdo, uma identidade). Mesmo conteúdo ⇒ relink comum. *Undo* reverte só os metadados lógicos do documento (o catálogo, por ADR-048 §6, não entra no undo).
2. **Relink em lote por pasta** (job `batch_relink`): varredura → candidatos por **tamanho** → filtro por **impressão rápida** → **SHA-256 completo decide**. **Nunca por nome.** Resultado estruturado: `matched` (único arquivo verificado ⇒ aplicado em `pump`), `unresolved`, `ambiguous` (≥ 2 cópias idênticas ⇒ **nada** aplicado), `rejected` (mesma impressão, SHA-256 diferente), `errors` (arquivo ilegível/permissão), mais contadores (varridos, hashes feitos, descartados pela impressão). Reconfere tamanho+impressão antes de aplicar.
3. **Varredura segura:** profundidade máxima (16), nº máximo de arquivos (500 mil), **symlinks/junctions não seguidos** por padrão (Windows: atributo de reparse point), laços detectados por diretório canônico visto, erros de permissão registrados sem abortar, ordem determinística, cancelável.
**Alternativas:** relink por nome/duração (colide com cópias e retakes); force relink recortando clips automaticamente (perda silenciosa); relink ambíguo escolhendo "o primeiro" (esconde decisão do usuário).

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
