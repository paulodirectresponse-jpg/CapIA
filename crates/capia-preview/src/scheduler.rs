//! Scheduler de preview headless (ADR-066). Um worker de render, um destino (`FrameSink`):
//!
//! * **Seek/scrub**: `request(t)` renderiza o instante `t`; um pedido novo torna o anterior
//!   obsoleto — se este ainda não começou é pulado, se já está renderizando é **descartado** ao
//!   terminar (nunca apresentado) e `MediaSource::cancel_pending` aborta o decode em andamento.
//! * **Reprodução**: o relógio (em ticks) define qual quadro está no prazo; o worker renderiza o
//!   quadro do instante e, se na hora de entregar já há um quadro mais novo no prazo, descarta o
//!   atrasado (`DropReason::Late`) em vez de acumular atraso.
//!
//! O mesmo `render_frame` do export é usado: o preview é o export de um instante.

use crate::clock::Clock;
use crate::sink::{DropReason, DroppedFrame, FrameSink, PreviewFrame};
use capia_model::SequenceId;
use capia_render::{MediaSource, RenderGraph, RenderSettings, render_frame};
use capia_time::{FrameRate, Rational, Ticks};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreviewStats {
    pub renders: u64,
    pub presented: u64,
    pub dropped_superseded: u64,
    pub dropped_late: u64,
    pub failed: u64,
    pub skipped_before_render: u64,
    pub max_render_micros: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Stopped,
    Playing,
    /// Chegou ao fim da sequence.
    Ended,
}

#[derive(Clone, Copy, Debug)]
struct Seek {
    id: u64,
    time: Ticks,
}

#[derive(Clone, Copy, Debug)]
struct Play {
    origin_time: Ticks,
    origin_clock: Ticks,
    speed: Rational,
}

#[derive(Debug)]
struct Ctl {
    shutdown: bool,
    next_id: u64,
    pending: Option<Seek>,
    play: Option<Play>,
    play_epoch: u64,
    last_played: Option<i64>,
    playhead: Ticks,
    ended: bool,
}

struct Shared {
    ctl: Mutex<Ctl>,
    cv: Condvar,
    clock: Arc<dyn Clock>,
    source: Arc<dyn MediaSource>,
    duration: Ticks,
    frame_rate: FrameRate,
    stats: Mutex<PreviewStats>,
    generation: AtomicU64,
}

pub struct PreviewScheduler {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for PreviewScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewScheduler").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Tempo de reprodução no relógio `clock`: `origin + (clock − origin_clock) × speed`.
fn play_time(p: &Play, clock: Ticks) -> Ticks {
    let d = i128::from(clock.0) - i128::from(p.origin_clock.0);
    let t = i128::from(p.origin_time.0) + d * i128::from(p.speed.num()) / i128::from(p.speed.den());
    Ticks(i64::try_from(t).unwrap_or(i64::MAX))
}

fn frame_of(t: Ticks, fd: i64) -> i64 {
    t.0.div_euclid(fd.max(1))
}

/// Relógio em que o quadro `n` vence (teto): inverte `play_time`.
fn due_clock(p: &Play, n: i64, fd: i64) -> Ticks {
    let target = i128::from(n) * i128::from(fd);
    let d = (target - i128::from(p.origin_time.0)) * i128::from(p.speed.den());
    let num = i128::from(p.speed.num());
    let c = i128::from(p.origin_clock.0) + (d + num - 1).div_euclid(num);
    Ticks(i64::try_from(c).unwrap_or(i64::MAX))
}

fn ticks_to_duration(t: Ticks) -> Duration {
    let ns = i128::from(t.0.max(0)) * 1_000_000_000 / i128::from(capia_time::TICKS_PER_SECOND);
    Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX))
}

