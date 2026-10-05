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

### ADR-059 — Serviço de decode persistente: pool de sessões, workers e prioridade
**Estado:** Aceita (Fase 2 — conclusão) · estende ADR-054 (decode exato) sem alterá-lo.
**Contexto:** o decode exato da M08 abre **um processo por chamada** (74–88 ms; 3,5 ms/quadro só quando em intervalo). Reprodução e scrub precisam reaproveitar o processo.
**Decisão:**
1. Novo crate `capia-decode` (depende só de `capia-time` e `capia-media`; fora do WASM). `FrameStream` (em `capia-media`) é uma **sessão persistente**: um `ffmpeg` que decodifica para frente a partir de um quadro lógico e entrega RGBA8 cru **em ordem**, com contrapressão (o processo bloqueia no pipe quando ninguém lê) e `Drop` que mata o processo. O n-ésimo quadro entregue é o `first + n` do `FrameIndex` (ordenado por PTS) — nunca `N/fps`. Tentativas de pouso: keyframe anterior → keyframe antes dele → início do arquivo.
2. `DecodeService`: **N workers = N sessões máximas**. Cada sessão é por `(namespace de projeto, conteúdo, stream)`. Um pedido usa, nesta ordem: cache → sessão do pool posicionada **até `forward_gap` quadros antes** do alvo (decodifica e descarta/cacheia os intermediários) → sessão nova (expulsando a ociosa menos recente se o pool está cheio). Sessão com erro/fim morre; sessão ociosa por `idle_timeout` é encerrada (`evict_idle`).
3. **Prioridade:** `Interactive > Playback > Background`, FIFO dentro da classe; sem prevenção de starvation para o *background* (é só prefetch descartável). **Cancelamento** por bilhete; **supersession por pista** (`Lane`): um pedido novo na pista torna **todos** os anteriores obsoletos — os enfileirados são pulados, o em andamento é abortado entre quadros e, mesmo que termine, seu resultado vira `Superseded` (nunca entregue como o quadro atual). A sessão interrompida volta íntegra ao pool.
4. **Métricas** (`DecodeMetrics`): pedidos por prioridade, supersedidos/cancelados/falhos, quadros decodificados, sessões abertas/reaproveitadas/expulsas (ociosas e por pressão), cache (hits/misses/evições/bytes), tempo de decode e de espera na fila, taxa de reaproveitamento.
5. A fonte é **sempre o original**: o serviço não conhece proxy. `shutdown` resolve os pendentes como `Shutdown` e mata as sessões.
**Medido (release, Linux):** sequencial 1080p mpeg4 = 28,9 ms/quadro com **1** sessão aberta e 119 reaproveitamentos (taxa 0,99); seek aleatório sem cache = 166 ms; cache *hit* = 0,3 ms.
**Alternativas:** processo por quadro (o estado anterior); biblioteca de decode em processo (libav* via FFI — adiaria a política LGPL/`unsafe_code = forbid`); um único processo global (sem isolamento por asset).

### ADR-060 — Cache de quadros por bytes, chave por conteúdo, prefetch e supersession de scrub
**Estado:** Aceita (Fase 2 — conclusão).
**Decisão:**
1. `ByteLru<K, V>`: LRU com **orçamento em bytes** (não em entradas). `insert` expulsa os menos recentes até caber; um item maior que **todo** o orçamento é rejeitado sem expulsar nada; contadores de hits/misses/evições/rejeições.
2. Chave do quadro: `(namespace, hash de conteúdo, stream, PTS, formato de pixel, versão do backend)`. **Conteúdo trocado ⇒ hash diferente ⇒ miss**; `invalidate_content/namespace` removem entradas e sessões ociosas. O `namespace` (hash do caminho do projeto) impede contaminação entre projetos que compartilham o processo, mesmo com o mesmo conteúdo.
3. **Prefetch** (`prefetch(src, from, dir, lane)`, tarefa `Background`): `Forward` = próximos `prefetch_ahead` quadros; `Backward` = até `backward_window` quadros anteriores (limitado); `Jump` = nada (o destino do salto é pedido interativo). O prefetch passa pelo mesmo caminho do pedido (mesma sessão, mesmo cache); *interativo vence background* na fila.
4. **Scrub rápido t1→t4:** com `Lane`, só o último pedido é entregue; o `ProjectSource` implementa `MediaSource::cancel_pending` (cancela os bilhetes em voo) e o scheduler de preview descarta o quadro obsoleto (ADR-066).
**Provas:** `capia-decode/tests/service.rs` (13 testes: LRU por bytes, orçamento nunca excedido, hit com a mesma alocação, conteúdo trocado, multi-stream, concorrência, sem contaminação entre projetos, prefetch, prioridade, supersession, cancelamento, ociosidade, shutdown); mutações #2/#3/#4.

### ADR-061 — Índice de frames de áudio (`CAIX`) e seek rápido com pouso verificado
**Estado:** Aceita (Fase 2 — conclusão) · resolve a limitação "decode de áudio O(início)" do ADR-055.
**Decisão:**
1. `AudioIndex`: para cada **frame decodificado** `(pts, amostra inicial, nº de amostras)` (ffprobe `-show_frames`); a posição de amostra é a **soma real de `nb_samples`** (inclui priming/skip do decoder), nunca `pts × taxa`. Formato binário `CAIX` v1 (cabeçalho 64 B, entradas de 24 B, rodapé SHA-256; truncagem e alteração rejeitadas; frames não contíguos/não monotônicos rejeitados). `AudioStream` ganhou `time_base` (opcional; registros antigos relêem do arquivo). Cache `aix` por `(hash, stream, produtor+ffprobe)`; job `audio_index` (cancelável, persistido, dedup).
2. **Seek:** `-seek_timestamp 1 -noaccurate_seek -ss <µs arredondados para cima>` num frame `SEEK_MARGIN_FRAMES` (6) antes do alvo + pré-roll; descarta até a amostra exata. Só para contêineres de seek exato (`mov/mp4/m4a/3gp/3g2/mj2/wav/aiff`); os demais decodificam do início.
3. **Pouso verificado:** em mp4 com vídeo o demuxer pousa no keyframe de **vídeo** anterior (o áudio acompanha). O `ffmpeg` roda com `-copyts -af ashowinfo`; o `pts` do 1º frame entregue é lido do stderr e **conferido contra o índice** (igualdade exata). Se o pouso não é verificável, ou é posterior ao alvo, ou a janela (`alvo + folga de 4 s`) não alcança, a chamada decodifica do início (sempre exata). `AudioDecodeStats` expõe `decoded_frames/seeked/fell_back_to_start`.
4. **Exatidão:** PCM é **bit-exato** contra o decode do início; AAC é exato em **posição** e igual em valor até ~3·10⁻³ (estado do decoder/PNS no ponto de seek) — por isso a tolerância documentada de 10⁻² nos testes lossy, que ainda enxerga um deslocamento de 1 amostra (verificado no próprio teste).
**Medido:** 1 s em t = 0/60/300 s de WAV: 65/77/76 ms, **o mesmo ~5,3 s de janela decodificada** nos três (antes: 1,55 s em t = 300 s, O(início)). Teste `seek_cost_does_not_scale_with_the_start`; mutação #6.
**Limite:** GOP de vídeo longo (> 4 s) em mp4 com vídeo cai no *fallback* (correto, lento).

### ADR-062 — Cache de PCM por blocos com orçamento em bytes e read-ahead
**Estado:** Aceita (Fase 2 — conclusão).
**Decisão:** `PcmCache` (em `capia-decode`) guarda blocos de 16.384 amostras (`ByteLru`, chave `(namespace, conteúdo, stream, bloco, versão)`) na taxa/canais **nativos**; `read` monta qualquer intervalo em amostras inteiras (fronteiras de bloco, início ≠ 0, clipes curtos, além do fim ⇒ curto/vazio, blocos adjacentes) decodificando cada corrida contígua de blocos ausentes com **uma** chamada e lendo `PCM_READ_AHEAD_BLOCKS` (7) à frente quando a corrida vai até o fim do pedido. A reamostragem (`resample_sinc`) e o remix de canais para a saída acontecem no adaptador do projeto, não no cache.
**Medido:** 30 blocos sequenciais de 1 s: 26 ms/bloco (com read-ahead; 87 ms sem), 10 chamadas ao decoder; *hit* quente 0,02 ms.

### ADR-063 — Render graph (`capia-render`)
**Estado:** Aceita (Fase 2 — conclusão) · realiza o escopo "render graph" do ROADMAP.
**Decisão:** crate **puro** (depende só de `capia-time`/`capia-model`; `capia-commands` só como dev-dependency; compila para WASM). `RenderGraph::compile(doc, root)` congela a sequence raiz e as sequences alcançáveis por nested numa forma imutável (`BTreeMap` ⇒ ordem determinística), validando profundidade ≤ 16. `plan_video(seq, t)` devolve os layers visíveis de baixo para cima (primeira track = baixo): clip ativo por track, `content_time` (start, source_in, speed `Rational`, reversed), propriedades avaliadas **no tempo de conteúdo**, nested recursivo com o tempo da sequence filha. A mídia entra por um trait (`MediaSource`: quadro por tempo de fonte, imagem estática, PCM na taxa de saída); o crate nunca faz IO. APIs: `render_frame`, `render_video_range` (cadência da sequence ou das configurações; quadro `n` em `n × duração` exatos), `mix_audio_range`, todas com aritmética *checked*. Conteúdo fora do escopo (texto) gera o aviso determinístico `TEXT_NOT_RENDERED`.

### ADR-064 — Semântica do compositor CPU de referência
**Estado:** Aceita (Fase 2 — conclusão) · não revoga o ADR-034 (compositor wgpu da Fase 3): esta é a **referência** contra a qual o wgpu será comparado.
**Decisão:** RGBA8, alfa **reto**. Base **preto opaco**. Fonte *over* em inteiros com *half-up* (`blend_over`); opacidade do clip quantizada a 0–255 e multiplicada no alfa da fonte. Posicionamento *fit/contain* da fonte na tela, depois `scale` (em torno do centro) e `position_x/y` (pixels da tela, relativos ao centro); rotação só em **múltiplos de 90°** (outros ângulos ⇒ aviso, sem interpolar); reamostragem bilinear em ponto fixo sobre pixels pré-multiplicados com cobertura baseada no centro do pixel (sem *halo*) e caminho rápido para o caso identidade. Nested = canvas filho transparente composto como layer. Áudio: f32, soma por clip com ganho `volume_db` em blocos de 256 amostras, *mute/solo* de tracks, **hard clip** em ±1, retime por *varispeed* (interpolação linear por posição racional), reamostragem *sinc* (janela de Hann) quando a taxa difere. O digest de quadro é SHA-256 próprio (sem dependência) das dimensões + bytes. Tudo determinístico: sem float na decisão de tempo, sem paralelismo não ordenado.
**Fora do escopo:** texto, transições, efeitos, máscaras, rotação arbitrária, gerência de cor além de sRGB/BT.709 no export.

### ADR-065 — Semântica de cor e alfa
**Estado:** Aceita (Fase 2 — conclusão).
**Decisão:** pixels de trabalho são **RGBA8 sRGB, alfa reto** (sem espaço linear na referência; o RGBA16F linear do ADR-034 é do compositor wgpu). Imagens com alfa (PNG) entram com o alfa original; vídeo é opaco. Na saída para MP4: `scale=out_color_matrix=bt709:out_range=tv,format=yuv420p` e tags BT.709/limited explícitas (`-colorspace/-color_primaries/-color_trc/-color_range`); o decode de entrada usa o `rgba` padrão do ffmpeg (a matriz da **fonte** é a do metadado; fontes sem tag seguem o padrão do ffmpeg — limitação documentada). O proxy nunca entra na cadeia de cor do export.

### ADR-066 — Contrato do preview headless (`capia-preview`)
**Estado:** Aceita (Fase 2 — conclusão) · não cria o preview visual (Fase 3, gate OD-1).
**Decisão:** crate sem projeto/mídia/SQLite (depende de `capia-time`, `capia-model`, `capia-render`). `PreviewScheduler` = **um worker de render** + um `FrameSink`. `request(t)` (seek/scrub) substitui o pedido pendente e chama `MediaSource::cancel_pending`; o quadro cujo render já estava em curso é **descartado** ao terminar (`DropReason::Superseded`). `play(from, speed)` usa um `Clock` injetável (em **ticks**; `SystemClock`/`ManualClock`): o quadro do instante é `floor(tempo de reprodução / duração do quadro)`; se ao terminar já há quadro posterior no prazo, o atrasado é descartado (`DropReason::Late`) — nunca acumula atraso nem volta no tempo; chegar ao fim ⇒ `PlayState::Ended`. `pause` fixa o playhead. O preview usa o **mesmo** `render_frame` do export: a paridade é por construção e provada com digests exatos (`capia-project/tests/parity.rs`). `HeadlessSink` registra digest/tempo/índice/avisos; o trait `FrameSink` é o ponto de encaixe do presenter da Fase 3.

