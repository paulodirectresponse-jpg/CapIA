# ADR DRAFT (Trilha B) — MCP como adaptador, webhooks at-least-once assinados

Rascunho para o integrador numerar e mover para `docs/DECISIONS.md` (esta trilha não edita aquele arquivo).

## ADR-1xx — MCP é um adaptador fino sobre `Core::call`

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

## ADR-1xx — Webhooks: entrega at-least-once, HMAC v1, revalidação a cada tentativa

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

## Observações para o integrador

* `ServerConfig` ganhou uso de `webhook_max_attempts/backoff_*` (já existentes) — sem campos novos.
* `tokio` do `capia-server` agora declara `rt-multi-thread` e `net` (a matriz de arquitetura já permitia `tokio`).
* Novo `mcp::Headless` (Core + pump + despachante sem socket) usado por `capia-server mcp-stdio`.
