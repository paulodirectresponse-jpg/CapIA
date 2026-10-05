# CapIA — FASE 6 / INTEGRAÇÃO E FINALIZAÇÃO — ESPECIFICAÇÃO DE CONCLUSÃO

**Objetivo:** concluir a última fase do roadmap atual: tornar o CapIA integrável externamente, instalável, atualizável, observável e pronto para beta/release.

**Base:** `claude/phase5-autonomy`
**Target:** `claude/phase6-finalization`

---

# 1. Missão

Entregar um produto distribuível mantendo a mesma Engine API e todas as garantias das Fases 1–5.

Fluxo-alvo principal:

`cliente externo → REST/MCP → cria projeto/run → envia briefing + raw + referência → acompanha progresso → aprova quando exigido → recebe variantes/export → webhook de conclusão`

O mesmo fluxo, executado pela UI, REST ou MCP, deve chegar ao mesmo estado autoritativo do projeto.

---

# 2. Fronteira

A Fase 6 IMPLEMENTA:
- `capia-server`;
- REST API;
- MCP Server;
- Webhooks;
- autenticação/scopes;
- installer/bundle;
- update;
- crash reporting opt-in;
- hardening de performance;
- docs de usuário/API;
- beta/release tooling.

A Fase 6 NÃO deve:
- reescrever o engine;
- introduzir uma segunda lógica de edição;
- bypassar Command Engine;
- criar shell/filesystem arbitrário;
- relaxar segurança para facilitar integração.

---

# 3. Princípios inegociáveis

1. UI, REST e MCP são clientes da mesma Engine API.
2. `Actor::Api` e `Actor::Agent` só escrevem via `preview → apply_plan`.
3. Mesmos operation_ids/idempotência/locks/revisions.
4. Mesmos scopes/permissões em todas superfícies.
5. Servidor local por padrão em loopback.
6. Nenhum segredo na WebView/API responses/logs/crash reports.
7. Webhooks assinados e idempotentes.
8. Installer/update nunca instalam binário não assinado quando política exigir assinatura.
9. Update tem rollback seguro.
10. Migrações de projeto seguem regras forward-only existentes.
11. Produto manual e AI-Off continuam funcionando.
12. Qualquer dependência externa de release deve ser explicitamente marcada como externa.

---

# 4. Capia Server

Criar `capia-server` como host headless da Engine API.

Responsabilidades:
- lifecycle de projetos;
- REST;
- MCP;
- webhook dispatcher;
- auth;
- event stream;
- health/metrics locais;
- shutdown gracioso.

Não duplicar lógica do desktop.

---

# 5. API versioning

Todas as rotas públicas versionadas, ex. `/v1/...`.

Definir política:
- additive changes;
- deprecation;
- breaking changes;
- content type;
- error envelope;
- request id.

---

# 6. API resources

Recursos mínimos:
- server/health;
- auth/tokens;
- projects;
- assets;
- sequences;
- timeline/query;
- commands/preview/apply;
- ai/runs;
- ai/approvals;
- memory proposals;
- gateway/generation status;
- exports/deliverables;
- webhooks.

---

# 7. Long-running operations

Usar jobs/Run ids.

Nunca bloquear request HTTP durante edição longa.

Pattern:
`POST -> 202 + operation/run id`
`GET status`
`SSE/WebSocket/event polling opcional`
`webhook final`.

---

# 8. Uploads

Suportar upload seguro:
- streaming;
- size limits;
- temp staging;
- hash;
- content sniff;
- import pelo asset system;
- cancel;
- resumable/chunked se necessário.

Nunca confiar em filename/MIME do cliente.

---

# 9. External automation canonical flow

Implementar E2E:
1. create/open project;
2. upload raw video;
3. upload/reference URL approved;
4. submit copy/brief;
5. start AI Run;
6. poll/subscribe status;
7. handle approval;
8. wait completion;
9. list variants;
10. export;
11. receive webhook;
12. verify same project state via UI.

---

# 10. Paridade

Criar conformance suite que execute a mesma tarefa por:
- direct Engine API;
- REST;
- MCP;
- UI/test controller quando viável.

Assert:
- same command effects;
- same sequence graph;
- same Run stage output;
- same export metadata;
- same security gates.

---

# 11. Authentication

Local token auth.

Token:
- id;
- secret hash;
- scopes;
- created;
- expires optional;
- last used;
- revoked.

Secret exibido uma única vez.

---

# 12. Scopes

Scopes mínimos:
- project:read
- project:write
- media:read
- media:write
- run:read
- run:start
- run:approve
- export:read
- export:start
- webhook:manage
- admin:tokens

Scopes least-privilege.

---

# 13. Server binding