### ADR-067 — `EncoderCapability`, política de encoders e estratégia de export MP4
**Estado:** Aceita (Fase 2 — conclusão) · aplica o ADR-032 (sem GPL/nonfree). **Não fecha `OUTPUT-H264`** (decisão de produção/jurídica do PO).
**Decisão:**
1. **Catálogo fechado** (`EncoderCapability`): backend (NVENC, QSV, AMF, Media Foundation `h264_mf`, OpenH264, VideoToolbox, VAAPI, V4L2, MPEG-4 nativo, x26x GPL), codec, hardware/software, **política** (`Approved`, `ApprovedWithConditions` = OpenH264 — a cobertura de patentes exige o binário da Cisco —, `Prohibited`), compilado?, **disponível?** (teste real de codificação de 2 quadros), motivo da indisponibilidade (linha útil do stderr), *pixel formats* (`-h encoder=`), perfis declarados. `capia media encoders` imprime tudo.
2. `libx264`, `libx264rgb`, `libx265` e **qualquer nome fora do catálogo** são **proibidos**: `select_export_encoder` recusa por nome (`MEDIA_ENCODER_PROHIBITED`), a escolha automática nunca os considera e `EncodeSession::start` repete a checagem (defesa em profundidade). **Sem fallback silencioso**: pedido de encoder indisponível ⇒ `MEDIA_ENCODER_UNAVAILABLE` com o motivo; H.264 sem encoder aprovado **não** vira outro codec.
3. **`mpeg4-reference`** (MPEG-4 Parte 2 nativo, LGPL) existe só como pedido **explícito** para provar mux, contagem de quadros e sincronismo onde não há encoder H.264 aprovado; o relatório diz o codec real. Não substitui H.264 e não resolve `OUTPUT-H264`.
4. VA-API fica declarado `indisponível` (exige `hwupload`, não implementado na Fase 2). Qualidade/bitrate padrão: H.264 8 Mb/s; AAC 192 kb/s.
5. **Validação (ffprobe) antes de publicar:** contêiner mp4, codec, dimensões, fps **racional exato**, nº de pacotes de vídeo == nº de quadros, duração de vídeo ≈ quadros × duração do quadro (±1 quadro), áudio (taxa/canais) e **drift A/V ≤ 1 quadro**.
**Estado medido:** esta VM (ffmpeg 6.1.1 distro) compila NVENC/QSV/VAAPI/V4L2 mas nenhum funciona (sem driver/dispositivo) e tem x264/x265 (proibidos) ⇒ **nenhum H.264 aprovado disponível aqui** — o caminho H.264 foi provado apenas pelo contrato (falha estruturada, sem publicar). **Windows (CI, `windows-latest`, ffmpeg do chocolatey):** `h264_mf` (Media Foundation) está **disponível**; NVENC/QSV/AMF compilados mas sem hardware (`nvcuda.dll`/MFX/`amfrt64.dll` ausentes); OpenH264 não compilado; x264/x265 presentes e proibidos. O teste de aceitação (`phase2_e2e`) exportou ali **MP4 H.264 real** via `h264_mf`, validado por ffprobe (codec `h264`, 64×48, 30/1 fps, 90 quadros, áudio 48 kHz estéreo, drift A/V ≤ 1 quadro). *Capacidade de engenharia provada*; a decisão de **produção/jurídica** (patentes H.264/AAC, qualidade/bitrate do `h264_mf` em software, binário OpenH264) segue **pendente — `OUTPUT-H264` continua aberta**.

### ADR-068 — Export atômico: staging → validação → rename
**Estado:** Aceita (Fase 2 — conclusão) · mesma disciplina do cache (ADR-057).
**Decisão:** todo export escreve numa **área de staging ao lado do destino** (`<destino>.partial-<pid>-<n>`), valida e só então publica com `rename` (atômico no mesmo volume). Erro, cancelamento ou kill ⇒ **o destino nunca existe parcial**; a sobra do staging é varrida na próxima execução do mesmo destino (`clean_stale_partials`, com repetição no Windows por causa de antivírus/handles). Destino existente ⇒ `EXPORT_EXISTS` salvo `overwrite` (que move o antigo para o lado antes de publicar e o restaura se a publicação falha). **Intermediário** (`capia-intermediate-v1`): `video.rgba` (RGBA8 cru), `audio.wav` (float32 IEEE), `manifest.json` (sequence, range em ticks, fps racional, dimensões, SHA-256 do vídeo e do áudio, **digest de cada quadro**, avisos) — tamanhos conferidos antes de publicar. **MP4:** quadros entram por pipe no ffmpeg (sem arquivo gigante), áudio de um WAV temporário no staging; `fsync` antes do `rename`. Pontos de falha (`export_frames_running`, `export_before_publish`) existem só com a feature `failpoints` e alimentam os crash tests.
**Provas:** `export.rs` (7), `crash_phase2.rs` (7 cenários com kill real), mutação #12.

### ADR-069 — S1 medido: presenter do preview = P2 (SharedBuffer → canvas WebGL); P1 eliminado
**Estado:** Aceita (fecha OD-1) · **condição residual obrigatória** abaixo · evidência: `docs/spikes/S1-preview-surface.md` §6 (3 execuções do harness corrigido em runner Windows com WebView2/DWM reais).
**Contexto:** o primeiro teste em Windows 11 não mediu nada por defeitos do harness (PowerShell 5.1/encoding, HWND filho em thread sem message loop, deadlock de `Mutex`, readback sem timeout, janela maior que a área útil da tela). Corrigidos (sem alterar critérios). O runner `windows-latest` tem WebView2, DWM e desktop interativo, mas **adaptador WARP (GPU por software)**.
**Aplicação da regra de decisão (§4 do spike, definida antes dos números):**
1. **P1 não passa.** Com o HWND filho (swapchain D3D12 viva: ~62 fps, 0 erros de surface) **abaixo** do WebView2 transparente, o padrão **nunca aparece** (grade 3×3 inteira = cor de fundo da página; `ms_until_pattern_visible = null`; 0 amostras válidas de latência), reproduzido em todas as execuções. O controle (mesmo HWND **acima**) mostra o padrão em 26–39 ms — a sonda e o posicionamento estão corretos. É o "airspace": o WebView2 em *windowed hosting* cobre a região com a própria árvore Chrome/D3D e a transparência compõe com o fundo da janela-pai, não com HWNDs irmãos. Isso é propriedade da arquitetura de hospedagem (o wry só faz windowed hosting, verificado no spike), **não da GPU**; portanto uma GPU real não muda o resultado (inferência registrada como tal).
2. **P2 passa os critérios funcionais e de latência:** padrão visível em 18–37 ms; overlay HTML composto sobre o preview (192,64,64 = mistura exata esperada); input — clique em botão HTML sobre o preview e no preview recebidos, 0 cliques no HWND nativo (sem airspace); resize contínuo sem amostra fora do retângulo esperado (padrão correto 47–92 ms após cada passo; 1º passo frio 304 ms); fullscreen com frame decodificado da tela; latência submissão→tela p50 **36 ms** / p95 48 ms (960×540) e p50 36,5 / p95 48 (1280×720); 60,0 fps a 540p e 57,4 fps a 720p (59 de 1807 slots perdidos, 3,3 %) **em WARP**, readback p50 5,5 ms (540p) / 10,9 ms (720p); custo do lado web desprezível (JS de upload+draw p50 0,3 ms; processos do WebView2 ≈ 0,15 % da máquina).
3. **Não mensurável no runner (declarado, não fabricado):** CPU% e GPU% do pipeline com **GPU real** (no runner o rasterizador WARP roda **dentro** do processo medido: 47 % da máquina a 720p, 24 % a 540p — número confundido, **não** é evidência a favor nem contra o critério CPU < 25 %); pacing/readback em GPU/PCIe reais; DPI 150/200 %; múltiplos monitores; flicker visual.
**Decisão:** presenter do preview = **P2**. P3 (janela de preview destacável nativa) permanece recurso adicional (dual monitor); P1 como presenter embutido está **descartado**. P4 (visual hosting/fork do wry) **não** é necessário agora — só se o gatilho abaixo disparar.
**Condição residual (critério de aceitação, não nova decisão):** a primeira entrega de preview da Fase 3 executa `tools/s1-preview-spike` (`powershell -ExecutionPolicy Bypass -File .\run.ps1 -P2Res 1280x720`) em PC com GPU real (e, se disponível, 150 % de DPI / 2 monitores). **Gatilho de reabertura:** P2 a 720p com CPU > 25 % da máquina atribuível ao pipeline de apresentação (readback + cópia + JS; excluindo o rasterizador de software), ou > 1 % de slots perdidos, ou artefato de resize/DPI → reabrir OD-1 (P4 ou revisão da ADR-001). Os critérios do S1 **não foram alterados**.
**Consequências:** `PreviewPresenter` ganha a implementação `SharedBufferCanvas` (`wgpu → readback assíncrono → SharedBuffer → texSubImage2D`); resolução de preview ajustável (540p/720p) e `frames_that_missed_their_slot` vira métrica permanente; a Fase 3 pode iniciar (gate OD-1 satisfeito), ainda sujeita a `OUTPUT-H264` na saída.

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
**Estado: FECHADA pela ADR-069 — presenter = P2 (SharedBuffer → canvas WebGL); P1 eliminado (airspace medido).** Condição residual: validação de CPU/pacing em GPU real como critério de aceitação da primeira entrega de preview da Fase 3, com gatilho de reabertura definido na ADR-069. Evidência em `docs/spikes/S1-preview-surface.md` §6. *(Histórico: aberta até a correção do harness; achado verificado — `wry` 0.57 só faz *windowed hosting* do WebView2.)*

### OD-2 — Modelo de licença do produto e build do FFmpeg
**Estado (M03): FECHADA pela ADR-032** (direção do PO: LGPL, dynamic linking, sem GPL/nonfree). Riscos jurídicos/patentes registrados na ADR; modelo de licença **do repositório** do produto continua a definir pelo PO antes do primeiro release.
Se o produto for **comercial e de código fechado** (recomendação implícita do contexto): FFmpeg LGPL com linkagem dinâmica, sem x264/x265; H.264/HEVC via encoders de hardware (NVENC/QSV/AMF) com fallback OpenH264 (qualidade inferior) ou licenciamento comercial de encoder; avaliar royalties de patentes (AAC, HEVC). Se **open-source GPL**: x264/x265 liberados. Também define a licença do repositório. **Quem decide:** o dono do produto (decisão de negócio), com aconselhamento jurídico.

### OD-3 — Baseline de plataforma e hardware de referência
**Estado (M03): FECHADA pela ADR-033.**
Proposta: Windows 10 22H2+ e Windows 11, x64 (ARM64 depois); GPU com D3D12 (feature level 11_0+); 16 GB RAM; hardware de referência para metas de desempenho: notebook com CPU de 8 núcleos (≈2021+) e GPU integrada Intel Iris Xe **e** uma máquina com GPU NVIDIA dedicada. Afeta backend wgpu, decode/encode HW e metas de `TEST_STRATEGY.md` §8.

### OD-4 / OUTPUT-H264 — Caminho confiável de exportação MP4/H.264 no Windows
**Estado (M04): ABERTA — não resolvida de propósito.** Precisa estar definida **antes da entrega do Editor** (saída da Fase 3). Requisitos: encoder de hardware quando disponível (NVENC/AMF/QSV); Media Foundation/FFmpeg quando adequado; fallback por software **legal e de qualidade aceitável**; **sem x264/x265 GPL** no build padrão (ADR-032). Fatos e incógnitas em `docs/STATUS.md` ("OUTPUT-H264"). Decide: Product Owner, com apoio jurídico para patentes (H.264/HEVC/AAC e binário OpenH264).

---

## Fase 3 — Editor manual (ADR-070..076)

### ADR-070 — Ponte WASM da timeline: lógica de UX do core em `capia-timeline-wasm`
**Estado:** Aceita (Fase 3) · aplica ADR-016/ADR-004.
**Contexto:** o arrasto precisa de snap, movimento de grupo e colocação a < 16 ms **sem IPC**, e a regra de snap/colocação não pode existir duas vezes (JS e Rust divergiriam).
**Decisão:** `crates/capia-timeline-wasm` compila `capia-time/model/commands` para `wasm32-unknown-unknown` e expõe uma ABI C mínima (`capia_alloc/free/call`; JSON entra e sai; réplica `thread_local` da sequence ativa mantida pelos mesmos *patches* que o engine devolve). Só `snap_clip`, `snap_point`, `group_move`, `placement`, `frame_align` — funções **puras** sobre a réplica; nenhuma escrita. É a **única** crate com `unsafe` além de `capia-webview-surface` (borda FFI); só existe `unsafe` em `wasm32` (testes nativos usam `call_bytes`). O front nunca recalcula snap em JS.
**Alternativas:** reimplementar snap em TS (rejeitada: duas verdades); `wasm-bindgen` (rejeitada: dependência extra para 5 funções); IPC por quadro de arrasto (rejeitada: meta < 16 ms).
**Consequências:** `pnpm build:wasm`/CI geram o `.wasm` (gitignored); testes do front carregam o módulo real; paridade nativo × WASM da ADR-016 continua valendo.

