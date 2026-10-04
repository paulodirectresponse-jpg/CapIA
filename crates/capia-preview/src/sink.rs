use capia_render::{Image, RenderError, RenderWarning, frame_digest};
use capia_time::Ticks;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Um quadro entregue ao destino.
#[derive(Debug)]
pub struct PreviewFrame {
    pub request_id: u64,
    /// Instante da sequence que foi renderizado.
    pub time: Ticks,
    /// Índice do quadro na cadência da sequence quando veio da reprodução; `None` num seek.
    pub frame_index: Option<i64>,
    pub image: Image,
    pub warnings: Vec<RenderWarning>,
    /// Tempo gasto renderizando.
    pub render_time: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    /// Um pedido mais novo (seek/scrub) tornou este obsoleto.
    Superseded,
    /// Na reprodução, um quadro posterior já estava no prazo quando este ficou pronto.
    Late,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DroppedFrame {
    pub request_id: u64,
    pub time: Ticks,
    pub reason: DropReason,
}

/// Destino dos quadros. O preview visual (Fase 3) será outro `FrameSink`; aqui há o headless.
pub trait FrameSink: Send {
    fn present(&mut self, frame: PreviewFrame);
    fn dropped(&mut self, _d: DroppedFrame) {}
    fn failed(&mut self, _request_id: u64, _e: &RenderError) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presented {
    pub request_id: u64,
    pub time: Ticks,
    pub frame_index: Option<i64>,
    pub digest: String,
    pub warnings: Vec<&'static str>,
}

#[derive(Debug, Default)]
struct Log {
    presented: Vec<Presented>,
    dropped: Vec<DroppedFrame>,
    failed: Vec<(u64, String)>,
}

/// Sink de teste/CLI: registra o digest de cada quadro (sem guardar pixels).
#[derive(Clone, Debug, Default)]
pub struct HeadlessSink {
    log: Arc<(Mutex<Log>, Condvar)>,
}

impl HeadlessSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn presented(&self) -> Vec<Presented> {
        self.lock().presented.clone()
    }

    pub fn dropped(&self) -> Vec<DroppedFrame> {
        self.lock().dropped.clone()
    }

    pub fn failures(&self) -> Vec<(u64, String)> {
        self.lock().failed.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Log> {
        self.log
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Espera até haver `n` quadros apresentados (ou o prazo).
    pub fn wait_presented(&self, n: usize, timeout: Duration) -> bool {
        self.wait(|l| l.presented.len() >= n, timeout)
    }

    /// Espera até haver `n` eventos (apresentados + descartados + falhas) no total.
    pub fn wait_events(&self, n: usize, timeout: Duration) -> bool {
        self.wait(
            |l| l.presented.len() + l.dropped.len() + l.failed.len() >= n,
            timeout,
        )
    }

    fn wait(&self, ok: impl Fn(&Log) -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut g = self.lock();
        while !ok(&g) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            g = self
                .log
                .1
                .wait_timeout(g, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        true
    }
}

impl FrameSink for HeadlessSink {
    fn present(&mut self, f: PreviewFrame) {
        let digest = frame_digest(&f.image);
        let mut g = self.lock();
        g.presented.push(Presented {
            request_id: f.request_id,
            time: f.time,
            frame_index: f.frame_index,
            digest,
            warnings: f.warnings.iter().map(|w| w.code).collect(),
        });
        self.log.1.notify_all();
    }

    fn dropped(&mut self, d: DroppedFrame) {
        self.lock().dropped.push(d);
        self.log.1.notify_all();
    }

    fn failed(&mut self, request_id: u64, e: &RenderError) {
        self.lock().failed.push((request_id, e.to_string()));
        self.log.1.notify_all();
    }
}
