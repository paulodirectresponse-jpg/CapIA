# CapIA — FASE 4 / INTELIGÊNCIA — ESPECIFICAÇÃO DE CONCLUSÃO

**Objetivo:** concluir toda a engenharia da Fase 4 em uma única missão autônoma, sem iniciar a autonomia da Fase 5.

**Base atual:** `claude/phase3-editor`  
**HEAD atual:** `a76937f58967c6d270eba1bd87f4fcbdf26af30e`  
**Commit de código validado da Fase 3:** `72a0600c7e23de1020feea13840314c44e634af1`  
**Novo branch após closeout:** `claude/phase4-intelligence`

---

# 0. ETAPA 0 — CLOSEOUT TÉCNICO DA FASE 3

Antes de começar a implementação efetiva da Fase 4, corrija o estado vermelho do HEAD atual.

O run 73 falhou em:

1. Windows: `Instalar FFmpeg`;
2. Linux E2E: `17 fluxos` falharam e o teste de desempenho foi pulado.

Regras:
- não reduzir cobertura;
- não mascarar falha de infraestrutura;
- não remover testes;
- se a instalação do FFmpeg depender de fonte externa instável, tornar o CI reprodutível/cacheável;
- se o E2E Linux for flaky, eliminar a causa e provar estabilidade;
- repetir até um mesmo commit ficar verde em todos os jobs relevantes.

Quando o closeout estiver verde:
- atualizar STATUS/ROADMAP;
- registrar `PHASE 3 ENGINEERING COMPLETE — HUMAN ACCEPTANCE PENDING`;
- preservar como pendências externas: 3 usuários reais, residual P2 em GPU real, decisão jurídico/comercial de H.264;
- criar `claude/phase4-intelligence`;
- continuar imediatamente, sem aguardar o usuário.

---

# 1. FRONTEIRA DA FASE 4

A Fase 4 é **IA assistida, pontual e controlada**.

Ela deve permitir:
- configurar providers/modelos;
- escolher um Brain;
- detectar capacidades;
- transcrever;
- gerar legendas automáticas;
- analisar mídia;
- analisar vídeos de referência;
- interpretar briefings/copys/documentos;
- conversar com um assistente;
- executar pedidos pontuais via tools e transações;
- exibir custo, chamadas e resultados;
- funcionar sem IA quando providers estiverem desligados.

Ela NÃO deve implementar a autonomia completa da Fase 5.

Fora de escopo:
- state machine autônoma `UNDERSTAND→PLAN→...→DONE` completa;
- Producer/Planner/Editor/Critic loop autônomo;
- Runs persistentes/retomáveis multi-stage da Fase 5;
- geração autônoma de múltiplas variações;
- memória auto-promovida;
- Asset Gateway completo;
- geração autônoma de imagem/vídeo;
- execução automática de um briefing inteiro;
- REST/MCP público da Fase 6.

---

# 2. PRINCÍPIOS INEGOCIÁVEIS

1. IA é cliente da Engine API.
2. IA nunca escreve no documento diretamente.
3. Toda mutação de timeline passa pelo Command Engine.
4. `Actor::Agent` só escreve via `preview → apply_plan`.
5. Todo comando de IA tem `operation_id`.
6. Sem shell tool.
7. Sem filesystem arbitrário.
8. Sem HTTP genérico exposto ao modelo.
9. Segredos ficam fora da WebView, `.capia`, logs e prompts persistidos.
10. Editor manual continua 100% funcional com IA desligada.
11. Falha de provider nunca deixa transação parcialmente aplicada.
12. Resultados de IA são tratados como entrada não confiável e validados por schema.
13. Prompt injection proveniente de mídia/documento nunca concede novas permissões.
14. Custos e ações de rede devem ser observáveis.

---

# 3. ARQUITETURA ESPERADA

Separar claramente:

- secrets;
- provider abstraction;
- model registry;
- brain profile;
- capability router;
- canonical request/response types;
- tool registry/executor;
- intelligence jobs;
- analyses/transcripts/reference grammar;
- assistant/chat task executor;
- UI/configuration.

Sugestões de crates, adaptáveis à arquitetura existente:

- `capia-secrets`
- `capia-ai`
- `capia-intelligence`
- extensões em `capia-store`/app DB
- adaptações em `capia-editor-api`
- UI sob `editor-ui`

Não criar acoplamento direto UI → provider.

---

# 4. PERSISTÊNCIA DE CONFIGURAÇÃO

Configuração global de IA deve ficar fora do `.capia` quando for configuração do usuário.

Persistir de forma apropriada:
- ProviderConfig;
- ModelEndpoint;
- BrainProfile;
- pricing metadata;
- capability probe;
- UI preferences;
- budgets.

