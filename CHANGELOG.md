# Changelog

Formato [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/); versionamento [SemVer](https://semver.org/lang/pt-BR/). Cada entrada diz o que **existe no repositório** e distingue o que é verificado por CI do que depende de verificação **externa** (humano, hardware, certificado, provedor real) — ver [`docs/KNOWN_ISSUES.md`](docs/KNOWN_ISSUES.md).

## [Unreleased]

### Pendente para o release final
- Certificado de assinatura de código real e verificação do instalador/atualizador assinados em Windows 10 22H2 e 11 limpos.
- Beta com usuários reais e fechamento do gate “sem Blocker/Critical aberto”.
- Decisão de produto/jurídica sobre H.264/AAC.

## [0.6.0-rc.2] - 2026-10-06

Correção depois de um teste manual real do RC1 por um editor experiente. **Não** declara a Fase 6 completa nem aceita nada externo. Evidências: `docs/RC2_TEST_REPORT.md`.

### Corrigido
- **Transição entre clipes inteiros** falhava com "sem sobra de mídia" (`INSUFFICIENT_HANDLES`). Agora a dissolução congela o quadro da borda quando não há sobra (ADR-120); preview e export continuam idênticos. A UI aplica a transição no corte quando o clipe da esquerda está selecionado e, em clipe isolado, diz "Coloque a transição entre dois clipes encostados."
- **Piscada no preview**: o canvas deixou de ser apagado a cada mudança de tamanho e a qualidade automática não troca de resolução por um pico isolado de latência (confirmação sustentada + intervalo mínimo).
- **CI Windows**: colisão de diretório temporário no self-test do desktop e socket herdando modo não-bloqueante no receptor de teste de webhook.

### Adicionado
- **OpenAI nativo** (`POST /v1/responses`: texto, streaming, tools, saída estruturada, imagem, uso, erros, cancelamento) quando a base URL é `api.openai.com`; os demais provedores compatíveis seguem em `chat/completions`.
- **Conectar IA** (uma ação): importa modelos, escolhe o padrão, mede cada capacidade (conexão/auth, texto, streaming, tools, saída estruturada, visão) e cria o Brain Profile `auto`; habilita `whisper-1` à parte para transcrição. O probe reporta `ready | partial | failed` com honestidade.
- **Chat com contexto da UI**: seleção e playhead chegam ao assistente (ids sanitizados), então "este clipe" tem referente; resposta a "o que você pode fazer?" sem tools.
- **Home** com projetos recentes; idioma inicial pelo sistema; tracks "Vídeo N / Áudio N"; losango de keyframe clicável ao lado de Posição/Escala/Rotação/Opacidade; Error Boundary global em pt-BR; progresso de export (%, decorrido, ETA, quadros/s, ×tempo real); textos de mídia offline/relink em linguagem simples.
- **Jornada E2E de 25 passos** (`packages/e2e/tests/rc2-journey.spec.ts`).
- `InstallerSwitcher` do updater (stage atômico, switch, restauração do instalador retido).

### Ainda pendente (externo — nunca marcado como feito)
OpenAI real com chave do usuário, certificado de assinatura, Windows 10/11 limpos, beta humano, decisão jurídica de H.264/AAC, pentest independente, endpoint de crash. Trilhas **não** feitas neste RC: layout de painéis (barra lateral reduzida/inspetor contextual), auditoria de acessibilidade em 1366×768 e escalas de 125/150%, simplificação de texto/legendas, descoberta de sequences aninhadas, medição de gargalo do export.

## [0.6.0-rc.1] - 2026-10-05

Release candidate da **Fase 6 (Integração e Finalização)**. Estado: `PHASE 6 ENGINEERING COMPLETE — EXTERNAL RELEASE ACCEPTANCE PENDING` só vale quando `node tools/phase6-acceptance/run-all.mjs` reportar isso; até lá, o estado é o do `docs/STATUS.md`. Esta versão **não** declara a Fase 6 completa.

### Adicionado
- **`capia-server`** (host headless da Engine API, ADR-102): REST `/v1` com **58 operações** declaradas em um catálogo único (scope, efeito, classe de rate limit, schema), `Idempotency-Key`, SSE (`GET /v1/events/stream`), OpenAPI (`GET /v1/openapi.json`), tokens com scopes (11), auditoria, upload seguro, bind em loopback por padrão; endpoint MCP (`POST /mcp`) e tools derivadas do mesmo catálogo; webhooks assinados (HMAC-SHA256) com retry e dead-letter. *Estado:* contrato e núcleo REST no repositório; MCP stdio, entrega de webhooks e paridade UI×REST×MCP em integração/verificação externa.
- **Documentação do usuário** (`docs/user/`, pt-BR) e **da API** (`docs/api/`, com referência REST e matriz de scopes **geradas do catálogo** por `tools/docs/gen-api-docs.mjs`, `openapi.json` e tabela de tools MCP).
- **Exemplos** (`examples/`): fluxo canônico por curl, PowerShell, Node e Python; cliente MCP; receptores de webhook (Node e Python) que verificam assinatura, janela de 5 minutos e de-duplicação, com testes de vetores independentes.
- **Pacote de aceitação da Fase 6** (`tools/phase6-acceptance/`): executor declarativo (`passed|failed|pending_external|not_available`), validadores de evidência de máquina limpa, beta, instalador/assinatura, update/rollback e paridade; agregador `run-all.mjs`.
- Processo de release: `docs/RELEASE.md`, `docs/MIGRATION_COMPAT.md`, `docs/KNOWN_ISSUES.md`, projeto de exemplo sintético (`tools/sample-project/`).

### Entregue nas fases anteriores (consolidado)
- **Fase 1 — Fundação:** workspace Cargo + pnpm, núcleo Rust headless (`capia-time` com `Ticks` inteiros, `capia-model`, `capia-commands`) que compila para WASM, regras de arquitetura verificadas, CI.
- **Fase 2 — Motor:** Command Engine transacional com undo/redo e histórico; persistência `.capia` (SQLite, migrations só para frente); assets por conteúdo (SHA-256), relink, jobs/proxies/waveforms; render graph + compositor + mixer; export com encoders aprovados (staging → ffprobe → `rename`). Sem x264/x265.
- **Fase 3 — Editor:** UI de edição manual completa (timeline em canvas, inspector, keyframes, texto/legendas/transições, preview, histórico, export/entregáveis, atalhos, pt-BR/en), shell Tauri.
- **Fase 4 — Inteligência:** providers (OpenAI-compatível, Anthropic, Google, locais), Capability Router, chaves no cofre do SO com redação central, transcrição, legendas automáticas, remoção de silêncio, detecção de cenas, Reference Analyzer, Demand Interpreter, assistente.
- **Fase 5 — Autonomia:** AI Run persistente e retomável (planner/editor/critic), aprovações e orçamento, memória em 4 escopos, Asset Gateway com proveniência, geração opt-in, variantes, undo seletivo, Critic com visão.

### Segurança
- Tokens guardados só como SHA-256; segredos de token e de webhook mostrados uma única vez; mensagens de erro passam pelo redator central; webhooks sem redirect e com política anti-SSRF; catálogo sem shell/FS/HTTP/segredos e sem undo/aprovação de memória.

### Limitações conhecidas
Ver [`docs/KNOWN_ISSUES.md`](docs/KNOWN_ISSUES.md): nenhuma avaliação com provedores reais, usuários reais, máquina limpa física, certificado de assinatura ou revisão jurídica de H.264/AAC foi executada.
