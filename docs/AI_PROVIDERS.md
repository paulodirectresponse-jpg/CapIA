# AI PROVIDERS — Provider abstraction, Model Registry, Brain Profile, Capability Router

## 1. Objetivo

O app não é preso a nenhum fornecedor. O usuário cadastra providers e modelos, escolhe o **PRIMARY AI BRAIN** e, opcionalmente, overrides por capability.

```
AI Orchestrator
      ↓
Capability Router  ←  Brain Profile (ativo)
      ↓
Provider Abstraction (trait + formato canônico)
      ↓
┌──────────────────┬───────────────┬───────────────┬──────────────────┬────────────────┐
│ OpenAI-compatible│ Anthropic     │ Google Gemini │ Local runtimes   │ Custom HTTP    │
│ (OpenAI, Azure*, │ (Messages API)│               │ (Ollama, LM      │ (adapter       │
│ OpenRouter, Groq,│               │               │ Studio, whisper. │ declarativo)   │
│ vLLM, etc.)      │               │               │ cpp, ONNX)       │                │
└──────────────────┴───────────────┴───────────────┴──────────────────┴────────────────┘
```
Todo código de provider vive em `capia-ai` (Rust). A WebView só vê configuração não secreta.

## 2. Entidades (app.db)

```rust
struct ProviderConfig {
  id, kind: ProviderKind /* OpenAiCompatible | Anthropic | Google | Ollama | WhisperLocal | CustomHttp | ... */,
  display_name, base_url: Option<Url>, credential_ref: Option<CredentialRef>,   // ponteiro p/ Credential Manager
  bound_host: Option<String>,        // host ao qual a credencial está vinculada (SECURITY.md §3)
  extra_headers: Map<String,String>, // não secretos
  timeout_s, max_concurrency, rate_limit: Option<RateLimit>, enabled, created_at,
}

struct ModelEndpoint {
  id, provider_id, model_id: String /* id do provider */, display_name,
  capabilities: Capabilities, context_window: u32, max_output_tokens: u32,
  pricing: Option<Pricing> /* por 1M tokens in/out/cached, por imagem, por segundo de vídeo/áudio; moeda; fonte; data */,
  default_params: Json /* temperature, top_p, reasoning effort, etc. */,
  capability_source: Declared|Probed|Preset, last_probe: Option<ProbeResult>, enabled,
}

struct Capabilities {
  text_generation, tool_calling, parallel_tool_calls, structured_output /* json schema */, streaming,
  vision_input, audio_input, video_input, pdf_input,
  image_generation, video_generation, speech_to_text, text_to_speech, embeddings,
  reasoning_controls,
}

struct BrainProfile {
  id, name, brain: ModelEndpointId,                         // PRIMARY AI BRAIN
  role_overrides: Map<AgentRole, ModelEndpointId>,          // opcional (ex.: Critic com outro modelo)
  capability_overrides: Map<Capability, CapabilityPolicy>,  // Vision: Auto | Model(X); Transcription: Auto | Provider(C)...
  fallbacks: Map<Capability, Vec<ModelEndpointId>>,
  budgets: Budgets /* por Run, por projeto/mês; limite de aprovação automática */,
  privacy: PrivacyPolicy /* ex.: "nunca enviar vídeo", "transcrição só local" */,
}
```
Um Brain Profile é marcado como **ativo** nas configurações; projetos podem fixar um profile.

## 3. Requisitos mínimos para ser Brain

O Brain orquestra via tools; portanto exige `text_generation + tool_calling + structured_output` (ou emulação confiável) e `context_window ≥ 64k`. O app **impede** selecionar como Brain um modelo que não atenda (mostra o motivo). Vision no Brain é desejável mas não obrigatória (o router delega).

## 4. Interface de provider (formato canônico)

```rust
#[async_trait]
trait ModelProvider {
  fn kind(&self) -> ProviderKind;
  async fn chat(&self, req: ChatRequest, cred: &SecretHandle) -> Result<ChatStream, ProviderError>;
  async fn transcribe(&self, req: SttRequest, cred) -> Result<Transcript, ProviderError>;     // opcional
  async fn generate_image(&self, req: ImageGenRequest, cred) -> Result<GenJob, ProviderError>; // opcional
  async fn generate_video(&self, req: VideoGenRequest, cred) -> Result<GenJob, ProviderError>; // opcional (normalmente assíncrono/polling)
  async fn embed(&self, req: EmbedRequest, cred) -> Result<Embeddings, ProviderError>;         // opcional
  async fn list_models(&self, cred) -> Result<Vec<RemoteModelInfo>, ProviderError>;           // opcional
  async fn probe(&self, model: &str, cred) -> ProbeResult;  // teste de conexão + tool call + json + imagem
}
```