### ADR-071 — Extensões do modelo e dos comandos para o editor
**Estado:** Aceita (Fase 3) · **Não** altera nenhuma decisão `D-S7-*`.
**Decisão:** (1) `Sequence.header` ganha `width/height` (padrão 1920×1080); (2) clips de **texto** (`ClipContent::Text` + `TextStyle`, tamanho em ‰ da altura do quadro) e **legendas** = clips de texto em trilhas de função `Captions`; (3) **transições** na entrada do clip (`transition_in`: dissolve/fade/slide_in; dissolve exige *handles* de fonte metade de cada lado); (4) **grupos** = rótulo `group` no clip; (5) **pastas** e **deliverables** no `Document` (com ops e comandos); (6) propriedades `fade_in/fade_out` (segundos); (7) comandos `reorder_clip` (reordenação magnética), `set_text`, `set_transition`, `detach_audio`, `group/ungroup`, pastas/deliverables; (8) `RenderSettings.design_size`: deslocamentos `position_x/y` são pixels **da sequence** e escalam para a saída — preview 540p/720p compõe como o export.
**Compatibilidade de digest:** campos novos são serializados só quando diferem do padrão (resolução 1920×1080, `TextStyle` padrão, listas vazias): digests dourados e a cadeia do journal **não mudaram**.
**Consequências:** schema do documento segue compatível; testes `capia-commands/tests/editor.rs`, `capia-render/tests/editor_render.rs`.

### ADR-072 — `capia-editor-api`: fachada JSON única; devserver e Tauri são adaptadores finos
**Estado:** Aceita (Fase 3) · aplica ADR-002/ADR-029.
**Decisão:** `capia-editor-api::Session::call(método, JSON)` é **o** contrato UI↔engine (projeto, `sequence.get`, `command.execute/undo/redo`, histórico, assets, miniatura/peaks, `render.frame`, export em lote, `events.poll`). Respostas de escrita trazem *patches* (ops primitivas) que a UI aplica à réplica imutável (`applyChange`; `null` = remoção). Adaptadores: `apps/desktop` (Tauri: **dois** comandos IPC fixos, `editor_call`/`editor_call_binary`; método desconhecido ⇒ `UNKNOWN_METHOD`) e `capia-devserver` (HTTP em 127.0.0.1 **só** dev/E2E; HTTP/1.1 mínimo sobre `std::net` com `TCP_NODELAY` — o servidor anterior somava ~40 ms por Nagle+ACK atrasado). Todo comando/undo/redo publica `revision_changed`; clientes defasados (CLI, IA futura, outra janela) ressincronizam; quem emitiu ignora (revisão já aplicada).
**Regra inegociável verificada por teste:** nenhum código da UI além de `store/controller.ts` chama métodos que escrevem (`architecture.test.ts`); componentes só leem quadros/codificadores.

### ADR-073 — Render do preview fora do lock da sessão; grafo em cache por revisão
**Estado:** Aceita (Fase 3) · aplica ADR-063/066.
**Contexto:** um quadro (compositor + decode) segurava o lock da sessão e atrasava comandos/undo; o grafo era recompilado (O(clips)) a cada quadro.
**Decisão:** `Session::begin` em duas fases — preparação sob o lock (`Project::prepare_frame` ⇒ `FrameJob` independente do projeto) e render **fora** dele (`Outcome::Later(Job)`). `Project::render_graph_cached` reaproveita o grafo enquanto `document.revision` não muda (muda em todo commit/undo/redo). O render continua **único** (`render_frame`) e a fonte é sempre o original.
**Medido (5.000 clips, Linux, release):** preview 36 → 23 ms; undo/redo do engine ≈ 8–16 ms mesmo com render concorrente.

### ADR-074 — Preview P2 no app: SharedBuffer em crate isolada, com queda explícita para IPC
**Estado:** Aceita (Fase 3) · implementa o ADR-069 no produto.
**Decisão:** `crates/capia-webview-surface` (Windows) cria o SharedBuffer do WebView2 (`CreateSharedBuffer` + `PostSharedBufferToScript`, mesma sequência medida no S1) e escreve `cabeçalho(64 B: "CAPF", seq, w, h) + RGBA8`. O comando `preview_render_shared` renderiza **dentro** dele (nenhum byte de quadro no IPC); o JS confirma `sharedbufferreceived`, lê uma visão sem cópia e sobe a textura WebGL (`texSubImage2D`). Um pedido em voo por vez (agendador *latest-wins*) ⇒ sem leitura rasgada. Sem WebView2 17/Windows/timeout ⇒ **mesmo** apresentador por IPC binário; o canvas expõe `data-transport` e o CI Windows **exige** `shared-buffer`. `unsafe` fica confinado em `win.rs`/`FrameRegion` (testado em Linux com memória comum); P1/airspace não existe. Qualidade 540p/720p/Auto (histerese por latência), proxy = modo de desempenho de resolução menor (o proxy **nunca** é fonte de decode — ADR-063), métrica de *dropped slots* (pedido substituído antes de renderizar), safe areas, caixa de transformação e fullscreen são overlays HTML que não alteram a saída.
**Residual (humano/hardware):** CPU/pacing do P2 em GPU real — `tools/phase3-acceptance/gpu-residual.ps1` (gatilho de reabertura inalterado).

### ADR-075 — `OUTPUT-H264`: integração de engenharia no fluxo real do editor
**Estado:** Aceita (Fase 3) · aplica ADR-032/067/068. **Não resolve a decisão jurídica/de produto.**
**Decisão:** o diálogo de exportação lista **apenas** as `EncoderCapability` aprovadas (nunca x264/x265; sem fallback silencioso), mostra o motivo quando não há H.264 aprovado e exporta pelo mesmo pipeline atômico (staging → ffprobe → rename). O relatório exibido é o do ffprobe pós-export (codec real, resolução, quadros). O CI Windows exporta MP4 H.264 real (`h264_mf`) pelo **app real** (WebView2 + IPC) e o E2E exige `h264` no relatório. Deliverables em lote (sequence + preset + destino + tamanho) vivem no documento; progresso e resultado por item; cancelamento real.
**Continua pendente (produto/jurídico):** patentes H.264/AAC, qualidade/bitrate de produção do `h264_mf` em software, binário OpenH264. O texto do diálogo diz isso ao usuário.

### ADR-076 — Estratégia de testes do editor: E2E real em vez de pirâmide de unitários de UI
**Estado:** Aceita (Fase 3).
**Decisão:** `packages/e2e` (Playwright) dirige a UI compilada contra o **engine real + FFmpeg real** (devserver no Linux; o **app Tauri real** no Windows por CDP/WebView2). Os 17 fluxos críticos, edição (seleção/marquee/grupo/copiar/ripple/faixas/keyframes/legendas/idioma/atalhos), crash/recuperação, smoke visual (sondas de pixel + capturas como artefato; sem goldens frágeis) e benchmark de UX da timeline (5.000 clips, JSON com passa/falha por meta; *smoke* no CI, estrito localmente). Unitários só onde há lógica (kit, edição pura, agendador, teclas, preferências, fronteira UI→engine).

### ADR-077 — Monitoração de áudio do preview: mix no engine, relógio de áudio mestre
**Estado:** Aceita (Fase 3).
**Decisão:** `render.audio {sequence, from, duration}` devolve PCM f32le estéreo 48 kHz do **mesmo mixer** do export (mute/solo/volume/fades já aplicados), preparado sob o lock e mixado fora dele (ADR-073). O front agenda blocos contíguos de 0,5 s com ~1,5 s de folga no `AudioContext`; a 1× o playhead segue `currentTime` do contexto (imagem espera o 1º bloco, ~dezenas de ms); em outras velocidades, seek ou edição durante a reprodução o áudio reinicia/silencia. Falha do engine desliga só o áudio. Preferência `preview.audio` (persistida).
**Fora do escopo:** *scrub* sonoro e A/V fino medido em hardware real (pendente de aceitação humana).

---

## Fase 4 — Inteligência (ADR-078 … ADR-086)

### ADR-078 — Fronteira de segredos: `capia-secrets`, credencial write-only, redator central
**Estado:** Aceita (Fase 4) · aplica CLAUDE.md #6 e SECURITY.md.
**Decisão:** o crate-folha `capia-secrets` define `SecretString` (zeroize; `Debug`/`Display` redigidos; sem `Clone`/`Serialize`), `SecretStore` (Credential Manager no Windows; `MemoryStore` explícito nos demais — nunca silencioso), `CredentialRef` (ponteiro `capia/provider/<id>`) e o **redator** do processo (valores registrados em claro/base64/URL-encoded + padrões de `Authorization`, `x-api-key`, `?key=`, prefixos `sk-…`/`AIza…`). Toda mensagem de erro/auditoria/diagnóstico passa por ele; o gancho de pânico (`install_redacting_panic_hook`) cobre *crash*. A UI envia a chave **uma vez** (`ai.provider.save`) e só recebe `credential_configured`; a referência e o **host** da credencial são decididos no servidor (a UI não os define); trocar o host da URL exige recadastrar a chave. A credencial só é lida do cofre no instante de montar o header.
**Alternativas:** guardar a chave no AppDb cifrado (rejeitada: segredo fora do cofre do SO); devolver a chave mascarada à UI (rejeitada: superfície inútil).
**Verificação:** canário único exercitado por salvar/probe/chat/tool/erro/timeout/fallback e buscado em eventos, status, diagnóstico, histórico, snapshot, uso, **todos os arquivos em disco** (projeto, WAL, cache, AppDb) — 0 ocorrências (`capia-intelligence/tests/service.rs`, `capia-ai/tests/security.rs`).

### ADR-079 — Contrato canônico de provider; adapters nativos sem vazamento de tipos
**Estado:** Aceita (Fase 4).
**Decisão:** `capia-ai` define mensagens/partes/tools/uso/eventos canônicos e o trait `ModelProvider` (`chat` em stream, `transcribe`, `list_models`, `probe`). Adapters: OpenAI-compatível (OpenAI, OpenRouter, Groq, vLLM, Azure-like por base URL), Anthropic (Messages; turnos fundidos; saída estruturada por tool sintética forçada), Google (SSE com chave em header; schema → subconjunto Gemini), **Replay** (por digest, por roteiro ou por respondedor; uso sintético explícito; erros sintéticos) e whisper.cpp local. A mesma suíte de contrato roda contra os três adapters reais por servidores HTTP falsos que falam o protocolo de cada fabricante. Custo em **micro-unidades inteiras**; preço desconhecido ≠ zero. HTTP endurecido: URL/IP/DNS filtrados (SSRF), credencial presa ao host, **nenhum** redirect leva credencial a outro host, TLS normal (sem `danger_*`, verificado por teste de política), limites de corpo/evento/tempo (conexão, primeiro byte, total, ocioso).
**Critério do ROADMAP:** trocar o Brain entre ≥ 3 famílias **só por configuração** — `service.rs::the_brain_swaps_between_three_provider_families_by_configuration_only`.

### ADR-080 — Registry, Brain Profile e Capability Router: fallback só pelo que o usuário configurou
**Estado:** Aceita (Fase 4).
**Decisão:** capabilities têm **origem** (`declared | probed | preset`); o probe testa texto, streaming, tool call, saída estruturada, visão e STT de verdade. O Router escolhe por papel → override de capability → Brain → fallbacks **configurados**; descarta rotas que violam privacidade (`never_upload_video`, `vision_frames_only`, `transcription_local_only`, `no_cloud_audio`, `no_external_document_upload`), contexto e orçamento, e **explica** a decisão. Endpoints "auto" (qualquer outro modelo habilitado) só entram quando **nenhum** endpoint nomeado serve a capability (ex.: o Brain não tem STT): o prompt do usuário nunca vai a um provider que ele não escolheu para aquela função — achado do teste de canário (uma chave inválida caía silenciosamente noutro provider). Visão nunca cai para modelo só-texto. Saúde: `unknown/healthy/degraded/unavailable`; uma falha transitória não remove o modelo.

