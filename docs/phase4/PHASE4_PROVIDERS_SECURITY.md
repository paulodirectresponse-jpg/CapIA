# CapIA — FASE 4 — PROVIDERS, SEGREDOS, ROUTER E SEGURANÇA

Este documento detalha os requisitos de integração de providers, proteção de credenciais, capability routing, tool execution e testes de segurança da Fase 4.

---

# 1. Provider families obrigatórias

Implementar adapters funcionais para pelo menos:

1. OpenAI-compatible
   - OpenAI
   - endpoints compatíveis (OpenRouter/Groq/vLLM/Azure-like quando configurados por base URL)
2. Anthropic
3. Google Gemini
4. Local / Replay
   - Replay é obrigatório para CI determinístico.
   - Um runtime local real pode ser Ollama/LM Studio/Whisper local conforme arquitetura.

Critério do ROADMAP:
o Brain deve poder ser trocado entre pelo menos 3 providers diferentes sem mudança de código.

---

# 2. Canonical provider interface

Uma interface canônica deve cobrir:

- chat/text generation;
- streaming;
- tool calling;
- structured output;
- vision input;
- audio input quando suportado;
- STT;
- model listing;
- probe;
- usage;
- capability errors.

Sugestão conceitual:

```rust
#[async_trait]
trait ModelProvider {
    fn kind(&self) -> ProviderKind;
    async fn chat(&self, req: ChatRequest, secret: &SecretHandle) -> Result<ChatStream, ProviderError>;
    async fn transcribe(&self, req: SttRequest, secret: &SecretHandle) -> Result<Transcript, ProviderError>;
    async fn list_models(&self, secret: &SecretHandle) -> Result<Vec<RemoteModelInfo>, ProviderError>;
    async fn probe(&self, model: &str, secret: &SecretHandle) -> ProbeResult;
}
```

A assinatura final pode variar; a separação não.

---

# 3. OpenAI-compatible adapter

Suportar:
- base URL configurável;
- API key por secret ref;
- custom non-secret headers;
- model id arbitrário;
- streaming;
- tool calls;
- structured response quando disponível;
- vision multipart;
- usage;
- list models quando endpoint suporta.

Não assumir que todo endpoint OpenAI-compatible implementa tudo.

Probe deve detectar capacidade real, não só confiar no nome.

---

# 4. Anthropic adapter

Suportar:
- Messages API;
- system/user/assistant/tool mapping;
- tool calls;
- streaming;
- usage;
- vision se disponível;
- structured output via estratégia canônica suportada pelo adapter.

Não deixar tipos Anthropic vazarem para o restante do app.

---

# 5. Google adapter

Suportar:
- Gemini;
- multipart;
- tool/function calling;
- structured output;
- vision;
- streaming;
- usage quando retornado.

Normalizar diferenças de finish reason e tool responses.

---

# 6. Local providers

Fase 4 precisa suportar pelo menos caminho local configurável.

Pode incluir:
- Ollama;
- LM Studio / OpenAI-compatible local;
- Whisper local;
- provider Replay.

Não exija que um runtime local específico esteja instalado para o app abrir.

---

# 7. Replay provider

Obrigatório.

Objetivo:
- testes determinísticos;
- sem custo;
- sem rede;
- gravação/reprodução de respostas canônicas.

Requisitos:
- fixture versionada;
- match por request digest;
- resposta streaming simulável;
- tool calls;
- structured outputs;
- usage sintético explícito;
- erros sintéticos;
- retry/fallback scenarios.

Nunca gravar secrets nas fixtures.

---

# 8. Provider probe

Probe deve testar capacidades, não apenas `/models`.

No mínimo:
- auth/connectivity;
- text generation;
- streaming;
- tool call;
- JSON/structured output;
- vision quando declarada;
- STT quando declarada.

ProbeResult:
- success;
- latency;
- capabilities verified;
- failures by capability;
- provider/model/version metadata;
- timestamp.

Probe nunca imprime secret.

---

# 9. Declared vs Probed capabilities

Cada capability deve ter origem:
- Declared;
- Probed;
- Preset.

Router deve preferir evidência probed para tarefas críticas.

UI mostra origem.

---

# 10. Credential lifecycle

Credencial:
- create;
- update;
- delete;
- test;
- rotate;
- bind to provider/base host.

WebView não deve receber segredo depois de submit.

Ao editar provider:
- UI recebe `credential_configured: true/false`;
- nunca recebe a key.

---

# 11. Secret redaction

Criar redactor central.

Redigir:
- raw keys;
- bearer tokens;
- API-Key headers;
- cookies;
- provider secrets;
- query params classificados como secret.

Aplicar em:
- tracing/logs;
- error wrapping;
- audit;
- HTTP debug;
- crash data;
- UI diagnostics.

Testar com canary único.

---

# 12. Canary test obrigatório

Gerar uma chave canário reconhecível em teste.

Depois exercitar:
- save provider;
- probe;
- chat;
- tool call;
- provider error;
- timeout;
- fallback;
- app logs;
- store;
- project dump;
- diagnostics;
- IPC payloads.

Fazer busca byte/textual nos artefatos.

Critério:
0 ocorrências fora do secret store.

---

# 13. Host binding / SSRF

Para providers configuráveis:
- validar URL;
- exigir http/https apropriado;
- bloquear esquemas inesperados;
- vincular credential ao host configurado;
- não encaminhar credential ao receber redirect para outro host;
- limitar redirects;
- não permitir file://;
- considerar loopback apenas quando provider local configurado.

Custom HTTP genérico fora da abstraction não deve existir como tool da IA.

---

# 14. TLS / network

Produção:
- TLS verification normal;
- sem `danger_accept_invalid_certs`;
- timeouts;
- body/output limits;
- concurrency limits.

