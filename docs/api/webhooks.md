# Webhooks

O CapIA avisa o seu sistema quando algo termina, por `POST` assinado para uma URL que você registra. Webhooks são **opcionais** (o editor e a API funcionam sem eles) e **nunca bloqueiam** uma Run ou um export: se o seu endpoint estiver fora, a entrega entra em retry e, no fim, em dead-letter — a Run continua e conclui normalmente.

## Registro (`webhook:manage`)

| Operação | Rota |
|---|---|
| `webhooks.create` | `POST /v1/webhooks` — `{url, events[], description?}` (1–16 eventos; `"*"` assina todos) |
| `webhooks.list` / `webhooks.get` | `GET /v1/webhooks`, `GET /v1/webhooks/{webhook_id}` |
| `webhooks.update` | `PATCH /v1/webhooks/{webhook_id}` — `url`, `events`, `description`, `enabled` |
| `webhooks.rotate_secret` | `POST /v1/webhooks/{webhook_id}/rotate-secret` |
| `webhooks.delete` | `DELETE /v1/webhooks/{webhook_id}` |
| `webhooks.test` | `POST /v1/webhooks/{webhook_id}/test` — dispara `webhook.test` (202) |
| `webhooks.deliveries` | `GET /v1/webhooks/{webhook_id}/deliveries` — log de entregas |
| `webhooks.redeliver` | `POST /v1/webhooks/{webhook_id}/deliveries/{delivery_id}/redeliver` (202) |

### Segredos são write-only

O segredo de assinatura é gerado pelo servidor e mostrado **uma única vez**, na resposta de `webhooks.create` (e de `webhooks.rotate_secret`). Depois disso nenhuma rota o devolve: `webhooks.get/list` não trazem o segredo. O servidor o guarda no cofre de segredos (no banco só há uma referência, `secret_ref`) e o usa só para assinar. Se perder o segredo, **rotacione**. Guarde-o no cofre do seu lado, nunca em Git.

### Política de destino (SSRF)

A URL passa pela política do cliente de entrega do servidor (`crates/capia-ai/src/webhook.rs`), no registro **e** a cada entrega: `https` (ou `http` **somente em loopback** — o receptor local é o caso de uso principal), sem credenciais na URL, DNS filtrado (sem rede privada, link-local ou endereço de metadados de nuvem), **nenhum redirect seguido** (um `3xx` conta como falha), timeout de conexão de 5 s e total de 15 s, e o corpo da sua resposta nunca é lido (só o status importa). Um destino recusado não vira entrega.

## Eventos

Tipos publicados: `run.started`, `run.waiting_user`, `run.completed`, `run.failed`, `run.cancelled`, `export.completed`, `export.failed`, `asset.imported`, `webhook.test`. Assine só o que usa. **Ignore tipos que não conhece** (novos eventos são mudança aditiva).

| Tipo | Quando | `data` típico (interface prevista; os campos de topo são o contrato) |
|---|---|---|
| `run.started` | a Run saiu da fila | estágio inicial |
| `run.waiting_user` | decisão pendente (aprovação, pergunta, extensão de orçamento) | tipo da decisão e `decision_id` |
| `run.completed` | Run concluída | sequences produzidas, resumo de uso/custo |
| `run.failed` | Run falhou | erro redigido (código, mensagem, estágio) |
| `run.cancelled` | Run cancelada | — |
| `export.completed` | export validado e publicado | caminho/relatório de validação |
| `export.failed` | export falhou/cancelado | erro redigido |
| `asset.imported` | importação de asset concluída | `asset_id` |
| `webhook.test` | você chamou `webhooks.test` | mensagem fixa |

### Payload (versão 1)

O corpo é JSON UTF-8:

```json
{
  "id": "evt_0001",
  "type": "run.completed",
  "version": 1,
  "occurred_at": "2026-01-01T00:00:00Z",
  "occurred_ms": 1767225600000,
  "project_id": "prj_example",
  "run_id": "run_example",
  "export_id": null,
  "data": {}
}
```

`project_id`, `run_id` e `export_id` são `null` quando não se aplicam. `data` nunca contém segredos nem conteúdo de mídia. O `id` é **único por evento** e idêntico em todas as tentativas de entrega.

## Cabeçalhos e assinatura

| Cabeçalho | Valor |
|---|---|
| `X-CapIA-Timestamp` | instante do envio (Unix, em segundos; receptores robustos aceitam também milissegundos) |
| `X-CapIA-Signature` | `v1=<hex>`, em que `<hex>` é o **HMAC-SHA256** (minúsculas, 64 hex) da string `"<timestamp>.<corpo bruto>"` com o segredo do webhook como chave. Durante uma rotação pode haver mais de um `v1=…` separado por vírgula: aceite se **qualquer** bater |
| `X-CapIA-Event-Id` | o `id` do evento (use para de-duplicar) |
| `X-CapIA-Delivery` | id desta entrega (uma entrega tem várias tentativas) |
| `X-CapIA-Attempt` | número da tentativa, a partir de 1 |