### ADR-081 — Tool System e gate de autorização
**Estado:** Aceita (Fase 4) · aplica ADR-029/030.
**Decisão:** registry versionado de tools com schema de entrada/saída, `Permission` e `SideEffect`. Ordem do gate (SECURITY §15): tool conhecida → permitida na tarefa → permitida ao papel → permissão concedida → argumentos válidos (JSON Schema) → orçamento/privacidade → efeito colateral aprovado. **Não existem** (e o registry recusa registrar) tools de shell, filesystem, HTTP genérico, configurações ou segredos. Saída de tool é **limitada** (itens, strings, bytes). `operation_id` das escritas é derivado de `task + passo + índice` (nunca escolhido pelo modelo): retry não duplica edição; reuso é detectado pelo engine. Comandos que o assistente pode propor são uma **lista fechada** (clips/tracks/markers/texto/propriedades); registrar/apagar asset, apagar sequence/pasta, entregáveis e variantes ficam fora. `Actor::Agent` só escreve por `preview → apply_plan` (token HMAC preso ao ator); `Session::agent_preview/agent_apply` são entradas **só Rust** (não existem em `call`/`begin`, logo a UI/JS não as alcança).

### ADR-082 — Persistência de IA: schema 4 (`ai_records`, `ai_usage`) e AppDb global
**Estado:** Aceita (Fase 4) · migration 004 (só para frente).
**Decisão:** resultados derivados (transcrições, gramáticas de referência, DemandSpec, tarefas do assistente) vivem no `.capia` em `ai_records(kind,id,version,parent,json)` e o uso/custo por chamada em `ai_usage` — **fora do documento e do undo**; chave por conteúdo+parâmetros+modelo (cache determinístico). Configuração global (registry, perfis) vive num `AppDb` separado do projeto (kv, migrations próprias, `application_id` próprio, schema futuro rejeitado sem tocar o arquivo). **Nenhum segredo** em nenhum dos dois (redação na escrita). Tarefas em andamento viram `interrupted` ao reabrir, nunca `completed`.

### ADR-083 — Detecção de cenas local, determinística e medida
**Estado:** Aceita (Fase 4).
**Decisão:** quadros RGB24 64×36 a taxa constante (filtro `fps` do FFmpeg), histograma 3×16 + diferença de pixel; corte = salto de pixel **e** de histograma acima da atividade local (mediana); salto só de histograma conta se isolado (conteúdo escuro); rejeição de *flash* (o quadro anterior reaparece); gradual por *twin comparison* com histerese relativa (dissolve/fade; fade por preto atravessa o preto); fade-in/out nas bordas do material não é fronteira. Streaming com memória O(n·200 B). **Métrica do critério:** `(FP+FN)/N_anotado ≤ 5 %` com janelas de tolerância. **Corpus anotado por construção** (lavfi/xfade/fade/concat, só geradores **determinísticos** — `gradients`/`sierpinski` sem semente explícita variam entre execuções e foram descartados) + conjunto *held-out* de fontes não usadas no ajuste. Resultado: 26/26 (erro 0 %); held-out 4/4. **Limite honesto:** corpus sintético não substitui vídeo real anotado por humanos (`tools/phase4-acceptance/reference-analyzer --corpus`).

### ADR-084 — Reference Analyzer: `ReferenceGrammar` determinística e local-first
**Estado:** Aceita (Fase 4).
**Decisão:** a gramática (planos, ritmo de cortes, mistura de transições, perfil de áudio, fala, hook/corpo/CTA) é **dado estrutural**, nunca o conteúdo protegido; mesmo arquivo + parâmetros ⇒ mesmo JSON (digest de conteúdo exclui só o carimbo). Cenas e áudio rodam sem rede; a transcrição é opcional e, sem provider, sai a nota `no_stt_available`. Persistida como registro derivado com proveniência (`fully_local`).

### ADR-085 — Demand Interpreter: `DemandSpec` com fontes **verificadas**, sem tools
**Estado:** Aceita (Fase 4).
**Decisão:** documentos (DOCX/PDF/TXT/MD, ou a fala de um vídeo) viram **unidades endereçáveis** por extração segura (ZIP com teto de entradas/bytes descomprimidos; PDF em thread isolado com `catch_unwind` e prazo; tipo por extensão **e** assinatura). O LLM propõe campos com `(documento, unidade, citação literal)`; o **código** confere cada citação contra o texto — fonte que não confere é descartada e o campo vira `unverified`; o que não consta vira `open_questions` (nunca inventado). O prompt delimita o conteúdo como `untrusted_data` e o Interpreter **não recebe nenhuma tool**: texto malicioso não tem como acionar nada (testado com injeção em PDF/DOCX/TXT/transcrição). Edições do usuário criam nova versão.

### ADR-086 — Serviço `ai.*` hospedado nos composition roots; assistente pontual
**Estado:** Aceita (Fase 4) · não inicia AI Run autônomo (Fase 5).
**Decisão:** `IntelligenceService` (`capia-intelligence`) é a única superfície `ai.*`; `capia-devserver` e `capia-desktop` a roteiam e mesclam seus eventos no `events.poll` do editor (a UI mantém um laço de poll). Sem o serviço — ou com a IA desligada — o editor funciona igual (E2E prova **zero** chamadas `ai.*` e zero rede externa numa sessão de edição). O assistente de chat é um laço curto (≤ 8 passos, ≤ 4 tools/passo), cancelável de verdade (aborta a conexão HTTP; nenhuma tool tardia aplica), com modos **Ask** (pausa em `awaiting_approval`; só o host aprova por `task_id + plan_token`, persistido **antes** de avisar a UI) e **Auto** (aplica, mas ainda por `preview → apply_plan`); um token só vale para plano pré-visualizado **na mesma tarefa**. Não há Producer/Planner/Critic, variantes autônomas, memória autônoma nem Asset Gateway completo (Fase 5). Roteiros Replay para E2E entram só no devserver (`CAPIA_AI_REPLAY_SCRIPT`), nunca no produto.

---

## Fase 5 — Autonomia (ADR-087 … ADR-119)

> Estado geral: engenharia em integração (ver `docs/STATUS.md`). Especificações-fonte em `docs/phase5/`. Nenhuma ADR abaixo declara aceitação humana, uso de provider real ou CI verde.

### ADR-087 — Schema 5: persistência da autonomia (`AutonomyStore`)
**Estado:** Aceita (Fase 5) · migration `m005_autonomy` (só para frente, aditiva; projetos v1..v4 ganham tabelas vazias).
**Decisão:** nove tabelas novas no `.capia`, fora do documento e do undo: `ai_runs` (cursor: `status`, `stage`, `revision`, `parent_run_id`, `variant_group`, JSON da Run), `ai_run_stages` (uma linha por execução de stage; `UNIQUE(idem_key)`), `ai_run_events` (eventos duráveis por `(run_id, seq)`), `ai_side_effects` (livro de efeitos), `ai_provenance`, `ai_memory` (**só** escopo `project`), `ai_memory_log`, `ai_budget_ledger` (`reserve|settle|release`). `AutonomyStore` (`capia-store/src/autonomy.rs`) usa conexão própria (WAL coexiste com o journal) e redige segredos registrados no `capia-secrets` antes de gravar. Memória de User/Client vive no `AppDb` global (ADR-082), nunca no projeto. Schema mais novo continua rejeitado sem tocar o arquivo (ADR-043).
**Alternativas:** blob JSON único por Run (sem CAS nem auditoria por stage); guardar a Run dentro do documento (viola "Brief ≠ Research ≠ Edit Plan ≠ Timeline" e polui o undo); arquivo SQLite separado (perde atomicidade com import de assets e backup único).
**Consequências:** reabrir um projeto v4 migra sem perda; a Run sobrevive a fechar/crash; o documento não carrega estado de IA.
**Evidência:** `capia-store/tests/autonomy_store.rs` (`a_schema_4_project_migrates_and_keeps_its_content`, `provenance_memory_and_events_round_trip_and_redact_registered_secrets`).

### ADR-088 — Máquina de estados da AI Run, cursor atômico e recuperação
**Estado:** Aceita (Fase 5).
**Decisão:** estágios `UNDERSTAND → PLAN → VALIDATE_PLAN → ACQUIRE → EDIT → REVIEW → CORRECT → DONE` e estados de espera `pending/running/waiting_user/paused/failed/cancelled/completed` são **enums fortes**. A tabela `machine::transition(stage, outcome)` é a **única** fonte de transições (fechada: o que não consta é `IllegalTransition`; `Cancel`/`Failure` valem de qualquer estágio). Só `EDIT` e `CORRECT` podem escrever (`RunStage::may_write`). Cada transição é **uma** transação SQLite com compare-and-swap por `revision` (`AutonomyStore::advance`: registro do stage + eventos + novo cursor juntos). `Orchestrator::recover()` marca stages `started` como `interrupted` (nunca `completed`), converte `Running → Paused` (com `resume_stage`) e **nunca retoma sozinho**: a retomada é ação explícita (`ai.run.resume`). O orchestrator não edita a timeline, não escolhe provider fora do router e não lê segredos. Taxonomia de erros (`RunErrorKind`) decide retry × espera × falha.
**Alternativas:** estado em memória + log (perde a Run no crash); transições espalhadas nos handlers; auto-resume ao abrir o projeto (gasto e escrita sem o usuário saber).
**Consequências:** replay/retomada determinísticos; testes de propriedade percorrem a tabela.
**Evidência:** `autonomy_properties.rs::random_walks_over_the_state_machine_never_break_the_write_gates`; `autonomy_store.rs::the_cursor_moves_only_with_the_expected_revision_and_atomically`, `started_stages_become_interrupted_never_completed`; `autonomy_scenarios.rs::pause_stops_between_steps_and_resume_finishes_the_run`; unitários em `autonomy::machine`.

### ADR-089 — Livros de efeitos colaterais e de orçamento (idempotência)
**Estado:** Aceita (Fase 5) · estende ADR-029.
**Decisão:** todo efeito externo ou caro passa por `claim_effect(effect_key, …)` (`INSERT OR IGNORE`: **a primeira tentativa vence**; resume/retry/callback duplicado enxergam o mesmo registro). Chaves determinísticas: `llm:<run>:…` (chamadas de papel; resultado cacheado no livro), `gw:<run>:<need>:<cand>` (download), `imp:<…>` (import de asset), `gen:<run>:<need>:v<versão>:…` (geração). Estados do efeito: `intent → submitted → done | failed`, com `external_id` (ticket/job). Orçamento: `ai_budget_ledger` com `ledger_reserve` (transação `IMMEDIATE`; recusa se comprometido + pedido > teto; reserva repetida é idempotente), `ledger_settle` (custo real; libera o excedente) e `ledger_release`. O custo é gravado **no momento da chamada**, não no fim do stage. Preço desconhecido nunca é tratado como zero. `operation_id`s de comandos da IA são derivados (`<run>:<ns>:<chave>:…`), nunca do modelo.
**Alternativas:** deduplicar só em memória; somar custo ao fim do stage (crash perde o gasto); contador simples sem reserva (corrida entre workers estoura o teto).
**Consequências:** nenhum retry paga duas vezes nem aplica duas vezes; o teto vale sob concorrência.
**Evidência:** `autonomy_store.rs::only_the_first_claim_of_an_effect_executes`, `parallel_reservations_never_exceed_the_limit`, `settle_and_release_reconcile_reserved_with_actual`; `autonomy_budget.rs` (limite igual à estimativa roda; um micro a menos pergunta; preço desconhecido ≠ zero; teto de geração antes do submit); `autonomy_properties.rs::random_ledger_operations_never_commit_above_the_limit`.

### ADR-090 — O Editor é um compilador determinístico; nada escreve antes de plano validado
**Estado:** Aceita (Fase 5) · aplica ADR-029/030/081.
**Decisão:** não existe "agente Editor" com LLM. `EditPlan → comandos` é compilação determinística (`autonomy/plan.rs`; tempo do plano em milissegundos inteiros → `Ticks` alinhados a quadro; sem float de segundos no modelo). Escrita só em `EDIT`/`CORRECT`, só por `preview → apply_plan` com `Actor::Agent("run:<id>")` (token preso ao ator; `Session::agent_preview/agent_apply` continuam só Rust), e só **depois** de `VALIDATE_PLAN` produzir um `ValidationReport` válido (dry-run por transação) e das aprovações exigidas. Aprovações (`PlanApproval`/`SpendApproval`) ficam presas ao **digest** do plano (produção + EditPlans + versão do spec): plano alterado ⇒ aprovação inválida. Se a revisão do documento mudou (edição manual) entre validar e aplicar, o resultado é `Drift` → revalidar; edição manual nunca é sobrescrita. Operações destrutivas acima do limiar da política exigem aprovação.
**Alternativas:** LLM emitindo comandos diretamente; aplicar e reverter se inválido (viola o princípio 3 e polui o histórico).
**Consequências:** o histórico mostra só aplicações de planos validados; auditoria de Runs é possível por ator.
**Evidência:** `autonomy_scenarios.rs::plan_approval_gates_every_write_and_reject_change_and_approve_all_work`, `an_invalid_plan_is_replanned_and_the_replan_loop_is_bounded`, `a_manual_edit_during_the_run_drifts_the_diff_and_is_never_overwritten`; `autonomy_security.rs::preview_apply_integrity_wrong_actor_or_altered_plan_never_writes`; `autonomy_crash.rs::crash_before_the_apply_writes_nothing_and_after_it_never_applies_twice`; unitários de `autonomy::plan`.

