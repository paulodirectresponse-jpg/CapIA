# ADR drafts — Trilha D-2 (endurecimento de segurança do servidor REST)

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
`details`. (5) A `Idempotency-Key` do cliente é gravada só como digest (`ik_<sha256[..40]>`) no banco
e na auditoria.

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
`pnpm check:arch` e, opcionalmente, `tools/mutation-phase6.py` (36 mutações; cada uma deve ser
detectada). Mutante sobrevivente é achado e exige teste novo — nunca enfraquecer asserção.