> O formato exato do `X-CapIA-Timestamp` (segundos × milissegundos) e dos ids segue a implementação do servidor; os receptores de exemplo aceitam ambos para o timestamp. O restante acima é o contrato do projeto.

### Como verificar (obrigatório)

1. Leia o **corpo bruto** (bytes) **antes** de qualquer parse. Re-serializar o JSON muda os bytes e invalida a assinatura.
2. Rejeite se faltar `X-CapIA-Timestamp`/`X-CapIA-Signature` ou se estiverem malformados.
3. Rejeite se `|agora − timestamp| > 5 minutos` (janela de replay; também para timestamps no futuro).
4. Calcule `HMAC-SHA256(segredo, timestamp + "." + corpo)` e compare **em tempo constante** (`timingSafeEqual` / `hmac.compare_digest`) com cada `v1=` do cabeçalho.
5. **Só depois** de a assinatura ser válida: de-duplique por `X-CapIA-Event-Id` (entrega repetida do mesmo evento → responda `2xx` sem reprocessar) e processe. De-duplicar antes de verificar permitiria a um atacante “queimar” ids.
6. Responda `2xx` rápido (< 15 s, idealmente < 1 s) e processe em segundo plano. O corpo da resposta é ignorado.

Receptores completos e testados, com vetores calculados de forma independente: [`examples/webhook-receiver/receiver.mjs`](../../examples/webhook-receiver/receiver.mjs) e [`receiver.py`](../../examples/webhook-receiver/receiver.py).

Exemplo mínimo (Node):

```js
import { createHmac, timingSafeEqual } from "node:crypto";

function verify(secret, timestamp, signatureHeader, rawBody /* Buffer */, nowMs = Date.now()) {
  if (Math.abs(nowMs - Number(timestamp) * 1000) > 5 * 60 * 1000) return false;
  const expected = createHmac("sha256", secret)
    .update(`${timestamp}.`)
    .update(rawBody)
    .digest();
  return signatureHeader
    .split(",")
    .map((p) => p.trim())
    .filter((p) => p.startsWith("v1="))
    .some((p) => {
      const got = Buffer.from(p.slice(3), "hex");
      return got.length === expected.length && timingSafeEqual(got, expected);
    });
}
```

## Entrega, retries e dead-letter

Cada evento publicado vira uma entrega por webhook inscrito (no máximo uma por `webhook × evento`; o banco impõe a unicidade). Estados do log (`deliveries`): `pending → delivering → delivered`, ou `retrying` (com `next_attempt`) e, por fim, **`dead`** (dead-letter).

- **Sucesso:** qualquer `2xx`.
- **Falha transitória** (erro de rede, timeout, `5xx`, `408`, `425`, `429`, `3xx`): nova tentativa com **backoff exponencial**; ao esgotar as tentativas a entrega vai para `dead`.
- **Falha permanente** (`4xx` exceto 408/425/429): o seu endpoint recusou — repetir não adianta, a entrega vai direto para `dead`.
- O número de tentativas e os intervalos exatos são definidos pelo servidor (interface prevista); `X-CapIA-Attempt` informa a tentativa atual. O estado das entregas é persistido no banco do servidor (a retomada após reinício é interface prevista).
- O log guarda, por tentativa: status HTTP, latência, erro curto (sem URL/segredo), próxima tentativa e estado terminal. Consulte com `webhooks.deliveries`.
- **Reentregar:** `webhooks.redeliver` enfileira de novo uma entrega (inclusive `dead`) com o **mesmo** `X-CapIA-Event-Id` — seu receptor de-duplica ou reprocessa conforme a sua política. Conserte o endpoint primeiro.
- **Ordem não é garantida** entre eventos diferentes (retries reordenam). Use `occurred_ms` e consulte o estado atual (`runs.get`) em vez de confiar na sequência.

## Testar sem expor nada

`webhooks.test` entrega um `webhook.test` real, assinado, ao seu endpoint. Não existe “echo público”: use o receptor local de exemplo em `127.0.0.1`:

```
CAPIA_WEBHOOK_SECRET=<segredo da criação> node examples/webhook-receiver/receiver.mjs --port 9000
```

Registre `http://127.0.0.1:9000/` e chame `webhooks.test`.

## Segurança do receptor — checklist

- Verifique a assinatura em **toda** requisição; responda 401/400 para o resto.
- Corpo bruto, comparação em tempo constante, janela de 5 minutos, de-duplicação **depois** da verificação.
- Limite o tamanho do corpo; não faça `eval`/shell com `data`.
- HTTPS fora de loopback. Não registre o segredo em logs.
- Rotacione o segredo ao suspeitar de vazamento (`webhooks.rotate_secret`) e aceite as duas assinaturas durante a troca.