### ADR-091 — Papéis Producer/Planner/Critic sem tools e REVIEW → CORRECT limitado
**Estado:** Aceita (Fase 5) · estende ADR-085.
**Decisão:** Producer (`ProductionPlan`), Planner (`EditPlan`) e o lado semântico do Critic são papéis de LLM com prompt **versionado**, saída com JSON Schema, **zero tools**, contexto limitado e entradas externas (DemandSpec, transcrição, nomes de arquivo, metadados de fontes) em blocos `untrusted_data`. Nenhum papel escreve. O Critic tem checagens **determinísticas** (duração, placeholders, assets offline, CTA, conteúdo proibido, safe area, buracos, beats ausentes, formato; `RUBRIC_VERSION`) e achados semânticos com **evidência**. A correção é um `CorrectionPlan` compilado de um **vocabulário fechado** (`FixAction`: `TrimToDuration`, `AddCtaText`, `DeleteClip`, `SetProperty`, `SetClipEnabled`) — sem comando livre. O laço REVIEW→CORRECT é **limitado** (ciclos e orçamento); detecta oscilação/correção que não ajuda e pergunta ao usuário (`ReviewExhausted → WAITING_USER`).
**Alternativas:** Critic com tools de edição; correção por comando livre do modelo; laço sem teto.
**Consequências:** injeção em brief/transcrição/metadados não tem como acionar tool ou comando.
**Evidência:** `autonomy_scenarios.rs::the_correction_loop_fixes_in_one_cycle_with_minimal_commands`, `a_correction_that_does_not_help_stops_the_loop_and_asks_the_user`; `autonomy_security.rs::hostile_brief_stays_inside_untrusted_blocks_and_cannot_break_out`, `a_hostile_model_cannot_spend_fetch_or_promote_memory_without_a_human`; unitários de `autonomy::critic`/`roles`.

### ADR-092 — Memória em 4 escopos; promoção só por aprovação humana
**Estado:** Aceita (Fase 5) · substitui o "memória ainda não implementada" de `AI_SYSTEM.md §9`.
**Decisão:** escopos `System` (embutido, versionado, somente leitura) · `User` · `Client` · `Project`; precedência `Project > Client > User > System`, conflito resolvido **de forma visível**. A IA só **propõe** (`Proposed`); o único caminho para `Active` em User/Client é `MemoryManager::approve`, que exige um `UserApproval` (construído só pelo serviço, por ação explícita da UI). Memória Client exige `client_id` explícito e é isolada por cliente; User exige o `AppDb`. Item `rejected`/`archived` não ressuscita e nunca é recuperado. Correções repetidas só **propõem** após `SIGNAL_THRESHOLD` (3) e nunca ativam. Memória de Project proposta pela IA só ativa se `project_memory_auto_activate` (padrão `false`). Conteúdo é sanitizado e limitado; excluir remove o conteúdo e o log de auditoria guarda só um digest. Todo evento vai para `ai_memory_log`.
**Alternativas:** memória autônoma com ativação silenciosa (envenenamento por brief hostil); memória única sem escopos.
**Consequências:** brief importado ou saída de modelo não alteram preferências sem o usuário.
**Evidência:** `autonomy_memory.rs` (todos os testes), `autonomy_properties.rs::random_memory_operations_never_activate_user_or_client_without_a_human`, `AiRunsPanel.test.tsx` ("propostas de memória … inativas até o clique explícito"), `autonomy_service.rs::memory_and_gateway_endpoints_are_safe_by_default`.

### ADR-093 — Asset Gateway, `SafeFetcher` como única rede além dos providers, e pasta de mídia durável
**Estado:** Aceita (Fase 5) · estende ADR-079 (HTTP endurecido) e ADR-046..049 (assets).
**Decisão:** o Brain **não** tem `http.get`; pede *needs* tipados e o orquestrador chama adapters (`AssetGatewayAdapter`): `LocalLibrary`, `ApprovedUrl` (allow-list de hosts) e `ReplayCatalog` (testes/dev). `SafeFetcher` (`capia-ai/src/fetch.rs`) é o **único** código de rede do produto fora dos providers de IA: `https` (loopback só com opt-in de teste), sem credencial na URL, host dentro da allow-list **a cada redirect**, DNS filtrado (sem privado/link-local/metadata), tipo de conteúdo permitido, teto de bytes **durante** o stream, hash SHA-256 em streaming, escrita em `*.part` + `rename` atômico, cancelamento real. Metadados de fontes externas são **dados não confiáveis**. Veredito de licença (`license_verdict`): `KnownAllowed/UserProvided/Generated` → permitir; `Unknown` → **aprovação** (padrão seguro); `KnownRestricted` → rejeitar (padrão) ou pedir decisão. Cada download tem proveniência em `ai_provenance` (adapter, fonte, licença, hash, run). O arquivo adquirido é movido para `<projeto>-media/ai/<sha>.bin` — pasta **durável**, porque o cache (`<projeto>.capia-cache/`) é descartável e o catálogo guarda o caminho — e importado pelo sistema de assets (hash + probe + documento + catálogo na mesma transação) com atribuição à Run. Adapter desligado ⇒ o app continua; fallback por replanejamento ou `WAITING_USER`.
**Alternativas:** `http.get` como tool; manter o download no cache (o original some ao limpar); importar sem proveniência.
**Consequências:** a superfície de rede é auditável (`pnpm check:arch` + suíte de segurança); mídia da IA sobrevive à limpeza do cache.
**Evidência:** `capia-ai/tests/fetch.rs` (`a_redirect_to_a_host_outside_the_allow_list_is_not_followed`, `private_and_non_allowed_urls_never_reach_the_network`, `cancelling_aborts_the_download_and_leaves_nothing`, …); `autonomy_scenarios.rs::a_missing_broll_is_acquired_through_the_gateway_with_approval_and_provenance`, `known_license_free_assets_need_no_approval_and_restricted_ones_are_skipped`, `a_disabled_gateway_never_breaks_the_app_and_optional_needs_fall_back_by_replanning`; `autonomy_crash_acquire.rs::crash_after_the_download_never_downloads_twice_and_imports_one_asset`.

### ADR-094 — Geração de mídia: opt-in, aprovação + orçamento, jobs idempotentes
**Estado:** Aceita (Fase 5).
**Decisão:** geradores (imagem/vídeo/TTS) são providers de **job** separados do Brain, registrados num `GenerationRegistry` **desligado por padrão** (`ai.generation.set_enabled`, persistido na configuração do gateway). Mesmo ligada, a política exige aprovação (`generation_requires_approval`) **e** orçamento reservado antes do submit. `idempotency_key` determinística por (run, need, prompt, modelo, versão do plano); o `job_id` do provider é persistido no livro de efeitos e, após crash, o orquestrador **consulta** o job antes de qualquer novo submit. A mídia gerada passa por staging → hash → proveniência (prompt, modelo, parâmetros, custo) → import; versões novas não sobrescrevem o arquivo anterior.
**Alternativas:** geração sempre disponível; submit sem consulta prévia (paga duas vezes após crash).
**Consequências:** um app sem provider de geração funciona igual; o custo é aprovado antes.
**Evidência:** `autonomy_scenarios.rs::generation_needs_approval_and_budget_and_records_full_provenance`, `cancelling_during_generation_stops_everything_and_applies_nothing`; `autonomy_crash_acquire.rs::crash_after_the_generation_submit_never_submits_or_pays_twice`; `autonomy_budget.rs::the_generation_cap_is_enforced_before_submitting`; `autonomy_service.rs::memory_and_gateway_endpoints_are_safe_by_default`; E2E `autonomy.spec.ts` ("a geração por IA e as fontes começam desligadas").

### ADR-095 — Undo seletivo como nova entrada de histórico, com conflitos por entidade e por dependência
**Estado:** Aceita (Fase 5) · estende ADR-030 e `COMMAND_SYSTEM.md §6/§7`.
**Decisão:** `Engine::selective_undo_report(entries)` (somente leitura) e `selective_undo(actor, entries, mode, label)` desfazem entradas **escolhidas** (por exemplo todas de `run:<id>`) aplicando as `inverse_ops` como **nova entrada** (autor = quem pediu), ela mesma desfazível; o histórico nunca é apagado nem reescrito. Conflito = entrada posterior **não selecionada** que toca as mesmas entidades **ou** que perderia entidades por dependência (a inversa já não aplica limpa, ou removeria sequence/track/clip que a entrada posterior usa — ex.: faixa manual dentro de sequence da IA). Modos: `Safe` (tudo ou nada: `CONFLICT`) e `Partial` (pula as entradas em conflito e só prossegue se o resultado for válido). Só ator humano pode pedir (`Agent`/`Api` são recusados: `PreviewRequired`). Engine API: `history.undo_report` e `history.undo_selective` com `entries` **ou** `actor_id` (exatamente um).
**Alternativas:** undo linear apenas (desfaz edição manual junto); `git revert` semântico sem relatório; sobrescrever edição manual silenciosamente.
**Consequências:** o usuário desfaz uma Run inteira sem perder o que editou depois; o conflito é explicado antes de aplicar.
**Evidência:** `capia-commands/tests/selective_undo.rs` (`undoes_a_run_when_nothing_else_touched_it_and_keeps_history`, `a_later_manual_edit_on_the_same_entity_is_a_conflict`, `partial_undoes_the_clean_entries_only`, `agents_cannot_selectively_undo_and_unknown_entries_are_rejected`); `autonomy_variants.rs::undoing_a_run_removes_only_what_the_ai_made_and_manual_edits_win_conflicts`, `a_manual_edit_on_the_ai_output_is_reported_as_a_conflict_and_never_erased_in_safe_mode`.

### ADR-096 — Import de asset pelo Gateway só por caminho Rust, sob atribuição da Run
**Estado:** Aceita (Fase 5) · aplica ADR-046/087.
**Decisão:** `EditorApi::agent_import_begin/poll/cancel` são métodos **só Rust** (não existem em `call`/`begin`): reaproveitam `import_asset_async` (hash + probe + registro no documento e no catálogo na MESMA transação). O Gateway só chama depois de validar tamanho/tipo/licença; o resultado é atribuído à Run (`ai_provenance.run_id`, efeito `imp:*`), e `Cancel` aborta o ticket em staging. Falha nunca deixa asset parcial válido.
**Alternativas:** expor import ao modelo como tool; escrever direto no catálogo.
**Consequências:** o catálogo continua com uma única porta de escrita; assets da IA são rastreáveis e removíveis (`ai.run.cleanup` lista candidatos).
**Evidência:** `autonomy_crash_acquire.rs`, `autonomy_scenarios.rs::a_missing_broll_is_acquired_through_the_gateway_with_approval_and_provenance`.

### ADR-097 — Variantes: Runs filhas em grupo, estratégias de sequence compartilhadas
**Estado:** Aceita (Fase 5).
**Decisão:** `ai.run.variants` cria uma Run **filha** (`parent_run_id`, `variant_group`) a partir de uma Run **concluída** (1..20 variantes por pedido; teto de Runs por grupo; herda política/orçamento/Brain/DemandSpec e a sequence master). O Planner escolhe a estratégia por deliverable: `standalone`, `hook_plus_master` (ganchos distintos com o corpo master **aninhado** via `insert_nested`, na mesma transação do master), `shared_master` e `format_variant` (`variant_of`). Cada variante é uma sequence distinta e **100 % editável**; `ai.run.group` lista o grupo. Nenhuma Run cria outra Run por conta própria (sem Run aninhada autônoma).
**Alternativas:** clonar a sequence e editar; variantes como estado fora da timeline.
**Consequências:** o usuário edita/descarta variantes como qualquer sequence; undo seletivo por Run isola cada variante.
**Evidência:** `autonomy_variants.rs::one_run_makes_a_master_and_variants_that_are_distinct_editable_sequences`; unitários `autonomy::plan` (`hook_plus_master_shares_one_transaction_and_nests_the_master`); `autonomy_security.rs` ("no nested/recursive run").

