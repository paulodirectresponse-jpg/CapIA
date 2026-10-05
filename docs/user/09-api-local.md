# 9. API local (REST, MCP, webhooks)

Para **automatizar** o CapIA a partir de outro programa (um painel interno, um script, um agente de IA via MCP): o `capia-server` expõe a **mesma Engine API** que a interface usa. É **opcional** — o editor não precisa dele — e **vem desligado**: só existe quando você inicia o servidor.

> **Estado (0.6.0-rc.1, Fase 6):** o núcleo REST, tokens/scopes, idempotência e SSE existem no repositório e têm testes; MCP stdio e entrega de webhooks estavam em integração quando esta página foi escrita; a verificação com o servidor real, o pentest independente e a paridade UI×REST×MCP são **externos/pendentes** ([KNOWN_ISSUES](../KNOWN_ISSUES.md)). Referência completa: [`docs/api/`](../api/README.md).

## Iniciar o servidor

```
capia-server serve --data-dir <pasta> [--port N] [--bootstrap]
```

- **Somente loopback (`127.0.0.1`) por padrão.** Sem `--port`, a porta é aleatória: o comando imprime `capia-server listening on http://127.0.0.1:<porta>` e a publica em `server.json` dentro da pasta de dados.
- `--bootstrap`: se **ainda não existir nenhum token**, imprime `CAPIA_BOOTSTRAP_TOKEN=…` (todos os scopes, **mostrado uma única vez**). Troque-o por tokens de escopo mínimo e revogue-o.
- Encerrar com graça: `capia-server stop --data-dir <pasta>` (ou `--stop-on-stdin-eof` quando o servidor é filho de outro processo).
- **Não exponha na rede.** Bind remoto exige opt-in explícito (`--bind IP --allow-remote --remote-tls-terminated-by-proxy`), um proxy reverso com **TLS**, e continua exigindo token; o servidor avisa na inicialização. CORS é **negado** por padrão (`--cors-origin` libera origens específicas); o cabeçalho `Host` é validado (`--allowed-host` acrescenta nomes).
- Se não houver cofre do SO na plataforma, os segredos de webhook ficam só em memória (recrie/rotacione após reiniciar); o servidor avisa.

## Criar tokens e scopes

Tokens ficam no banco do servidor **somente como hash**; o segredo aparece **uma vez**.

```
capia-server token create --data-dir <pasta> --name painel-leitura --scopes project:read,run:read [--expires-in 86400]
capia-server token list   --data-dir <pasta>
capia-server token revoke --data-dir <pasta> --id <token_id>
```

Pela API, quem tem `admin:tokens` usa `POST /v1/tokens`, `…/rotate` (novo segredo, o antigo é revogado) e `DELETE /v1/tokens/{id}`. Dê **o menor conjunto** de scopes:

| Scope | Permite |
|---|---|
| `project:read` / `project:write` | ler projeto, sequences, timeline, histórico, métricas / criar-abrir projetos e **editar** (preview → aplicar) |
| `media:read` / `media:write` | listar assets e uploads / enviar e importar mídia |
| `run:read` / `run:start` / `run:approve` | ver Runs / **iniciar, pausar, retomar, cancelar**, variantes / **aprovar decisões** (gasto, geração, licença) |
| `export:read` / `export:start` | ver exports e entregáveis / iniciar e cancelar exports |
| `webhook:manage` | registrar e gerir webhooks |
| `admin:tokens` | gerir tokens e ler a auditoria — **não dê a agentes de IA** |

Regras práticas: o sistema que **inicia** Runs não precisa de `run:approve`; um painel só de leitura precisa de `project:read` + `run:read`. Guarde tokens em variável de ambiente ou cofre — **nunca** em Git. Se vazar: **revogue primeiro**. Desfazer uma Run e aprovar memória de IA **não existem** na API: só pela interface.

## Primeiro pedido

```
curl http://127.0.0.1:<porta>/v1/health                              # público
curl -H "Authorization: Bearer $CAPIA_TOKEN" http://127.0.0.1:<porta>/v1/server
```

Operações que demoram devolvem **202** + um id (acompanhe por status, por `GET /v1/events/stream` (SSE) ou por webhook). Escritas aceitam `Idempotency-Key` (repita com a **mesma** chave após um timeout). O fluxo completo (briefing + bruto + referência → export → webhook) com curl, PowerShell, Node e Python está em [`docs/api/canonical-flow.md`](../api/canonical-flow.md) e em [`examples/`](../../examples/).

## MCP

Para agentes: `POST /mcp` no servidor HTTP, ou `capia-server mcp-stdio --data-dir <pasta> --token-env CAPIA_TOKEN` (stdio — **interface prevista**, ver [mcp.md](../api/mcp.md)). Mesmos scopes e mesma validação da REST; sem shell, sem acesso livre a arquivos, sem valores de segredos.

## Webhooks

Registre uma URL (`https`, ou `http` **só em loopback**); o servidor assina cada entrega com HMAC-SHA256 e você **verifica** a assinatura e a janela de 5 minutos. Receptor de exemplo testado: [`examples/webhook-receiver/`](../../examples/webhook-receiver/). O segredo é mostrado uma vez; perdeu, rotacione. Detalhes: [webhooks.md](../api/webhooks.md).

## Segurança e privacidade

Tudo é local por padrão; cada chamada externa é **auditada** (quem, o quê, resultado); a API não aceita caminhos de arquivo do cliente (mídia entra por upload + importação); mensagens de erro nunca carregam segredos. Veja [08](08-privacidade-e-seguranca.md) e [`docs/api/security.md`](../api/security.md).
