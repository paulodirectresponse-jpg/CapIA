# Segurança da API local (resumo do modelo de ameaças)

Complementa `docs/SECURITY.md` (credenciais, redação, segurança da IA). Aqui: o que muda quando o CapIA aceita clientes externos. Documentos de origem: `docs/phase6/PHASE6_API_MCP_WEBHOOKS.md` e `docs/phase6/PHASE6_PERFORMANCE_SECURITY.md`.

> **Estado.** Os itens abaixo são **requisitos de projeto e defesas já presentes no código** (o que está implementado hoje é indicado). A suíte de segurança da Fase 6 (`tools/phase6-acceptance/security/`) é o lugar onde cada item é verificado contra o servidor real; um **pentest externo** não foi feito. “Verificado em teste” só vale para o que a tabela “Evidência” cita.

## Ativos e fronteiras

- **Ativos:** timeline/projeto do usuário, mídia local, chaves de provider (cofre do SO), tokens da API, segredos de webhook, orçamento (dinheiro) das Runs.
- **Fronteira principal:** processo `capia-server` em loopback. Qualquer processo do mesmo usuário na máquina pode tentar falar com a porta; por isso **todo** pedido exige token e scope.
- **Limite reconhecido:** um malware rodando **como o mesmo usuário do SO** pode ler arquivos do usuário e o ambiente do processo; nenhum token local resiste a isso. A API reduz o raio (scopes, auditoria, revogação), não elimina a premissa de confiança no perfil do usuário.

## Ameaças e defesas

| Ameaça | Defesa | Onde |
|---|---|---|
| Outro processo local usa a API | bind em `127.0.0.1`; token Bearer por cliente; scopes mínimos; auditoria por token; revogação imediata | catálogo + servidor |
| Exposição na rede | bind remoto só por configuração explícita, com aviso, token obrigatório e TLS (proxy reverso); tokens nunca em texto claro fora de loopback | requisito de projeto |
| Roubo/vazamento de token | só o SHA-256 é guardado; segredo mostrado uma vez; rotação e expiração; mensagens de erro passam pelo redator central | `serverdb.rs` (`api_tokens`), `error.rs` (`redact_global`) |
| Escalada de privilégio por rota | **toda** operação do catálogo declara exatamente um scope (exceto `server.health`); teste do catálogo falha se faltar; REST e MCP leem o mesmo catálogo | `catalog.rs` (testes `every_operation_declares_a_scope…`) |
| Ação destrutiva ou gasto por cliente | escrita só por `preview → apply_plan` (ator `Api`) e Runs; aprovação (`run:approve`) é scope separado, presa ao digest do plano; orçamento com reserva/liquidação; undo. **Desfazer uma Run e aprovar memória (User/Client) não são operações da API/MCP**: só um humano, pela UI (teste `nothing_dangerous_is_in_the_catalog`) | Engine API, Fase 5 |
| Corpo gigante, JSON profundo | limites de tamanho/profundidade; schemas fechados (`additionalProperties: false`, tamanhos máximos por campo) | `catalog.rs` (teste `every_schema_compiles_and_forbids_unknown_fields`) |
| Path traversal / saída arbitrária | a API nunca recebe caminho de arquivo do cliente para ler: mídia entra por **upload** + importação pelo sistema de assets; export só escreve na política de pasta de saída (staging → validação → `rename`) | catálogo (não há `path` nos schemas de entrada), export (ADR-068) |
| Upload malicioso (bomba, tipo falso) | streaming com cota/timeout, hash, staging, **sniffing** do conteúdo (nunca confiar em nome/MIME do cliente), importação pelo pipeline hostil-por-padrão (FFmpeg/ffprobe sem shell, com timeout) | requisito de projeto; `capia-media` |
| SSRF por URL do cliente (webhook, fonte do Gateway) | política única de URL: `https` (ou `http` só loopback), sem credencial na URL, DNS filtrado (sem rede privada/link-local/metadata), **sem redirect**, timeouts, corpo da resposta não lido | `capia-ai/src/webhook.rs` (`tests/webhook.rs`), `SafeFetcher` |
| Webhook forjado/reenviado (do lado do receptor) | HMAC-SHA256 sobre `timestamp.corpo`, janela de 5 min, de-duplicação por event id | [webhooks.md](webhooks.md); receptores de exemplo testados |
| Endpoint de webhook hostil/lento | timeouts (5 s/15 s), corpo da resposta descartado, retry com backoff, dead-letter; nunca bloqueia a Run | `webhook.rs` |
| Injeção de prompt por argumentos de tool/briefing | conteúdo de cliente = `untrusted_data`; papéis de LLM sem tools; lista fechada de comandos; sem shell/FS/HTTP/segredos | Fases 4–5, [mcp.md](mcp.md) |
| Repetição de requisição (retry duplicando efeito) | `Idempotency-Key` por token com hash do pedido; resultado guardado; indeterminado nunca re-executa às cegas | `serverdb.rs` (`IdemBegin`) |
| CSRF / páginas web chamando a API | CORS fechado por padrão; token Bearer em cabeçalho (não cookie); `Host` validado | requisito de projeto |
| Vazamento de segredo por resposta/log/crash | segredos fora de respostas, logs, diagnóstico e relatórios de crash; canário de segredo nas suítes | Fases 4–6 |
| Negação de serviço local | rate limit por token e classe, teto de Runs simultâneas, backpressure (sem criar tarefas sem limite) | `catalog.rs` (classes), `TOO_MANY_RUNS` |
| Auditoria insuficiente | log de chamadas externas (superfície, operação, resultado, revisões, chave de idempotência) | `serverdb.rs` (`audit`), `audit.list` |

## Evidência hoje (verificada no repositório)

- Catálogo: toda operação tem scope (exceto o health); nomes/rotas únicos; leituras nunca mutam; schemas fecham campos; nada perigoso no catálogo (`crates/capia-server/src/catalog.rs`, módulo de testes).
- Primitivas de MAC: HMAC-SHA256 contra os vetores RFC 4231 e comparação em tempo constante (`crates/capia-server/src/mac.rs`).
- Cliente de entrega de webhooks: política de URL, sem redirect, timeouts (`crates/capia-ai/tests/webhook.rs`).
- Banco do servidor: tokens só como hash, idempotência, auditoria, entregas com estados (`crates/capia-store/src/serverdb.rs`).
- Receptores de exemplo: assinatura, replay, dedupe, forja (`examples/webhook-receiver/`).

## Evidência pendente (a produzir pela suíte da Fase 6 / externa)

401 sem token, 403 por scope, revogação, escalada por rota, replay de idempotência, revisão desatualizada, JSON malformado, traversal, upload acima do limite, SSRF, CORS, forja/replay de webhook, fuzzing REST e MCP, canário de segredo por REST/MCP/webhook/relatório de crash/diagnóstico, kill durante upload/apply/Run/webhook/export. Ver `tools/phase6-acceptance/security/`. **Pentest independente: externo, não executado.**

## Reportar uma vulnerabilidade

Não abra issue pública com detalhes exploráveis. Envie o relato (versão, passos, `request_id` se houver) ao mantenedor do projeto por canal privado; **nunca** inclua tokens, chaves ou mídia de terceiros. O bundle de diagnóstico redigido ([../user/07-solucao-de-problemas.md](../user/07-solucao-de-problemas.md)) é seguro para anexar.
