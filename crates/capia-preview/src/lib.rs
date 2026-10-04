//! Preview **headless** (ADR-066): scheduler de render com cancelamento/supersession de quadros,
//! playhead, cadência por relógio injetável e um `FrameSink` (o preview visual da Fase 3 será outro
//! sink). Reusa o `render_frame` do export — o preview é o export de um instante — e nunca usa o
//! proxy como fonte de verdade.

mod clock;
mod scheduler;
mod sink;

pub use clock::{Clock, ManualClock, SystemClock};
pub use scheduler::{PlayState, PreviewScheduler, PreviewStats};
pub use sink::{DropReason, DroppedFrame, FrameSink, HeadlessSink, Presented, PreviewFrame};
