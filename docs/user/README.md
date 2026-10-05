# CapIA — documentação do usuário

CapIA (nome de trabalho) é um editor de vídeo para desktop, **AI-first**, para Direct Response, UGC, Ads e VSLs. A **timeline é a fonte de verdade**: tudo que a IA produz é clip comum, 100 % editável à mão, com desfazer. **O editor funciona sem IA e sem rede.**

> **Versão desta documentação: 0.6.0-rc.1 (release candidate).** Ela descreve o que existe no repositório hoje. Itens marcados **“Fase 6”** estão em integração ou dependem de verificação externa; o estado exato está em [`docs/KNOWN_ISSUES.md`](../KNOWN_ISSUES.md) e em [`docs/phase6/IMPL_DOCS_ACCEPTANCE.md`](../phase6/IMPL_DOCS_ACCEPTANCE.md).

| # | Guia | Para quê |
|---|---|---|
| 1 | [Primeiros passos](01-primeiros-passos.md) | instalar → primeiro projeto → importar → editar → configurar IA → Run autônoma → exportar |
| 2 | [Manual do editor](02-editor-manual.md) | timeline, inspector, texto/legendas, áudio, preview, histórico, atalhos |
| 3 | [Provedores de IA](03-provedores-de-ia.md) | configurar chaves (cofre do SO), modelos, Brain, privacidade; **IA desligada funciona** |
| 4 | [Autonomia (AI Run)](04-autonomia.md) | como a Run planeja, edita, revisa e corrige; variantes; desfazer |
| 5 | [Aprovações e orçamentos](05-aprovacoes-e-orcamentos.md) | o que pede sua decisão e como limitar custo |
| 6 | [Backup e recuperação](06-backup-e-recuperacao.md) | formato do projeto, pastas laterais, o que é descartável, atualização e rollback |
| 7 | [Solução de problemas](07-solucao-de-problemas.md) | FFmpeg, WebView2, offline/relink, erros de IA, diagnóstico |
| 8 | [Privacidade e segurança](08-privacidade-e-seguranca.md) | o que sai da máquina e o que nunca sai |
| 9 | [API local](09-api-local.md) | REST/MCP/webhooks para automação (loopback, tokens e scopes) |

Documentação para desenvolvedores de integração: [`docs/api/`](../api/README.md). Notas de versão: [`CHANGELOG.md`](../../CHANGELOG.md).

## Convenções

- **Verificado por CI** = há teste automatizado no repositório. **Externo/pendente** = depende de máquina, pessoa, provedor ou certificado reais e **não** foi verificado.
- Atalhos usam `Ctrl`; os nomes dos botões seguem a interface em português.
