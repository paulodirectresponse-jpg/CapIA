# 5. Aprovações e orçamentos

A Run para e pede sua decisão (**Esperando você**) em pontos definidos por uma **política** — por padrão conservadora. Cada decisão mostra a **pergunta**, as **opções** e **“Se você escolher isto: …”**. Uma decisão fica **presa ao plano/estado** que a originou: se o plano mudou, a decisão é velha e você é consultado de novo (nada é aprovado “no escuro”).

## Quando ela pergunta

| Decisão | Quando | Opções típicas |
|---|---|---|
| **Pergunta em aberto** | briefing incompleto (até 2 rodadas por padrão) | responder |
| **Aprovação do plano** | **sempre** por padrão (caixa “Pedir minha aprovação do plano antes de editar”) | aprovar · pedir mudanças (replanejar) · rejeitar |
| **Gasto** | custo estimado acima do limite (padrão: 1.000.000 micro-unidades) — **sempre** exige aprovação | aprovar o gasto · pular |
| **Mídia de fonte externa** | licença **desconhecida** (restrita é rejeitada por padrão) | usar · pular este asset |
| **Geração por IA** | sempre que a Run quer gerar mídia (padrão seguro) | gerar · não gerar |
| **Conflito** | você alterou a timeline em uso | replanejar · parar |
| **Extensão de orçamento / de ciclos** | limite atingido | estender · parar |
| **Aprovação final** | opcional (desligada por padrão) | aprovar · parar (mantém o resultado sem concluir) |
| **Remoções grandes** | plano remove mais de 25 clips/tracks | exige aprovação do plano |

## Orçamento de uma Run

Limites (todos opcionais, exceto os ciclos): **custo máximo**, **tokens**, **chamadas ao provedor**, **gerações**, **ciclos de revisão** (padrão 2), **replanejamentos** (padrão 3) e **tempo total**. Antes de cada chamada paga o CapIA **reserva** o valor no livro de orçamento e depois **liquida**; ao atingir um limite a Run **para e pergunta** (estender ou parar), nunca estoura em silêncio.

- “Custo” aparece como `Custo: …`; chamadas de preço desconhecido aparecem como “N chamada(s) com preço desconhecido” — desconhecido **não** conta como zero.
- Também existe o teto por tarefa em **Configurar IA → Custo máximo por tarefa**.
- Geração de mídia **sempre** pede aprovação, mesmo com orçamento sobrando.

## Via API

`runs.approve` (scope `run:approve`, separado de `run:start`) aceita `decision_id` + `option`. Recomendação: o sistema que **inicia** Runs não deve ter `run:approve`; deixe gasto, geração e licença com um humano. Veja [`docs/api/canonical-flow.md`](../api/canonical-flow.md).