No `.capia`, persistir apenas o que pertence ao projeto:
- DemandSpec;
- transcript/analysis refs;
- ReferenceGrammar refs;
- AI transaction/history metadata quando aplicável;
- assistant task records se necessários para rastreabilidade.

Segredos nunca entram no projeto.

---

# 5. `capia-secrets`

Implementar armazenamento seguro no Windows via Credential Manager ou camada equivalente aprovada pela documentação.

API deve trabalhar com:
- `CredentialRef`
- `SecretHandle`

Regras:
- Rust core recebe secret handle;
- WebView recebe no máximo estado `configured/not configured`;
- nunca retornar valor secreto ao frontend depois de salvo;
- nunca serializar em Debug;
- limpar buffers temporários onde razoável;
- suportar create/update/delete/test;
- binding opcional ao host/base URL;
- rotação de credencial.

Testes:
- chave canário não aparece em `.capia`;
- não aparece em app DB;
- não aparece nos eventos IPC;
- não aparece em logs;
- não aparece em error strings;
- não aparece em crash-test artifacts;
- não aparece em snapshot de UI/E2E.

---

# 6. MODEL REGISTRY

Implementar Model Registry persistente.

ModelEndpoint deve conter pelo menos:
- provider;
- model id;
- display name;
- enabled;
- context window;
- max output;
- capabilities;
- source das capabilities;
- params padrão;
- pricing;
- last probe;
- health/probe status.

Suportar edição manual e atualização via provider quando houver `list_models`.

Nunca depender de uma lista hardcoded como verdade única.

---

# 7. BRAIN PROFILE

Implementar Brain Profile.

Campos:
- brain model;
- role overrides;
- capability overrides;
- fallbacks;
- budgets;
- privacy rules.

Requisitos:
- profile ativo global;
- projeto pode fixar profile se arquitetura permitir;
- impedir Brain incompatível;
- UI explica por que um modelo não pode ser Brain.

Requisito mínimo:
- text generation;
- tool calling;
- structured output confiável ou emulação validada;
- janela de contexto conforme especificação atual.

---

# 8. CAPABILITY ROUTER

Implementar seleção por capability.

Capabilities iniciais:
- text;
- tools;
- structured output;
- streaming;
- vision;
- STT;
- audio input;
- PDF/document;
- embeddings se necessário;
- image/video generation apenas como capability conhecida, sem construir autonomia de geração.

Router considera:
- Brain Profile;
- overrides;
- availability;
- probe status;
- privacy;
- fallback order;
- budget.

Retornar decisão explicável:
- chosen endpoint;
- reason;
- fallbacks considered;
- capability requirements.

---

# 9. TOOL SYSTEM

Criar Tool Registry versionado.

Cada tool possui:
- name;
- version;
- description;
- input schema;
- output schema;
- permission;
- side effect;
- timeout;
- idempotency;
- cost hint quando aplicável.

Permissões mínimas:
- ReadProject
- ReadMedia
- WriteTimeline
- ManageAssets
- NetworkFetch
- SpendMoney
- RunHeavyCompute

Tools da Fase 4:
- project.read_brief
- project.list_sequences
- timeline.get_state
- timeline.query_clips
- timeline.preview
- timeline.apply_plan
- assets.search
- assets.get
- media.analyze
- media.sample_frames
- media.transcribe
- media.detect_scenes
- reference.analyze
- reference.get_grammar
- render.frame

Pode adicionar tools necessárias, mas não shell/filesystem/http genérico.

---

# 10. GATE DE ESCRITA DA IA

Pedidos pontuais do assistente podem editar timeline.

Fluxo obrigatório:
1. interpretar pedido;
2. montar lista de comandos;
3. `preview`;
4. validar diff;
5. aplicar por `apply_plan`;
6. registrar tool calls e transaction metadata.

A Fase 4 não exige o grande EditPlan da Fase 5 para toda tarefa, mas nunca permite escrita direta.

Exemplo:
“adicione legendas”
→ transcript
→ comandos de caption
→ preview
→ apply.

“corte os silêncios”
→ análise
→ comandos
→ preview
→ apply.

---

# 11. CHAT / ASSISTENTE

Implementar painel de assistente.

Suportar:
- conversa por projeto;
- streaming;
- mensagens de tool progress;
- resposta final;
- cancelamento;
- retry;
- erros;
- anexar/selecionar assets e sequences;
- scope explícito da sequence.

Pedidos pontuais iniciais:
- explicar o projeto;
- listar problemas;
- criar legendas;
- cortar silêncios;
- organizar clips;
- alterar texto;
- mover/trim/split por linguagem natural;
- analisar mídia/referência.

Não fazer execução autônoma longa multi-stage.

---

# 12. ESTRUTURA DE MENSAGEM CANÔNICA

