# Pacote de aceitação da Fase 4 (Inteligência)

Tudo aqui roda **sem credenciais** (provider Replay, FFmpeg local) e produz JSON legível por máquina em
`target/phase4-acceptance/`. O que depende de uma chave real ou de anotação humana está marcado como
**externo** e nunca é preenchido com número inventado: sem execução, o campo fica `null`.

| Pacote | Comando (1 clique) | O que prova | Externo? |
|---|---|---|---|
| Segurança | `node tools/phase4-acceptance/security/run.mjs` | canário (logs/projeto/IPC/AppDb/diagnóstico/crash), redirect, TLS, SSRF, headers, JSON malformado, corpo gigante, stream infinito, tool inexistente/args inválidos, injeção (transcrição/PDF/DOCX), tool entre projetos, escalada de permissão, tool tardia após cancelar, `operation_id` repetido, UI sem segredo | não |
| Cenas (Reference Analyzer) | `node tools/phase4-acceptance/reference-analyzer/run.mjs` | corpus anotado por construção (ffmpeg/lavfi): erro `(FP+FN)/N ≤ 5 %` + conjunto *held-out* | corpus real anotado: `--corpus <dir>` |
| DemandSpec (≥ 10 briefings) | `node tools/phase4-acceptance/demand-spec/run.mjs` | integridade do pipeline em 10 briefings TXT/DOCX/PDF (modo Replay) | **qualidade de LLM**: `--live` com chave real |
| Troca de Brain | `cargo test -p capia-intelligence --test service` | OpenAI-compatível ↔ Anthropic ↔ Google só por configuração | smoke com chaves reais: `--live` abaixo |

## Smoke opcional com chave real (não é gate de CI)

```
CAPIA_ACCEPT_API_KEY=… node tools/phase4-acceptance/demand-spec/run.mjs --live \
  --provider anthropic --model <id> --briefs <pasta> --expected <expected.json>
```
A chave é lida do ambiente, enviada **uma vez** ao serviço (cofre em memória do devserver) e nunca é
escrita no relatório. O relatório registra provider, modelo, data e as métricas **medidas**.

## Estado das pendências externas
Ver `docs/STATUS.md` (seção Fase 4): avaliação de qualidade com LLM real, corpus anotado por humanos e
smoke com chaves reais **não foram executados** neste ambiente.