### ADR-098 — Fronteira da UI para Runs, Memory e Sources
**Estado:** Aceita (Fase 5) · estende a regra do Editor (ADR-070) e ADR-086.
**Decisão:** a UI (painéis Runs, Memory e Sources dentro de `AiRunsPanel`) fala com `ai.run.*`/`ai.memory.*`/`ai.gateway.*`/`ai.generation.*` **apenas** por `store/aiController.ts`; o undo de Run é `controller.undoRun`/`undoRunReport` (comando `history.undo_selective`, nova entrada, modos `safe`/`partial`) — único lugar que chama `execute/undo/redo` continua sendo `store/controller.ts` (`architecture.test.ts`). A UI mostra custo estimado antes de aprovar, liga a aprovação ao id da decisão pendente, e exibe proposta de memória inativa até o clique explícito. Geração e fontes começam desligadas. Os bindings (`packages/engine-bindings/src/ai.ts`) não carregam segredo nem escrita direta.
**Alternativas:** UI chamando o cliente de engine direto; aprovação implícita.
**Consequências:** o editor 100 % manual não faz nenhuma chamada `ai.*` (E2E "AI Off" da Fase 4 mantido).
**Evidência:** `AiRunsPanel.test.tsx`, `aiController.test.ts`, `architecture.test.ts`, `packages/e2e/tests/autonomy.spec.ts`.

### ADR-099 — Testes de crash: failpoints em processo e SIGKILL real; pacote de aceitação da Fase 5
**Estado:** Aceita (Fase 5) · estende ADR-036/076.
**Decisão:** a feature `failpoints` (**só testes**, fora do build normal) expõe pontos nomeados nas fronteiras de stage e de efeito: `arm(nome)` dispara uma vez em processo (simula a morte do driver sem derrubar o teste) e `CAPIA_FAILPOINT`/`CAPIA_FAILPOINT_MODE` (`abort` padrão; `park` imprime `FAILPOINT_REACHED <ponto>` e dorme) permitem SIGKILL real de um processo-filho e retomada em **outro** processo. Invariantes: sem edição/download/geração duplicados, timeline final igual à execução limpa. Propriedades: caminhadas aleatórias na máquina, ledger ≤ teto, memória sem ativação sem humano (`CAPIA_PROP_CASES`). Aceitação: `tools/phase5-acceptance/` (`autonomy`, `crash-resume`, `security`, `gateway`, `memory`, `real-demands` + `run-all.mjs`, resumo em `target/phase5-acceptance/`). Itens que dependem de humano ou de provider real ficam como `pending_external` e **nunca** são preenchidos com resultado inventado; `real-demands/validate.mjs` só aceita ≥ 10 demandas distintas com nota inteira 1–5, `run_id` e `brief_file`, média ≥ 4,0.
**Alternativas:** só testes em processo (não provam fsync/WAL/lock reais); mocks de crash.
**Consequências:** o resultado final do pacote só pode ser `PHASE 5 ACCEPTED` com resultados humanos reais válidos.
**Evidência:** `autonomy_crash.rs`, `autonomy_crash_acquire.rs`, `autonomy_kill.rs`, `autonomy_properties.rs`, `tools/phase5-acceptance/real-demands/validate.test.mjs`.

### ADR-100 — Critic com visão real: amostragem determinística de quadros, evidência de quadro, degradação explícita
**Estado:** Aceita (Fase 5, closeout) · estende ADR-091.
**Decisão:** o REVIEW amostra **quadros compostos da timeline** (o mesmo `render.frame` do preview, por `Engine::render_frame`, só leitura) em instantes escolhidos por `autonomy::vision::plan_samples` — primeiro quadro, ponto médio de cada clip de mídia visual e de cada texto, último quadro; alinhados a quadro, ordem estável, **sem aleatoriedade**, no máximo `critic_max_frames` (padrão 6, teto 12) e 1,5 MB de PNG por Review; reduzidos a ≤ 384 px de maior lado e codificados em PNG. Nunca vídeo inteiro. Cada quadro vai ao Critic precedido de um rótulo que só contém **ids do sistema** (nada de nomes de arquivo nem texto do projeto); o conteúdo visual e qualquer texto dentro do quadro são **dados, nunca instruções** (preâmbulo `untrusted_data` + prompt v2). O Capability Router escolhe o modelo: a presença de imagem acrescenta `VisionInput` e a classe `Frames` entra na política de privacidade. O Critic recebe quadros + transcrição do bruto + digest da timeline + `DemandSpec` + `EditPlan` + `ReferenceGrammar` (sem a lista de shots) e avalia enquadramento, continuidade, adequação de B-roll, presença do produto/elemento, encaixe visual-semântico, aderência à referência e legibilidade/composição (categorias `framing`, `continuity`, `broll_fit`, `product_presence`, `visual_fit`, `legibility`, `composition`). **Achado visual só existe com ≥ 1 quadro citado** (`EvidenceRef` de tipo `frame`: índice, instante em ticks, clips visíveis, sha do PNG); sem quadro válido ele é descartado (anti-alucinação) e o `at` do achado aponta para o(s) instante(s) citado(s). A chave de cache (`llm:<run>:critic:<ciclo>:<dk>:<digest dos quadros>`) inclui o digest dos PNGs. **Degradação explícita:** se a política desliga a visão (`critic_vision=false` → `DISABLED_BY_POLICY`), o orçamento da Run já estourou (`BUDGET_EXCEEDED`, antes de renderizar qualquer quadro), o engine não renderiza (`NO_FRAMES`), a captura falha ou o Router/privacidade recusam (`NO_CAPABLE_MODEL`, `PRIVACY_POLICY_BLOCKED`…), o Critic **não cai**: repete só com texto e o Review registra `provenance.vision {status, reason, frames, model}` e `provenance.critic_mode` (`vision+text` | `deterministic+text`). Cancelamento é checado entre quadros.
**Alternativas:** extrair quadros do arquivo-fonte por FFmpeg (não é o resultado composto: perde cortes, overlays e enquadramento); enviar vídeo inteiro (viola local-first/privacidade/custo); deixar o modelo citar texto livre sem índice (alucinação sem prova).
**Consequências:** `png` entra como dependência de `capia-intelligence` (matriz de arquitetura atualizada); o Replay simula visão **olhando os pixels** dos quadros recebidos (B-roll vermelho = adequação ruim; sujeito fora do centro = enquadramento ruim) — não prova a qualidade de um modelo de visão real (pendência externa). Bugs achados no caminho e corrigidos: o Critic nunca recebia a transcrição; `AddCtaText` usava a chave do deliverable em vez do id real da sequence.
**Evidência:** `autonomy/vision.rs` (testes unitários), `tests/autonomy_vision.rs` (amostragem/limites, B-roll errado, enquadramento errado, sem modelo de visão, política, orçamento, sem compositor, cancelamento, injeção por texto no quadro, cache por digest).

### ADR-101 — App desktop de teste hospeda o cérebro Replay só por feature de cargo + env; E2E de autonomia sem skip no Windows
**Estado:** Aceita (Fase 5, closeout) · estende ADR-076/086.
**Decisão:** o app Tauri só embute `IntelligenceService::install_demo_autonomy()` quando compilado com a feature de cargo `e2e-testkit` (`capia-intelligence/testkit`) **e** executado com `CAPIA_AI_DEMO_BRAIN`; sob a mesma feature `CAPIA_E2E_APPDB` aponta o banco global do app para a pasta do teste (estado isolado e visível ao app reaberto). O build de produto (`pnpm desktop:build`) não liga a feature; `tools/check-architecture.mjs` falha se `testkit` aparecer no desktop fora de `e2e-testkit` ou se ela virar padrão. O cérebro demo é o roteador Replay por papel (`ROLE: producer|planner|critic`, senão DemandSpec), sem rede nem chave; nada é exposto à WebView (a UI só vê `ai.*` como sempre; o provider é registrado no core Rust). `CAPIA_AI_DEMO_DELAY_MS` alonga a resposta para o E2E poder matar o app no meio de uma Run. O brief `[demo:needs-correction]` faz o demo errar de propósito (CTA sem texto) para exercitar REVIEW → CORRECT. O job E2E Windows constrói o app com `-- --features e2e-testkit` e roda `autonomy.spec.ts` **sem skip**: Run completa (UNDERSTAND → PLAN → plano exibido e aprovado → VALIDATE_PLAN → EDIT por preview→apply_plan → REVIEW → CORRECT), duas variantes, progresso/custo, undo seletivo, fechar (kill) e reabrir confirmando Runs/histórico/timeline, `WAITING_USER` que sobrevive ao restart, cancelamento sem escrita, retomada manual de Run interrompida (`paused`, nunca sozinha) e AI Off.
**Alternativas:** feature padrão com env (arriscado: o binário de produto carregaria o cérebro); pular o teste no Windows (lacuna de cobertura real); expor um provider de teste via IPC (superfície de ataque).
**Consequências:** o binário do E2E difere do de produto só por essa feature; bugs achados pelo E2E: `AppDb` reaberto depois de kill era recusado como "arquivo alheio" (cabeçalho defasado, dados só no `-wal`) — corrigido em `AppDb::open` com teste de regressão.
**Evidência:** `packages/e2e/tests/autonomy.spec.ts`, `tools/check-architecture.test.mjs`, `capia-store/tests/intelligence_store.rs::an_app_db_left_by_a_killed_process_reopens_with_its_data`, `autonomy_service.rs::the_demo_brain_exercises_review_correct_and_two_variants_with_selective_undo`.

## Fase 6 — Integração e finalização (ADR-102 … ADR-119)
> Consolidado das trilhas A–E da Fase 6 (rascunhos originais em `docs/phase6/adr-drafts/`). Estado: **Accepted (Fase 6, engenharia)**; gates externos (certificado real, máquinas limpas físicas, beta humano, decisão jurídica de H.264/AAC, pentest independente) permanecem abertos.

### ADR-102 — `capia-server` é um host headless da MESMA Engine API; catálogo único de operações
**Estado:** Accepted (Fase 6).
**Decisão.** O crate `capia-server` hospeda `capia-editor-api::Session` + `capia-intelligence` (os
mesmos objetos que a UI usa). Toda operação externa é uma entrada do **catálogo**
(`src/catalog.rs`): nome, scope, mutante?, classe de rate limit, rota REST, schema JSON. REST, MCP,
OpenAPI, matriz de scopes e documentação derivam dele; um único pipeline (`Core::call`) executa
autenticação → scope → schema → rate limit → gate de projeto/shutdown → idempotência → handler →
auditoria. Não existe rota/tool fora do catálogo (testes de arquitetura do catálogo garantem: scope
em tudo exceto `server.health`, rotas únicas, GET nunca muta, nenhum parâmetro de caminho/segredo,
nenhum nome perigoso).

**Alternativas.** (a) Rotas escritas à mão por transporte — rejeitado: divergência REST×MCP é o
risco central; (b) framework HTTP (axum/hyper) — rejeitado por ora: superfície de dependência e
controle fino de limites (HTTP/1.1 mínimo e endurecido em `std::net`, como o devserver).

**Consequências.** Paridade UI↔REST↔MCP é por construção. Mudar uma operação = mudar o catálogo.

### ADR-103 — Escrita externa só por `preview → apply_plan` com `Actor::Api` por token
**Estado:** Accepted (Fase 6).
`commands.preview`/`commands.apply` usam `Session::agent_preview/agent_apply` (só Rust) com
`Actor::Api("token:<id>")`: o plan token HMAC do engine é preso ao ator, então outro token não
aplica o plano de quem fez o preview. Concorrência otimista: `expected_revision` → 409
`REVISION_CONFLICT`; plano velho → 409 `PLAN_STATE_CHANGED`. Gravações internas da sessão do
servidor (import de mídia finalizado no `pump`) usam o ator `System("capia-server")`. **Fora do
catálogo de propósito** (privilégio ≤ UI e regra "a IA só propõe"): undo/undo seletivo, aprovação de
memória (User/Client), configuração de providers/credenciais/gateway/geração, qualquer caminho de
arquivo do cliente. `ai.run.decide` aceita `decided_by` (`api:<token>`) para a trilha de auditoria
das aprovações; a UI continua gravando `user`.

### ADR-104 — Tokens, scopes, rotação
**Estado:** Accepted (Fase 6).
Token `capia_<64 hex>` (256 bits do SO), mostrado uma única vez; o banco guarda só o SHA-256
(índice único). Revogação, expiração e rotação atômica (novo + revoga antigo na mesma transação).
Sem escalada: um token só concede (create) ou rotaciona tokens cujos scopes ele possui.
`Authorization` inválido/revogado/expirado → a mesma resposta 401 (sem oráculo). O segredo é
registrado no redator global do processo (nunca sai em log/erro). Scopes: `project:read|write`,
`media:read|write`, `run:read|start|approve`, `export:read|start`, `webhook:manage`,
`admin:tokens`; `run:approve` é separado de `run:start` (menor privilégio).