Criar formato interno independente de provider:
- system;
- user;
- assistant;
- tool;
- multipart content;
- text;
- image/frame;
- audio/document ref;
- tool call;
- tool result.

Adapters convertem para APIs específicas.

Nenhum adapter deve contaminar a camada superior com formato proprietário.

---

# 13. STREAMING

Abstração unificada para:
- text delta;
- reasoning/status quando permitido;
- tool call delta;
- usage final;
- finish reason;
- error.

UI deve poder cancelar.

Cancelamento precisa abortar rede e impedir aplicação posterior de ferramenta obsoleta.

---

# 14. RETRIES / FALLBACKS

Classificar ProviderError:
- auth;
- rate limit;
- timeout;
- transient server;
- invalid request;
- unsupported capability;
- safety/refusal;
- malformed output.

Retry apenas para transitórios.

Fallback conforme Brain Profile.

Não retry infinito.

Registrar:
- tentativas;
- latência;
- endpoint;
- motivo do fallback.

---

# 15. STRUCTURED OUTPUT

Todo contrato crítico usa schema.

Validação obrigatória para:
- DemandSpec;
- ReferenceGrammar;
- transcript canonical;
- media analysis;
- tool arguments;
- tool results.

Se provider não oferece JSON Schema nativo:
- emular;
- parse;
- validar;
- retry de correção limitado.

Nunca aceitar JSON parcialmente válido silenciosamente.

---

# 16. CUSTO E USAGE

Registrar por chamada:
- provider;
- model;
- role/task;
- tokens input/output/cache;
- duração;
- custo estimado;
- moeda;
- pricing version/date;
- status.

UI mostra:
- custo da tarefa;
- custo da sessão;
- custo do projeto quando disponível.

Se pricing desconhecido:
- marcar desconhecido;
- nunca inventar custo.

---

# 17. CACHE DE RESULTADOS

Resultados determinísticos podem ser cacheados por:
- content hash;
- model endpoint;
- capability;
- prompt/schema version;
- params;
- algorithm version.

Aplicável a:
- transcript;
- sampled-frame descriptions;
- media analyses;
- ReferenceGrammar.

Cache deve ser invalidado por mudança de conteúdo/versão.

---

# 18. OBSERVABILIDADE

Logs estruturados com redaction.

Registrar:
- request id;
- task id;
- provider/model;
- tool call name;
- latency;
- result status;
- cost.

Não registrar:
- API key;
- Authorization;
- cookies;
- secret headers;
- áudio/frame bruto sem razão explícita;
- prompt contendo material sensível em log geral.

---

# 19. UI DE CONFIGURAÇÃO

Criar área de AI Settings.

Telas:
- Providers;
- Models;
- Brain Profiles;
- Capability routing;
- budgets/privacy;
- probe/test connection.

UX:
- adicionar provider;
- salvar credencial;
- testar;
- listar modelos;
- editar capabilities;
- selecionar Brain;
- configurar fallback;
- indicar provider offline/inválido.

---

# 20. PROVIDERS DESLIGADOS

Existe modo real `AI Off`.

Nesse modo:
- editor abre;
- projeto abre;
- mídia importa;
- timeline funciona;
- preview funciona;
- export funciona;
- nenhum request de rede de IA ocorre.

Criar E2E que prova isso.

---

# 21. MIGRATIONS

Qualquer novo schema:
- migration explícita;
- rollback/atomicidade;
- future schema rejection;
- corruption tests;
- nenhum segredo persistido.

Se criar app DB global, documentar ownership e migrations separadas do `.capia`.

---

# 22. FASE 4 — STATUS FINAL

Estados permitidos:

### `PHASE 4 COMPLETE`
Somente se todos os critérios objetivos e avaliações humanas previstas no ROADMAP forem realmente concluídos.

### `PHASE 4 ENGINEERING COMPLETE — EXTERNAL ACCEPTANCE PENDING`
Quando:
- implementação pronta;
- CI verde;
- replay e testes automatizados verdes;
- restam credenciais reais/avaliação manual/10 briefs externos.

Não fabricar resultados.

---

# 23. DEFINITION OF DONE GLOBAL

Antes de encerrar:
- Fase 3 HEAD verde;
- nova branch Fase 4;
- secrets seguro;
- ≥3 provider families integradas;
- provider local/replay;
- registry;
- profiles;
- routing;
- tools;
- transcript;
- captions;
- media analysis;
- Reference Analyzer;
- Demand Interpreter;
- chat pontual;
- cost/usage;
- security canary;
- AI-Off E2E;
- Windows CI;
- Linux CI;
- TypeScript;
- policy/security;
- docs atualizados;
- working tree limpa;
- branch pushed.

Detalhes técnicos e acceptance estão nos documentos companheiros.
