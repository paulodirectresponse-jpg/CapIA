# capia-ai

Providers, Model Registry, Brain Profile, Capability Router, dispatcher e Tool System (ADR-079..081). **Único** crate com HTTP de saída; não conhece documento/projeto/timeline.

| Módulo | Papel |
|---|---|
| `types`, `stt`, `capability` | contrato canônico (mensagens, tools, uso, transcrição; capabilities com origem) |
| `providers::{openai,anthropic,google,replay,whisper}` | adapters; **Replay** (digest/roteiro/respondedor) para CI determinístico |
| `http` | cliente endurecido: SSRF, credencial presa ao host, sem credencial em redirect, TLS normal, limites |
| `registry`, `brain`, `router` | configuração, privacidade/orçamento, rota explicada; fallback só configurado |
| `probe` | probe real (texto, streaming, tool, estruturado, visão, STT) |
| `dispatcher` | `AiRuntime`: retry/backoff, fallback, cancelamento real, custo em micro-unidades, cache |
| `tools`, `prompt` | registry versionado + gate; `untrusted_data` e contexto limitado |
| `testkit` (feature) | servidor HTTP falso (testes de contrato/segurança) |

`cargo test -p capia-ai` (80+ testes: contrato × 3 famílias, dispatcher, segurança).