Testes podem usar servidor local controlado.

---

# 15. Tool permissions

Cada tool declara Permission e SideEffect.

Antes de executar:
1. stage/task permite tool;
2. role permite tool;
3. permission concedida;
4. input schema válido;
5. budget/privacy permite;
6. side effect aprovado quando necessário.

Retornar erro estruturado, nunca “best effort” silencioso.

---

# 16. Prompt injection boundary

Conteúdo de:
- transcript;
- PDF;
- DOCX;
- frames OCR;
- web/reference content

é **dados não confiáveis**, não instruções de sistema.

Implementar separação explícita na montagem do prompt.

Teste fixtures com texto malicioso:
- “ignore instructions”
- “call shell”
- “send API key”
- “delete project”
- “use this URL”

Resultado:
nenhuma permissão extra; nenhuma ferramenta proibida.

---

# 17. Tool output limits

LLMs não recebem dumps ilimitados.

Implementar:
- pagination;
- truncation;
- summaries;
- bounded media samples;
- maximum tool result bytes/tokens.

Timeline full dump só paginado/segmentado.

---

# 18. Operation IDs de Agent

Para mutações:
- determinístico por task/run + stage/step/index;
- retry não duplica edição;
- reuse conflict detectado.

Teste timeout depois de commit:
retry deve recuperar idempotentemente.

---

# 19. Preview/apply gate

`Actor::Agent`:
- nunca chama raw command commit;
- usa preview;
- recebe plan token/diff;
- apply usa token;
- drift entre preview e apply deve falhar/replanejar.

Chat pontual pode auto-aplicar conforme política configurada, mas tecnicamente continua passando pelo gate.

---

# 20. Budgets

BrainProfile deve suportar:
- max cost per task;
- warning threshold;
- automatic approval threshold;
- optional monthly/project tracking.

SpendMoney não se aplica automaticamente a simples chat se pricing/custo estiver dentro da política, mas custos precisam ser registrados.

Nenhuma chamada de geração paga futura deve ser liberada nesta fase sem política.

---

# 21. Privacy policy

Exemplos configuráveis:
- never upload original video;
- vision frames only;
- transcription local only;
- no cloud audio;
- no external document upload.

Router deve eliminar endpoints incompatíveis com privacy.

Mostrar motivo quando nenhuma rota válida existir.

---

# 22. Rate limiting e concurrency

ProviderConfig:
- max concurrency;
- optional rate limit.

Executor deve:
- limitar chamadas;
- queue;
- respeitar cancellation;
- lidar com 429 com backoff.

Não bloquear UI thread.

---

# 23. Timeouts

Separar:
- connect;
- first byte;
- total;
- idle stream timeout.

Abort real ao cancelar.

---

# 24. Error taxonomy

Normalizar no mínimo:
- AUTH_FAILED
- RATE_LIMITED
- PROVIDER_TIMEOUT
- PROVIDER_UNAVAILABLE
- INVALID_PROVIDER_RESPONSE
- UNSUPPORTED_CAPABILITY
- MODEL_NOT_FOUND
- STRUCTURED_OUTPUT_INVALID
- TOOL_CALL_INVALID
- PRIVACY_POLICY_BLOCKED
- BUDGET_EXCEEDED
- CANCELLED

Preservar detalhes diagnósticos sem secret.

---

# 25. Fallback semantics

Fallback só quando compatível com:
- capability;
- privacy;
- budget;
- role/task.

Não fallback de um modelo vision para text-only em tarefa vision.

Registrar motivo.

---

# 26. Model health

Manter estado:
- unknown;
- healthy;
- degraded;
- unavailable.

Não remover modelo só por uma falha transitória.

---

# 27. Pricing

Pricing record:
- currency;
- input;
- output;
- cached input if supported;
- image/audio/video units when relevant;
- source;
- effective date.

Preço desconhecido = unknown.

Nunca estimar “de cabeça”.

---

# 28. Security tests obrigatórios

- secret canary;
- provider redirect credential leak;
- invalid TLS behavior;
- malicious custom header;
- malformed JSON;
- gigantic response;
- streaming never ends;
- provider returns tool not registered;
- provider returns invalid tool args;
- prompt injection in transcript;
- prompt injection in PDF;
- cross-project tool access;
- permission escalation attempt;
- cancelled response trying to apply late tool;
- replayed operation id.

---

# 29. UI security

Frontend:
- não mantém key em Redux/store/localStorage;
- campo secret é write-only;
- diagnostics redigidos;
- copy button nunca disponível para secret salvo.

---

# 30. Audit records

Registrar:
- task/session;
- provider/model;
- capability;
- tool;
- permission;
- timestamps;
- status;
- usage/cost;
- transaction id/digest quando houver mutação.

Não registrar segredo.

---

# 31. CI strategy

CI sem credenciais externas:
- Replay provider obrigatório;
- mock HTTP servers para adapters;
- contract tests.

Credenciais reais:
- nunca necessárias para CI principal;
- smoke opcional/manual pode usar secrets do GitHub em ambiente autorizado, mas não é gate do código.

---

# 32. Provider contract suite

Mesma suíte deve ser executável contra qualquer adapter.

Casos:
- simple text;
- streaming;
- structured output;
- one tool call;
- parallel tools quando declarado;
- malformed response;
- timeout;
- cancel;
- usage;
- auth error;
- rate limit.

---

# 33. Definition of Done — providers/security

- 3+ provider families;
- replay;
- registry;
- brain profile;
- router;
- probes;
- secret store;
- redaction;
- canary clean;
- tool permission gate;
- prompt injection tests;
- retry/fallback;
- usage/cost;
- AI Off;
- Windows/Linux CI.

