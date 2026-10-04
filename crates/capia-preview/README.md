# capia-preview

Preview **headless** (ADR-066). Depende de `capia-time`, `capia-model` e `capia-render`; sem projeto, mídia ou SQLite.

- **`PreviewScheduler`:** um worker de render + um `FrameSink`. `request(t)` (seek/scrub) substitui o pedido pendente e cancela o decode em voo; o quadro obsoleto é **descartado** (`DropReason::Superseded`). `play(from, speed)` segue um `Clock` em ticks (`SystemClock`, `ManualClock`); quadro atrasado ⇒ `DropReason::Late`; fim ⇒ `PlayState::Ended`.
- **`HeadlessSink`:** registra digest/tempo/índice/avisos (sem guardar pixels). O presenter visual da Fase 3 é outro `FrameSink` (gate OD-1).
- Usa o **mesmo** `render_frame` do export; a paridade é provada em `capia-project/tests/parity.rs`. Testes: `tests/scheduler.rs` (6).