### ADR-105 — Idempotência e auditoria de toda escrita externa
**Estado:** Accepted (Fase 6).
`Idempotency-Key` (REST) / `idempotency_key` (MCP) por (token, chave): replay devolve o mesmo
resultado semântico (cabeçalho `Idempotent-Replay`); mesma chave com pedido diferente → 422
`IDEMPOTENCY_KEY_REUSED`; em andamento → 409; `pending` deixado por processo que caiu →
`IDEMPOTENCY_INDETERMINATE` (o cliente verifica o estado e usa outra chave — nunca executa duas
vezes). Respostas com segredo único (tokens/webhooks) nunca entram na tabela de idempotência: o
replay vem sem o segredo (`secret_unavailable_on_replay`). Auditoria (`audit`): toda escrita e toda
recusa/erro com token, superfície, operação, revisões antes/depois e chave; leituras bem-sucedidas
não escrevem no banco; o log é aparado (100 mil entradas).

### ADR-106 — Um projeto aberto por vez; troca exclui chamadas em voo
**Estado:** Accepted (Fase 6).
O engine hospeda um projeto por sessão. Rotas `/v1/projects/{id}/…` exigem esse projeto aberto
(409 `PROJECT_NOT_OPEN`). `projects.create/open/close` só com Runs/exports ociosos (409
`PROJECT_BUSY`) e sob um `RwLock`: uma chamada validada para P1 nunca roda contra P2 (sem TOCTOU).
Projetos moram em `<data>/projects/<id>/project.capia`; **nenhum caminho do cliente** entra na API.

### ADR-107 — Uploads: streaming para staging, sniff, mídia durável, nada de caminhos na resposta
**Estado:** Accepted (Fase 6).
`POST /v1/uploads` (Content-Length obrigatório; sem `Transfer-Encoding`): hash durante a escrita,
teto por tipo/tamanho/cota/tempo/concorrência, *sniff* por bytes (nunca nome/MIME do cliente),
`*.part` → `rename` atômico, idempotente por conteúdo (token+hash+nome). Importar move a mídia do
staging para `<projeto>/media/<upload_id>/` (durável) e entra pelo sistema de assets (hash + probe +
catálogo na mesma transação). Documentos de uma Run são resolvidos pelo servidor a partir do
`upload_id`. Respostas nunca carregam caminhos do servidor (exceto o `path` de um export, que está
sob a raiz de saída escolhida pelo servidor).

### ADR-108 — Transporte: loopback, Host, CORS, limites, backpressure
**Estado:** Accepted (Fase 6).
Padrão loopback. Bind remoto exige `--allow-remote` **e** `--remote-tls-terminated-by-proxy` (nunca
token em texto claro por padrão; sem TLS nativo — proxy reverso documentado). `Host` só do próprio
servidor (421; anti DNS-rebinding). CORS fechado: qualquer `Origin` não listado → 403 (CSRF de
navegador); `*` proibido. Limites: linha 8 KiB, cabeçalhos 32 KiB/64, JSON 1 MiB e profundidade 32,
prazo de 10 s para o cabeçalho (slowloris), pool de workers fixo + fila finita (503 `OVERLOADED`,
nunca thread por conexão), SSE com teto. Shutdown gracioso: recusa escrita (503), pausa Runs
(retomáveis), espera o que está em voo, fecha o projeto.

### ADR-109 — Segredos de webhook no cofre; sem cofre ⇒ memória
**Estado:** Accepted (Fase 6).
O segredo de assinatura de um webhook fica só no `SecretStore` (Credential Manager no Windows). Sem
cofre do SO (Linux/CI) o armazenamento é em memória (nunca arquivo em claro): após reinício as
entregas do webhook ficam `dead` com `SECRET_UNAVAILABLE` até `webhooks.rotate_secret`, que as
reenfileira.

### ADR-110 — MCP é um adaptador fino sobre `Core::call`
**Estado:** Accepted (Fase 6).
**Status:** proposto. **Contexto:** a Fase 6 precisa expor a Engine API a agentes externos (MCP) sem criar um segundo
backend nem dar mais poder que a UI. **Decisão:** o MCP (`capia-server/src/mcp.rs`) só traduz JSON-RPC 2.0
(revisão `2025-06-18`, tolerando `2025-03-26`/`2024-11-05`) para `Core::call` com `surface:"mcp"`. Tools = operações do
catálogo com `surface == Both` (nome `runs_create`); `idempotency_key` é argumento extra só nas mutantes e vira
`CallCtx.idempotency_key` (tabela de idempotência compartilhada com a REST); resources `capia://…` são leituras que
passam pelas mesmas operações. Erro de operação = `isError:true` com envelope estruturado; erro JSON-RPC só para forma
do pedido. Sem `Principal` não há resposta (nenhuma confiança implícita; stdio revalida o token a cada mensagem).
Notificações nunca executam tools. **Alternativas rejeitadas:** (a) MCP com handlers próprios (divergência de
escopo/idempotência/auditoria — viola "um pipeline"); (b) `tools/list` filtrada por escopo (esconde a superfície sem
ganho de segurança e quebra paridade com o catálogo); (c) resources que abrem projetos (leitura com efeito colateral).
**Consequências:** paridade por construção (testada: direto × REST × MCP), mesmo audit com `surface`, mesmas
limitações de uploads (inline limitado pelo teto de JSON). Promoção de memória e undo seguem fora do catálogo.

### ADR-111 — Webhooks: entrega at-least-once, HMAC v1, revalidação a cada tentativa
**Estado:** Accepted (Fase 6).
**Status:** proposto. **Decisão:** o despachante lê `events`/`deliveries` (preenchidas na mesma transação pela
bomba), nunca é chamado pelo caminho de conclusão de Run/export e entrega em tarefas concorrentes (≤ 32 em voo).
Assinatura `X-CapIA-Signature: v1=HMAC-SHA256(segredo, "<ts>.<corpo cru>")` com `X-CapIA-Timestamp`, janela de replay de
300 s, `Event-Id`/`Delivery` estáveis entre tentativas e dedupe do lado do receptor. 2xx entrega; 3xx e 4xx
(exceto 408/425/429) matam na hora; o resto tem backoff exponencial com jitter ±20% e dead-letter após
`webhook_max_attempts`; `dead` é reabrível por endpoint. A URL é revalidada a cada tentativa (política + resolvedor
filtrado), o corpo da resposta nunca é lido, o segredo só existe no `SecretStore` (indisponível ⇒ `dead
SECRET_UNAVAILABLE`, reenfileirado pela rotação). **Alternativas rejeitadas:** exactly-once (impossível sem
cooperação do receptor); seguir redirects (vazaria a assinatura/corpo a outro host); fila única sequencial (um endpoint
lento atrasaria todos); guardar o segredo no banco. **Consequências:** receptores precisam deduplicar por `Event-Id`;
reiniciar sem cofre do SO exige rotação do segredo; um 3xx é tratado como erro permanente de configuração.

### ADR-112 — Fonte única da versão do app
**Estado:** Accepted (Fase 6).
**Contexto:** versão `0.0.0` espalhada (Cargo, 7 `package.json`, `tauri.conf.json`, fixture do contrato `engine_info`).
**Decisão:** `[workspace.package] version` do `Cargo.toml` é a única fonte (`0.6.0-rc.1`). `tools/release/check-version.mjs` (`pnpm check:version`, no CI do instalador e no release) falha se `tauri.conf.json`, qualquer `package.json`, a fixture, as dependências de caminho do workspace ou um crate com versão fixa divergirem. Em runtime tudo deriva de `CARGO_PKG_VERSION` (`engine_info`, `BuildInfo`, `--version`, diagnóstico, servidor).
**Alternativas:** gerar os JSON a partir do Cargo (rejeitada: esconde divergência e quebra o editor de texto); versão por pacote (rejeitada: sem sentido para um produto único).
**Consequências:** todo bump toca ~15 arquivos (o verificador lista); pré-release `-rc.N` é válida no NSIS (WiX não aceitaria; não usamos).