impl PreviewScheduler {
    pub fn new(
        graph: Arc<RenderGraph>,
        seq: SequenceId,
        source: Arc<dyn MediaSource>,
        settings: RenderSettings,
        mut sink: Box<dyn FrameSink>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, capia_render::RenderError> {
        settings.validate()?;
        let gs = graph.sequence(&seq)?;
        let shared = Arc::new(Shared {
            ctl: Mutex::new(Ctl {
                shutdown: false,
                next_id: 0,
                pending: None,
                play: None,
                play_epoch: 0,
                last_played: None,
                playhead: Ticks(0),
                ended: false,
            }),
            cv: Condvar::new(),
            clock,
            source,
            duration: gs.duration,
            frame_rate: settings.frame_rate.unwrap_or(gs.frame_rate),
            stats: Mutex::new(PreviewStats::default()),
            generation: AtomicU64::new(0),
        });
        let sh = Arc::clone(&shared);
        let worker = std::thread::spawn(move || worker(&sh, &graph, &seq, &settings, &mut *sink));
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    /// Pede o quadro de `time` (seek/scrub). Substitui qualquer pedido anterior ainda não entregue.
    pub fn request(&self, time: Ticks) -> u64 {
        let mut c = lock(&self.shared.ctl);
        c.next_id += 1;
        let id = c.next_id;
        c.pending = Some(Seek { id, time });
        // um seek interrompe a reprodução
        c.play = None;
        c.ended = false;
        c.playhead = time;
        drop(c);
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        // aborta o decode em andamento: o quadro em render ficou obsoleto
        self.shared.source.cancel_pending();
        self.shared.cv.notify_all();
        id
    }

    /// Começa a reprodução em `from` com `speed` (1/1 = tempo real).
    pub fn play(&self, from: Ticks, speed: Rational) {
        let speed = if speed.is_positive() {
            speed
        } else {
            Rational::ONE
        };
        let mut c = lock(&self.shared.ctl);
        c.pending = None;
        c.play = Some(Play {
            origin_time: from,
            origin_clock: self.shared.clock.now(),
            speed,
        });
        c.play_epoch += 1;
        c.last_played = None;
        c.ended = false;
        c.playhead = from;
        drop(c);
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        self.shared.source.cancel_pending();
        self.shared.cv.notify_all();
    }

    pub fn pause(&self) {
        let mut c = lock(&self.shared.ctl);
        if let Some(p) = c.play.take() {
            let t = play_time(&p, self.shared.clock.now());
            c.playhead = t;
        }
        drop(c);
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        self.shared.cv.notify_all();
    }

    pub fn state(&self) -> PlayState {
        let c = lock(&self.shared.ctl);
        if c.play.is_some() {
            PlayState::Playing
        } else if c.ended {
            PlayState::Ended
        } else {
            PlayState::Stopped
        }
    }

    /// Posição atual: na reprodução, o tempo do relógio; parado, o último seek/pausa.
    pub fn playhead(&self) -> Ticks {
        let c = lock(&self.shared.ctl);
        match &c.play {
            Some(p) => play_time(p, self.shared.clock.now()),
            None => c.playhead,
        }
    }

    pub fn stats(&self) -> PreviewStats {
        *lock(&self.shared.stats)
    }

    /// Acorda o worker (use depois de avançar um `ManualClock`).
    pub fn poke(&self) {
        self.shared.cv.notify_all();
    }

    pub fn frame_rate(&self) -> FrameRate {
        self.shared.frame_rate
    }

    pub fn shutdown(&mut self) {
        lock(&self.shared.ctl).shutdown = true;
        self.shared.source.cancel_pending();
        self.shared.cv.notify_all();
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

impl Drop for PreviewScheduler {
    fn drop(&mut self) {
        self.shutdown();
    }
}

enum Target {
    Seek(Seek),
    Play { n: i64, epoch: u64 },
}

fn worker(
    sh: &Arc<Shared>,
    graph: &RenderGraph,
    seq: &SequenceId,
    settings: &RenderSettings,
    sink: &mut dyn FrameSink,
) {
    let fd = sh.frame_rate.frame_duration().0.max(1);
    loop {
        let target = {
            let mut c = lock(&sh.ctl);
            loop {
                if c.shutdown {
                    return;
                }
                if let Some(s) = c.pending.take() {
                    break Target::Seek(s);
                }
                if let Some(p) = c.play {
                    let now = sh.clock.now();
                    let t = play_time(&p, now);
                    if t >= sh.duration {
                        c.play = None;
                        c.ended = true;
                        c.playhead = sh.duration;
                        continue;
                    }
                    let n = frame_of(t, fd);
                    if c.last_played.is_none_or(|l| l < n) {
                        break Target::Play {
                            n,
                            epoch: c.play_epoch,
                        };
                    }
                    // quadro `n` já entregue: espera o vencimento do próximo
                    let wait = if sh.clock.is_manual() {
                        Duration::from_millis(1)
                    } else {
                        let due = due_clock(&p, n + 1, fd);
                        ticks_to_duration(Ticks((due.0 - now.0).max(0)))
                            .max(Duration::from_micros(200))
                    };
                    c = sh
                        .cv
                        .wait_timeout(c, wait)
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .0;
                    continue;
                }
                c = sh
                    .cv
                    .wait(c)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        let (id, time, index, epoch) = match &target {
            Target::Seek(s) => (s.id, s.time, None, 0),
            Target::Play { n, epoch, .. } => {
                let mut c = lock(&sh.ctl);
                c.next_id += 1;
                (c.next_id, Ticks(n.saturating_mul(fd)), Some(*n), *epoch)
            }
        };
        let gen_before = sh.generation.load(Ordering::SeqCst);
        let started = Instant::now();
        let result = render_frame(graph, seq, time, settings, &*sh.source);
        let took = started.elapsed();
        {
            let mut s = lock(&sh.stats);
            s.renders += 1;
            s.max_render_micros = s.max_render_micros.max(took.as_micros() as u64);
        }
        // obsoleto? (um pedido/play/pause mais novo, ou — na reprodução — um quadro posterior no prazo)
        let stale = {
            let mut c = lock(&sh.ctl);
            let superseded =
                sh.generation.load(Ordering::SeqCst) != gen_before || c.pending.is_some();
            let late = match (&target, &c.play) {
                (Target::Play { n, .. }, Some(p)) => {
                    c.play_epoch == epoch && frame_of(play_time(p, sh.clock.now()), fd) > *n
                }
                _ => false,
            };
            if let Target::Play { n, .. } = &target
                && c.play_epoch == epoch
                && !superseded
            {
                c.last_played = Some(c.last_played.map_or(*n, |l| l.max(*n)));
            }
            if superseded {
                Some(DropReason::Superseded)
            } else if late {
                Some(DropReason::Late)
            } else {
                None
            }
        };
        match (result, stale) {
            (Ok(f), None) => {
                lock(&sh.stats).presented += 1;
                {
                    let mut c = lock(&sh.ctl);
                    if c.play.is_none() {
                        c.playhead = time;
                    }
                }
                sink.present(PreviewFrame {
                    request_id: id,
                    time,
                    frame_index: index,
                    image: f.image,
                    warnings: f.warnings,
                    render_time: took,
                });
            }
            (_, Some(reason)) => {
                {
                    let mut s = lock(&sh.stats);
                    match reason {
                        DropReason::Superseded => s.dropped_superseded += 1,
                        DropReason::Late => s.dropped_late += 1,
                    }
                }
                sink.dropped(DroppedFrame {
                    request_id: id,
                    time,
                    reason,
                });
            }
            (Err(e), None) => {
                lock(&sh.stats).failed += 1;
                sink.failed(id, &e);
            }
        }
    }
}