Default:
- `127.0.0.1`
- random/configured port.

Binding público requer configuração explícita e aviso.

CORS fechado por default.

---

# 14. Rate limits

Por token/rota:
- read;
- writes;
- uploads;
- run starts;
- approvals;
- export.

429 estruturado.

---

# 15. Webhooks

Eventos:
- run.started
- run.waiting_user
- run.completed
- run.failed
- export.completed
- export.failed

Payload versionado.

---

# 16. Webhook security

Assinatura HMAC:
- timestamp;
- event id;
- body digest/signature.

Replay window.
Idempotency event id.

---

# 17. Webhook retries

Exponential backoff.
Dead-letter state.
UI/API para reentregar.
Nunca bloquear Run completion por endpoint externo indisponível.

---

# 18. MCP Server

Implementar sobre as mesmas facades da REST.

Tools/resources devem ser explícitos e tipados.

Não expor:
- shell;
- arbitrary HTTP;
- arbitrary filesystem;
- secret values.

---

# 19. MCP operations

Mínimo:
- project create/open/list;
- assets import/list;
- sequences list;
- run create/status/approve/cancel;
- plan/review read;
- export start/status;
- project summary;
- timeline read/query;
- controlled edit preview/apply where policy allows.

---

# 20. Installer

Produzir bundle Windows com:
- Tauri app;
- required sidecars;
- FFmpeg approved build/runtime;
- WebView2 handling;
- migrations;
- shortcuts/uninstaller.

---

# 21. Clean install behavior

Testar:
- first launch;
- no config;
- no providers;
- AI Off;
- create/import/edit/export;
- enable provider later;
- uninstall/reinstall preserving user projects.

---

# 22. Signing

Installer signing pipeline preparado.

If certificate exists:
- sign artifact;
- verify signature.

If unavailable:
- external release pending;
- no fake pass.

---

# 23. Auto-update

Signed manifest/update artifacts.

Requirements:
- version compare;
- download;
- signature verify;
- staged update;
- rollback/recovery;
- no update during critical write without policy.

---

# 24. Crash reporting

Opt-in.

Collect only:
- version;
- OS;
- stack/crash metadata;
- redacted diagnostics;
- no media/project content by default;
- no secrets.

User can disable.

---

# 25. Diagnostics bundle

One-click support bundle:
- app version;
- system info;
- redacted logs;
- CI/build id;
- recent structured errors;
- no raw keys/media.

---

# 26. Performance hardening

Target project:
- ≥30 sequences;
- ≥5,000 clips;
- nested;
- media catalog large;
- AI Run history.

Measure:
- open;
- save;
- timeline;
- preview;
- REST latency;
- MCP latency;
- Run state updates;
- export queue.

---

# 27. Reliability

Soak tests:
- 8h server idle/active;
- repeated open/close projects;
- repeated external Runs;
- webhook failures;
- update interruption;
- installer repair.

---

# 28. Security hardening

Pentest básico:
- auth bypass;
- scope escalation;
- path traversal;
- SSRF;
- upload bombs;
- oversized JSON;
- webhook spoof/replay;
- token leak;
- CORS;
- host header;
- local privilege assumptions.

---

# 29. Documentation

User docs:
- install;
- first project;
- providers;
- manual editing;
- autonomous run;
- approvals;
- export;
- backup/recovery;
- troubleshooting.

Developer docs:
- REST;
- MCP;
- webhook;
- auth;
- examples.

---

# 30. Beta package

Create release candidate package:
- version;
- changelog;
- installer;
- hashes;
- SBOM/licensing;
- known issues;
- acceptance checklist.

---

# 31. Migration and compatibility

Test opening:
- Phase 3-era project;
- Phase 4 schema;
- Phase 5 schema;
- current schema.

No data loss.

---

# 32. External acceptance boundaries

May remain external:
- code-signing certificate procurement;
- Windows 10/11 physical clean-machine verification if CI images insufficient;
- beta users;
- legal/commercial H.264/AAC decisions;
- real provider acceptance inherited.

Engineering must provide executable harnesses for each.

---

# 33. Final states

`PHASE 6 COMPLETE`
only when release/human/external gates actually pass.

`PHASE 6 ENGINEERING COMPLETE — EXTERNAL RELEASE ACCEPTANCE PENDING`
when all engineering and automated validation are complete but external signing/physical/beta gates remain.

---

# 34. Definition of Done

- capia-server;
- REST;
- MCP;
- webhooks;
- auth/scopes;
- parity suite;
- external automation E2E;
- installer;
- updater;
- crash reporting;
- diagnostics;
- perf;
- security;
- docs;
- beta tooling;
- final CI green same HEAD.