### ADR-113 — Instalador NSIS por usuário
**Estado:** Accepted (Fase 6).
**Decisão:** NSIS via Tauri, `installMode: currentUser` (sem UAC; `%LOCALAPPDATA%\Programs\CapIA`; atualização sem elevação, pré-requisito do updater silencioso). Config do instalador **gerada** (`stage-bundle.mjs`) e mesclada por `--config`; `tauri.conf.json` segue com `bundle.active=false`. WebView2: `embedBootstrapper` por padrão em release (offline opcional). FFmpeg LGPL e (quando houver) `capia-server` como recursos/sidecar sob `<instalação>\ffmpeg\` (onde `MediaToolchain` já procura). Desinstalador **nunca** apaga projetos: projetos nunca vivem no diretório de instalação, e o gancho copia qualquer `.capia` encontrado ali para Documentos antes da remoção. Dados do app só saem se o usuário pedir.
**Alternativas:** instalação por máquina (rejeitada: UAC a cada update, projeto de usuário único); MSI/WiX (rejeitado: pré-release inválida, sem ganho).
**Consequências:** sem política corporativa por máquina nesta fase; para ambientes gerenciados, um instalador `perMachine` pode ser gerado depois com a mesma config.

### ADR-114 — Atualização assinada com máquina de estados persistida
**Estado:** Accepted (Fase 6).
**Decisão:** manifesto JSON assinado com Ed25519 sobre o JSON canônico sem `signature` (ordem de chaves, sem espaços), verificado sobre o JSON recebido; `Verifier` é um trait, implementação `ed25519-dalek` (puro Rust; BSD-3-Clause). Chaves públicas na build (`CAPIA_UPDATE_PUBKEYS`, múltiplas p/ rotação); sem chaves ⇒ `not_configured`. Política: canal exato, `stable` sem pré-release, sem downgrade silencioso (rollback só com manifesto marcado **e** pedido explícito), `min_version`, versões revertidas não voltam. Estados persistidos `Idle→Downloaded→Verified→Staged→Switched→Confirmed` com escrita atômica; `recover()` reconcilia estado × versão instalada × marcador de saúde; rollback automático por falha explícita de saúde ou >2 inicializações sem confirmar. Troca real atrás de `Switcher` (instalador silencioso), download atrás de `Downloader`, política de Runs atrás de `HostPolicy` (adiar/checkpoint). Hash/tamanho conferidos antes do staging e antes da troca.
**Alternativas:** `minisign`/`signify` (rejeitado: formato extra sem ganho aqui); `ring` (rejeitado: não puro Rust); updater do Tauri (rejeitado: pouco controle da máquina de estados/rollback e da política de Runs).
**Consequências:** 100% testável com falsos e injeção de falhas em todos os pontos; a troca real no Windows é o passo a validar; binário que não chega a `main()` precisa de rollback manual documentado.

### ADR-115 — Crash report opt-in, local-first
**Estado:** Accepted (Fase 6).
**Decisão:** desligado por padrão; opt-in explícito persistido no `AppDb`; gancho de pânico sempre grava registro **local** redigido (versão, SO, arquivo:linha curto, mensagem redigida; sem backtrace, sem conteúdo de projeto/mídia/prompt); envio apenas na abertura seguinte, com opt-in **e** `CrashSink` configurado; `capia-support` não tem código de rede. O usuário pode desligar e apagar os registros.
**Alternativas:** opt-out (rejeitada: política de privacidade do produto); envio dentro do gancho de pânico (rejeitada: frágil e perigoso em estado de pânico).
**Consequências:** endpoint de upload é pendência externa; a UI informa honestamente que, sem destino, os relatórios ficam locais.

### ADR-116 — Diagnóstico com preview e redação na saída
**Estado:** Accepted (Fase 6).
**Decisão:** o pacote de suporte é acionado pelo usuário, tem `preview()` com a lista exata de arquivos/tamanhos/descrições e a lista do que nunca entra; todo texto é redigido **na saída** (`redact_global` + remoção do diretório pessoal), mesmo que um segredo tenha sido escrito cru num log; só tipos conhecidos de arquivo entram (nada de `.capia`, mídia ou arbitrários); logs com rotação limitada (1 MiB×5), linhas truncadas.
**Alternativas:** confiar só na redação de escrita (rejeitada: logs de terceiros/crashes); incluir projeto "para ajudar" (rejeitada: dado do usuário).
**Consequências:** testes com canário no bundle/log/erro/crash são requisito de regressão.

### ADR-117 — Fixture de projeto grande e política do gate de desempenho
**Estado:** Accepted (Fase 6).
Status: **proposta** (o integrador numera e move para `docs/DECISIONS.md`).

## Contexto
A Fase 6 exige medir o engine num projeto grande (≥ 30 sequences, ≥ 5.000 clips, nested, legendas, áudio,
catálogo e histórico de Runs) preservando as metas da Fase 3, com um gate de regressão que não seja nem
frouxo (aprovar em vazio) nem frágil (reprovar por ruído de runner).

## Decisões
1. **Fixture = crate dev-only `capia-fixtures`.** Constrói um `.capia` real **somente** por Command Engine,
   `Catalog`/`AutonomyStore` públicos; é determinística por `seed` (`document_digest` estável). Entra na matriz
   de arquitetura como crate de apoio: dependência **apenas** via `[dev-dependencies]`, ignorada no grafo de
   ciclos (o ciclo project ⇄ fixtures é de teste). Sem SQL cru: se o produto não consegue gravar, a fixture não grava.
2. **Catálogo grande sem arquivos.** Registros sintéticos com disponibilidade `offline` (o catálogo aceita
   caminhos inexistentes); evita gerar milhares de arquivos e custa o mesmo para abrir/listar/`assets.list`.
3. **Relatório com amostras brutas.** O benchmark grava p50/p95/máx **e** as amostras, mais a máquina
   (CPU, núcleos, RAM, kernel, rustc, perfil). Números sem máquina não são comparáveis.
4. **Gate = mediana de N=3 execuções** por estatística, comparada a `thresholds.json`. Metas documentadas
   (`hard`: 30/50/100/16 ms, 2 s) **nunca** recebem tolerância; limites de regressão = ≈ 3× o p95 medido × fator
   1,5 (substituível por `CAPIA_PERF_TOLERANCE`). Métrica ausente/NaN/sem limite/pulada sem motivo permitido
   **reprova**. Afrouxar um limite `hard` exige novo ADR (há teste que trava os valores).
5. **Asserções no benchmark só de sanidade**; metas finas ficam no gate (um p95 isolado é ruído).
6. **Falha de disco**: "disco cheio" simulado por `RLIMIT_FSIZE` (`ulimit -f` + SIGXFSZ ignorado) num processo
   filho — exercita o caminho de erro de escrita (`EFBIG`) sem root. ENOSPC real e falha de `fsync` ficam fora
   (exigem mount/root) e são declarados como não cobertos.
7. **Soak**: testes rápidos de vazamento (fd/threads/RSS via `/proc/self`; pulam com motivo fora do Linux);
   soak longo (`soak.mjs --seconds`) manual/noturno, nunca no CI principal.
8. **Migração**: matriz v1..atual com dados em todas as tabelas; rollback = backup `.vN.bak` pré-migração;
   migração só para frente; schema novo é recusado por build antigo sem tocar o arquivo. Novo schema ⇒ estender a
   matriz (teste falha de propósito se `MIGRATIONS.len()` mudar sem atualizar).

## Alternativas rejeitadas
* Limites fixos de ms para tudo sem mediana: instável em runner compartilhado.
* Mediana + tolerância também nas metas documentadas: deixaria uma regressão material passar.
* Gerar mídia real para o catálogo: lento, sem ganho de cobertura para abrir/listar.
* Migração reversa: complexidade sem demanda; backup atômico basta.

## Consequências
Corrigido junto: `catalog::read_all` falhava em schema 2 (`validate_file`/`inspect` antes de migrar). Resíduos:
commit com snapshot a cada commit ≈ 100 ms (pior caso sintético); retenção/verificação de backups e checagem de
espaço livre pré-migração não existem (candidatos a trabalho futuro).

### ADR-118 — Endurecimento de segurança do servidor REST (D2-1…D2-7)
**Estado:** Accepted (Fase 6).
> Rascunhos para consolidar em `docs/DECISIONS.md` (o integrador numera). Contexto: Fase 6,
> `docs/phase6/PHASE6_PERFORMANCE_SECURITY.md` §9–28 e `docs/phase6/PENTEST_REPORT.md`.

## D2-1 — Segredos **registrados** nunca entram no pipeline do servidor (redação na entrada)

**Contexto.** O redator central (`capia-secrets`) já cobria erros, auditoria, diagnóstico e pânico,
mas o **eco de sucesso** não: um token/chave colado num nome de projeto, descrição de webhook, nome
de arquivo, brief ou `X-Request-Id` voltava na resposta e ficava gravado no `server.db`/projeto.

**Decisão.** (1) `Core::call_def` aplica `redact_registered` (só valores exatos registrados e suas
codificações; sem heurística, para não alterar texto legítimo) a **todas** as strings e chaves dos
parâmetros **antes** de schema/auditoria/handler — vale igual para REST e MCP. (2) O mesmo para
`X-Request-Id` e `X-Capia-Filename`. (3) `authenticate` registra o bearer reconhecido (tokens criados
por outro processo/CLI passam a ser "conhecidos" assim que usados). (4) `ApiErr::body` redige também
`details`. (5) A `Idempotency-Key` do cliente é um nonce, não um segredo: a **tabela de idempotência** só guarda
um digest (`ik_<sha256[..40]>`) e a auditoria mostra a chave para correlação, redigida se ela
coincidir com um segredo conhecido.

**Alternativas.** Recusar (422) pedidos que contenham segredo conhecido — rejeitado: transforma um
descuido do usuário em erro opaco e vaza a existência do segredo por oráculo; só redigir é mais
simples e seguro. Redigir só na saída — rejeitado: o dado já teria ido para o disco.

**Consequências.** Canário dos testes (`tests/secret_canary.rs`) cobre respostas, SSE, OpenAPI, banco
(+WAL), nomes de arquivo, stdout/stderr de um processo real. Limite: só valores **registrados** no
processo (tokens já apresentados/emitidos, chaves de provider carregadas, segredos de webhook).

## D2-2 — Nomes de arquivo do cliente: reservados do Windows e arquivos internos são prefixados

`sanitize_filename` agora aparta ponto/espaço finais e prefixa `_` em `CON/PRN/AUX/NUL/COM1-9/LPT1-9`
(com ou sem extensão) e em `meta.json[.tmp]` (case-insensitive: NTFS/APFS). Antes, `NUL.png` no
Windows abriria o dispositivo e `meta.json` era sobrescrito pelo próprio metadado do staging. O nome
em disco nunca mais é "o que o cliente mandou": é sempre `[A-Za-z0-9._ -]{1,120}`, sem ponto inicial.

## D2-3 — Upload só vale se chegar completo; staging órfão é varrido; chaves pendentes viram indeterminadas

(a) `store_upload` compara bytes recebidos com o `Content-Length` declarado: menos ⇒ 400 e nada fica
staged (antes, um cliente que caía no meio produzia um upload "completo" truncado quando o prefixo
passava no sniff). (b) Na abertura, diretórios `uploads/upl_*` sem `meta.json` (queda no meio do
streaming) são removidos. (c) Na abertura, `idempotency.status = 0` (processo morto) vira
indeterminado imediatamente (`idem_recover`) em vez de `IDEMPOTENCY_IN_PROGRESS` por 5 min; linhas
concluídas com mais de 24 h são aparadas (`idem_purge_older_than`).

## D2-4 — Política de IP de saída (webhooks/providers) normaliza IPv6 que embute IPv4

`UrlPolicy::check_ip` primeiro converte `::ffff:a.b.c.d` em IPv4 (o loopback mapeado passava quando
`allow_loopback=false`), e `blocked_v6` bloqueia IPv4-compatível (`::/96`), NAT64 (`64:ff9b::/96`) e
6to4 (`2002::/16`) com IPv4 bloqueado embutido. O registro de webhook exige URL **canônica**
(`scheme://host…`, sem `///`, `:/`, `\`, espaço): o que é gravado é o que o cliente HTTP vai discar.

## D2-5 — Cabeçalhos: espaço opcional é só SP/TAB; `Authorization` duplicado é 400; config CORS endurecida

`http::read_request` apara OWS ASCII (nunca NBSP/Unicode) e recusa `Authorization` duplicado (como
`Content-Length`/`Host`); `bearer()` idem. `ServerConfig::validate` recusa origens CORS `null`,
vazias, com `/` final ou com `*` em qualquer posição.

## D2-6 — Diretório de dados com permissão 0700 (Unix)

`Core::open` ajusta `data_dir` para `0700`: outro usuário local não lê projetos/mídia/uploads/`server.db`.
No Windows vale a herança de ACL do perfil do usuário (documentado como pré-requisito de instalação).

## D2-7 — Suíte de segurança como gate

`tools/phase6-acceptance/security-suite/run.mjs` roda pentest + canário + fuzz + queda + unitários +
`pnpm check:arch` e, opcionalmente, `tools/mutation-phase6.py` (37 mutações; cada uma deve ser
detectada). Mutante sobrevivente é achado e exige teste novo — nunca enfraquecer asserção.

### ADR-119 — Documentação gerada do catálogo, evidência externa honesta e processo de release
**Estado:** Accepted (Fase 6).
> Rascunho para o integrador incorporar a `docs/DECISIONS.md` com o próximo número livre. Não editar `DECISIONS.md` nesta frente.

## Contexto

A Fase 6 precisa de documentação de API que não divirja do código, de um pacote de aceitação que **nunca fabrique** resultados externos (certificado real, máquinas Windows limpas, usuários de beta, provedores reais) e de um processo de release repetível.

## Decisões

1. **Documentação de API derivada do catálogo único (ADR-102).** `docs/api/rest-reference.md`, `mcp-tools.md`, `openapi.json` e a matriz de scopes são **gerados** por `tools/docs/gen-api-docs.mjs` a partir do JSON do catálogo (`capia-server catalog`, ou o fixture `tools/docs/fixtures/catalog.json` transcrito mecanicamente do Rust). O gerador **não** interpreta Rust. `--check` falha se a documentação estiver defasada. Textos escritos à mão (convenções, auth, webhooks) ficam fora dos blocos gerados; só a matriz de scopes é injetada entre marcadores em `auth-and-scopes.md`.
2. **Vocabulário único de estado de aceitação:** `passed | failed | pending_external | not_available`. `not_available` = o passo não pôde rodar aqui (binário/teste de outra frente, variável, `CAPIA_P6_HEAVY`) e **nunca** conta como aprovado. O agregado é o **pior** estado; só “tudo `passed`” é `PHASE 6 COMPLETE`; suíte `not_available` impede declarar engenharia completa (`INCOMPLETE`).
3. **Evidência externa só por arquivo real validado.** Validadores (`clean-machine`, `beta-feedback`, `update`, `installer`, paridade) respondem `pending_external` sem arquivo ou com modelo (`"template": true`), `rejected` para malformado/inconsistente/que registra falha, `partial` para evidência bem formada mas incompleta (ex.: só uma versão do Windows; artefato não assinado; update assinado não executado por falta de certificado) e `accepted` só com todas as regras. Exigem atestados explícitos do executor e checagens de plausibilidade (datas não futuras, builds do Windows coerentes, hashes, igualdade recalculada). Um resultado negativo honesto é evidência válida que reprova o gate.
4. **Gate de beta:** “nenhum Blocker/Critical aberto para o RC”: aberto = `open`, `wontfix` ou `fixed` em versão posterior ao RC; `wontfix` nunca fecha Blocker/Critical. Mínimo de 5 usuários externos é **escolha de produto documentada**, parametrizável (`--min-users`); usuários internos não contam.
5. **Passos pesados** (compilam Rust, build do desktop) só rodam com `CAPIA_P6_HEAVY=1`; `steps.json` é declarativo e aponta para nomes **esperados** de testes de outras frentes, tratados como `not_available` enquanto não existirem.
6. **Exemplos não são produto:** vivem em `examples/`, sem dependência de crates/pacotes, com `.mjs` no lint/format; testados contra servidores falsos e com vetores HMAC independentes; passam a valer como integração real apenas quando executados no item `external-flow`.
7. **Documentos de release** (`RELEASE.md`, `CHANGELOG.md`, `KNOWN_ISSUES.md`, `MIGRATION_COMPAT.md`) são verificados por `installer/check-release.mjs` (versão única nas três fontes; seções e checklist presentes).

## Alternativas

- Escrever a referência da API à mão (diverge do código); parsear Rust por regex (frágil); `pending` genérico sem distinguir “não pôde rodar” de “depende de humano” (esconde lacunas de integração); tratar evidência faltante como “skipped/ok” (viola a regra de não fabricar).

## Consequências

- Mudar o catálogo exige regenerar `docs/api` (CI pode rodar `pnpm check:docs`).
- O estado final reportado pelo agregador é conservador por construção; fechar a Fase 6 exige as evidências externas listadas em `docs/phase6/IMPL_DOCS_ACCEPTANCE.md`.
- Os nomes esperados de testes de outras frentes em `steps.json` precisam ser ajustados pelo integrador quando divergirem.

## Evidência

`tools/docs/gen-api-docs.test.mjs`, `tools/phase6-acceptance/**/*.test.mjs`, `examples/**/*.test.mjs`, `tools/sample-project/make-sample.test.mjs` (todos em `pnpm test:tools`).