`ChatRequest` canônico: mensagens (system/user/assistant/tool) com partes (texto, imagem, áudio, documento), tools (JSON Schema), `tool_choice`, `response_schema`, parâmetros, metadata (`run_id`, papel). Cada adapter traduz para o formato nativo e normaliza a resposta: texto, tool calls, uso de tokens, motivo de parada, erros.

`ProviderError` classificado: `Auth`, `RateLimited{retry_after}`, `Overloaded`, `Timeout`, `ContextTooLong`, `ContentFiltered`, `InvalidRequest`, `Unavailable`, `Unknown` — base para retry/fallback.

## 5. Capability Router

```
resolve(capability, ctx{role, stage, input_size, privacy, budget}) → ModelEndpoint | Error
```
Algoritmo:
1. Override explícito no Brain Profile (para capability ou papel)? Usa.
2. `Auto`: o próprio Brain tem a capability? Usa o Brain (mantém coerência).
3. Senão, primeiro modelo habilitado que tenha a capability e respeite `privacy` e `budget` (ordem definida pelo usuário; desempate por custo).
4. Política de privacidade pode forçar local (ex.: transcrição com whisper.cpp).
5. Indisponível → fallbacks → erro `NoCapableModel` (a feature fica desabilitada; o editor continua).

O **Brain continua a autoridade**: modelos auxiliares são chamados *como tools* (ex.: `media.analyze` usa Vision internamente) e devolvem dados ao Brain; eles não tomam decisões de edição.

## 6. Cadastro e UX de configuração

- Presets de providers conhecidos (base URL, formato) e de modelos populares (capabilities/preços como **sugestão editável**, com data da informação).
- **Probe** ("Testar"): valida credencial, mede latência, testa tool calling, JSON estruturado e entrada de imagem; marca `capability_source=Probed`. Capabilities declaradas manualmente mas não verificadas aparecem como "não verificadas".
- `list_models` quando o provider suporta, para escolher `model_id` sem digitar.
- Custos desconhecidos: permitidos; o orçamento então conta apenas tokens/chamadas e avisa "custo desconhecido".

## 7. Execução e resiliência

- Concorrência e rate limit por provider (token bucket); fila do pool `ai` do Job System para chamadas longas.
- Retry: só `RateLimited/Overloaded/Timeout/Unavailable`, backoff exponencial com jitter, respeitando `retry_after`; máx. 3.
- Fallback: após esgotar retries, próximo modelo da lista de fallback, se compatível com o contexto (tamanho/capabilities).
- `ContextTooLong`: o orquestrador compacta contexto (digest menos detalhado, sumarização) e tenta de novo.
- Timeouts por tipo de chamada; cancelamento propagado do AI Run.
- Streaming quando suportado; a UI mostra progresso.

## 8. Transcrição, visão e geração (capabilities auxiliares)

| Capability | Opções | Notas |
|---|---|---|
| Speech-to-text | Local (whisper.cpp/ONNX) ou cloud | Exige timestamps por palavra para legendas; envia **só áudio comprimido** |
| Vision | Brain (se multimodal) ou modelo dedicado | Frames amostrados e reduzidos |
| Image generation | Providers de imagem | Proveniência obrigatória (`ASSET_SYSTEM.md` §7) |
| Video generation | Providers de vídeo (assíncronos) | Job com polling; custo alto → aprovação |
| TTS | Opcional | Voz gerada é asset com proveniência |
| Embeddings | Opcional (busca semântica em biblioteca/memória) | Pode ser local |

## 9. Registro de uso

`ai_usage(run_id, step_id, model_endpoint_id, provider_kind, tokens_in/out/cached, units (imagens/segundos), cost_estimated, latency_ms, status, timestamp)` — sem conteúdo de credenciais. Prompts/respostas ficam no `.capia` (para auditoria/replay), com opção do usuário de não reter.

---
**Estado de implementação (Fase 4):** OpenAI-compatível, Anthropic, Google, Replay e whisper.cpp implementados com suíte de contrato comum; probe real; Router com privacidade/orçamento; fallback só configurado (ADR-079/080).
