# capia-jobs

Executor genérico de jobs em background (ADR-052). **Não** conhece SQLite, FFmpeg, documento nem Tauri:
quem persiste implementa o trait `JobSink`; quem faz trabalho passa uma closure `FnOnce(&JobCtx) -> Result<Value, JobError>`.

## Garantias

| Garantia | Como |
|---|---|
| Limitado | N workers fixos; fila **limitada** por categoria (`JOB_QUEUE_FULL`); nada de thread por job |
| Prioridade sem starvation | `interactive` > `normal` > `background` por **créditos** (6/3/1 por ciclo): o background sempre avança |
| Cancelamento real | `CancelToken` chega ao job; jobs de mídia o repassam ao runner de processos, que **mata o filho** |
| Sem trabalho duplicado | `dedup_key` ativa devolve o job existente |
| Panic seguro | vira falha `JOB_PANICKED`; o worker continua |
| Estados | `queued → running → completed | failed | cancelled | interrupted` |
| Desligar | `shutdown` (e `Drop`): fila e jobs em execução viram `interrupted` — **nunca** `completed` |

## Uso

```rust
let exec = Executor::new(ExecutorConfig::default(), Some(sink));
let s = exec.submit(
    JobSpec::new(JobKind::FrameIndex, Priority::Normal).dedup("index:sha256:…"),
    |ctx| {
        ctx.set_progress(1, 10);
        ctx.check()?; // JOB_CANCELLED se cancelado
        Ok(serde_json::json!({ "ok": true }))
    },
)?;
let snapshot = s.handle.wait();
```

## Testes

`cargo test -p capia-jobs` (13 testes: justiça de prioridades, cancelamento, dedup, fila cheia, panic, shutdown, progresso monotônico…).
Medições: `cargo test --release -p capia-jobs --test perf -- --ignored --nocapture`.
