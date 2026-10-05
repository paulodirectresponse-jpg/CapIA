//! Serviço de decode persistente (ADR-059): pool limitado de sessões de ffmpeg reaproveitadas,
//! cache de quadros por bytes, fila com prioridade (Interactive > Playback > Background),
//! cancelamento e *supersession* por pista (scrub rápido descarta os pedidos obsoletos).

use crate::cache::{ByteLru, CacheStats};
use capia_media::{
    DecodeLimits, FrameIndex, FrameStream, MediaError, MediaErrorCode, MediaToolchain, PixelFormat,
    RawFrame,
};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Versão do backend de decode: entra na chave do cache (mudar o decoder invalida tudo).
pub const DECODE_BACKEND_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Background = 0,
    Playback = 1,
    Interactive = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Forward,
    Backward,
    Jump,
}

/// Pista de pedidos: um novo pedido na mesma pista torna os anteriores obsoletos.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Lane(pub u64);

/// Uma fonte de vídeo decodificável: **sempre o original** (o proxy nunca é fonte de verdade).
#[derive(Clone, Debug)]
pub struct VideoSource {
    /// Isolamento entre projetos que compartilham o processo.
    pub namespace: u64,
    /// Identidade por conteúdo (`sha256:…`).
    pub content: Arc<str>,
    pub path: PathBuf,
    pub index: Arc<FrameIndex>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FrameKey {
    pub namespace: u64,
    pub content: Arc<str>,
    pub stream: u32,
    pub pts: i64,
    pub pixel_format: PixelFormat,
    pub backend: u32,
}

impl FrameKey {
    fn of(src: &VideoSource, pts: i64) -> Self {
        Self {
            namespace: src.namespace,
            content: Arc::clone(&src.content),
            stream: src.index.stream_index(),
            pts,
            pixel_format: PixelFormat::Rgba8,
            backend: DECODE_BACKEND_VERSION,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    Cancelled,
    /// Um pedido mais novo da mesma pista tomou o lugar deste.
    Superseded,
    Shutdown,
    Media(MediaError),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("DECODE_CANCELLED"),
            Self::Superseded => f.write_str("DECODE_SUPERSEDED"),
            Self::Shutdown => f.write_str("DECODE_SHUTDOWN"),
            Self::Media(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for DecodeError {}

#[derive(Clone, Debug)]
pub struct DecodeConfig {
    pub toolchain: MediaToolchain,
    /// Sessões (processos ffmpeg) simultâneas; também o nº de workers.
    pub max_sessions: usize,
    pub frame_cache_bytes: u64,
    /// Sessão ociosa por mais que isso é encerrada.
    pub idle_timeout: Duration,
    /// Quantos quadros à frente vale decodificar-e-descartar numa sessão existente em vez de abrir outra.
    pub forward_gap: usize,
    pub prefetch_ahead: usize,
    pub backward_window: usize,
    pub limits: DecodeLimits,
}

impl DecodeConfig {
    pub fn new(toolchain: MediaToolchain) -> Self {
        Self {
            toolchain,
            max_sessions: 3,
            frame_cache_bytes: 256 * 1024 * 1024,
            idle_timeout: Duration::from_secs(20),
            forward_gap: 24,
            prefetch_ahead: 8,
            backward_window: 6,
            limits: DecodeLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeMetrics {
    pub requests: u64,
    pub completed: [u64; 3],
    pub superseded: u64,
    pub cancelled: u64,
    pub failed: u64,
    pub cache: CacheStats,
    pub frames_decoded: u64,
    pub sessions_opened: u64,
    /// Pedidos atendidos por uma sessão que já existia (sem processo novo).
    pub sessions_reused: u64,
    pub sessions_evicted_idle: u64,
    pub sessions_evicted_pressure: u64,
    pub sessions_alive: usize,
    pub decode_micros: u64,
    pub queue_wait_micros: u64,
}

impl DecodeMetrics {
    /// Fração de pedidos de quadro resolvidos sem abrir sessão nova (cache ou sessão reaproveitada).
    pub fn reuse_rate(&self) -> f64 {
        let served = self.cache.hits + self.sessions_reused + self.sessions_opened;
        if served == 0 {
            0.0
        } else {
            (self.cache.hits + self.sessions_reused) as f64 / served as f64
        }
    }
}

#[derive(Debug)]
enum Outcome {
    Pending,
    Done(Arc<RawFrame>),
    Failed(DecodeError),
}

#[derive(Debug)]
struct TicketState {
    outcome: Mutex<Outcome>,
    cv: Condvar,
    abort: AtomicBool,
    /// Marcado por supersession (o resultado é descartado mesmo que o quadro tenha sido decodificado).
    superseded: AtomicBool,
    completed_seq: AtomicU64,
}

/// Promessa de um quadro. `wait` bloqueia; `cancel` aborta o pedido.
#[derive(Clone, Debug)]
pub struct FrameTicket {
    state: Arc<TicketState>,
}

impl FrameTicket {
    fn new() -> Self {
        Self {
            state: Arc::new(TicketState {
                outcome: Mutex::new(Outcome::Pending),
                cv: Condvar::new(),
                abort: AtomicBool::new(false),
                superseded: AtomicBool::new(false),
                completed_seq: AtomicU64::new(0),
            }),
        }
    }

    pub fn wait(&self) -> Result<Arc<RawFrame>, DecodeError> {
        let mut g = lock(&self.state.outcome);
        loop {
            match &*g {
                Outcome::Pending => {
                    g = self
                        .state
                        .cv
                        .wait(g)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                Outcome::Done(f) => return Ok(Arc::clone(f)),
                Outcome::Failed(e) => return Err(e.clone()),
            }
        }
    }

    pub fn wait_timeout(&self, d: Duration) -> Option<Result<Arc<RawFrame>, DecodeError>> {
        let deadline = Instant::now() + d;
        let mut g = lock(&self.state.outcome);
        loop {
            match &*g {
                Outcome::Pending => {
                    let left = deadline.checked_duration_since(Instant::now())?;
                    g = self
                        .state
                        .cv
                        .wait_timeout(g, left)
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .0;
                }
                Outcome::Done(f) => return Some(Ok(Arc::clone(f))),
                Outcome::Failed(e) => return Some(Err(e.clone())),
            }
        }
    }

    pub fn is_done(&self) -> bool {
        !matches!(&*lock(&self.state.outcome), Outcome::Pending)
    }

    pub fn cancel(&self) {
        self.state.abort.store(true, Ordering::SeqCst);
    }

    /// Ordem global de conclusão (0 enquanto pendente): testes de prioridade a usam.
    pub fn completed_seq(&self) -> u64 {
        self.state.completed_seq.load(Ordering::SeqCst)
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

enum Job {
    Frame(usize),
    /// Garante `[first, last]` no cache (prefetch).
    Range(usize, usize),
}

struct Request {
    src: Arc<VideoSource>,
    job: Job,
    prio: Priority,
    lane: Option<Lane>,
    seq: u64,
    submitted: Instant,
    ticket: FrameTicket,
}

impl PartialEq for Request {
    fn eq(&self, o: &Self) -> bool {
        self.seq == o.seq
    }
}
impl Eq for Request {}
impl PartialOrd for Request {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Request {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (self.prio, Reverse(self.seq)).cmp(&(o.prio, Reverse(o.seq)))
    }
}

struct Session {
    key: (u64, Arc<str>, u32),
    stream: FrameStream,
    last_used: Instant,
}

struct State {
    queue: BinaryHeap<Request>,
    /// Pedidos de cada pista que ainda não terminaram (para a supersession alcançar o em andamento).
    lanes: HashMap<Lane, Vec<FrameTicket>>,
    pool: Vec<Session>,
    sessions_alive: usize,
    seq: u64,
    shutdown: bool,
    metrics: DecodeMetrics,
}

struct Inner {
    cfg: DecodeConfig,
    state: Mutex<State>,
    cv: Condvar,
    cache: Mutex<ByteLru<FrameKey, RawFrame>>,
    done_seq: AtomicU64,
}

#[derive(Debug)]
pub struct DecodeService {
    inner: Arc<Inner>,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner").finish_non_exhaustive()
    }
}

impl DecodeService {
    pub fn new(mut cfg: DecodeConfig) -> Self {
        cfg.max_sessions = cfg.max_sessions.max(1);
        let n = cfg.max_sessions;
        let inner = Arc::new(Inner {
            cache: Mutex::new(ByteLru::new(cfg.frame_cache_bytes)),
            state: Mutex::new(State {
                queue: BinaryHeap::new(),
                lanes: HashMap::new(),
                pool: Vec::new(),
                sessions_alive: 0,
                seq: 0,
                shutdown: false,
                metrics: DecodeMetrics::default(),
            }),
            cv: Condvar::new(),
            cfg,
            done_seq: AtomicU64::new(0),
        });
        let threads = (0..n)
            .map(|_| {
                let inner = Arc::clone(&inner);
                std::thread::spawn(move || worker(&inner))
            })
            .collect();
        Self {
            inner,
            threads: Mutex::new(threads),
        }
    }

    pub fn config(&self) -> &DecodeConfig {
        &self.inner.cfg
    }

    /// Pede o quadro lógico `frame`. Com `lane`, torna obsoletos os pedidos anteriores da pista.
    pub fn request(
        &self,
        src: &Arc<VideoSource>,
        frame: usize,
        prio: Priority,
        lane: Option<Lane>,
    ) -> FrameTicket {
        self.submit(src, Job::Frame(frame), prio, lane)
    }

    /// Atalho síncrono (sem pista).
    pub fn get_frame(
        &self,
        src: &Arc<VideoSource>,
        frame: usize,
        prio: Priority,
    ) -> Result<Arc<RawFrame>, DecodeError> {
        self.request(src, frame, prio, None).wait()
    }

    /// Entrega o quadro se já estiver no cache (sem decodificar).
    pub fn cached(&self, src: &VideoSource, frame: usize) -> Option<Arc<RawFrame>> {
        let pts = src.index.frame_by_index(frame)?.pts;
        lock(&self.inner.cache).get(&FrameKey::of(src, pts))
    }

    /// Prefetch em segundo plano ao redor de `from` (quadro recém-mostrado).
    /// `Forward`: os próximos `prefetch_ahead`; `Backward`: até `backward_window` anteriores;
    /// `Jump`: nada (o destino do salto é pedido de forma interativa).
    pub fn prefetch(
        &self,
        src: &Arc<VideoSource>,
        from: usize,
        dir: Direction,
        lane: Option<Lane>,
    ) -> Option<FrameTicket> {
        let cfg = &self.inner.cfg;
        let last = src.index.len().checked_sub(1)?;
        let (a, b) = match dir {
            Direction::Forward => (
                from.checked_add(1)?,
                from.saturating_add(cfg.prefetch_ahead).min(last),
            ),
            Direction::Backward => (
                from.saturating_sub(cfg.backward_window),
                from.checked_sub(1)?,
            ),
            Direction::Jump => return None,
        };
        if a > b || a > last {
            return None;
        }
        Some(self.submit(src, Job::Range(a, b), Priority::Background, lane))
    }

    fn submit(
        &self,
        src: &Arc<VideoSource>,
        job: Job,
        prio: Priority,
        lane: Option<Lane>,
    ) -> FrameTicket {
        let ticket = FrameTicket::new();
        let mut st = lock(&self.inner.state);
        if st.shutdown {
            fail(&self.inner, &ticket, DecodeError::Shutdown);
            return ticket;
        }
        st.metrics.requests += 1;
        if let Some(l) = lane {
            // supersession: tudo que a pista ainda tem pendente vira obsoleto
            let olds = st.lanes.entry(l).or_default();
            for t in olds.iter() {
                t.state.superseded.store(true, Ordering::SeqCst);
                t.state.abort.store(true, Ordering::SeqCst);
            }
            olds.retain(|t| !t.is_done());
            olds.push(ticket.clone());
        }
        st.seq += 1;
        let seq = st.seq;
        st.queue.push(Request {
            src: Arc::clone(src),
            job,
            prio,
            lane,
            seq,
            submitted: Instant::now(),
            ticket: ticket.clone(),
        });
        drop(st);
        self.inner.cv.notify_one();
        ticket
    }

    pub fn metrics(&self) -> DecodeMetrics {
        let st = lock(&self.inner.state);
        let mut m = st.metrics;
        m.sessions_alive = st.sessions_alive;
        drop(st);
        m.cache = lock(&self.inner.cache).stats();
        m
    }

    /// Encerra as sessões ociosas há mais que `idle_timeout`. Devolve quantas.
    pub fn evict_idle(&self) -> usize {
        evict_idle(&self.inner)
    }

    /// Esquece tudo de um projeto (quadros em cache e sessões ociosas).
    pub fn invalidate_namespace(&self, namespace: u64) {
        lock(&self.inner.cache).retain(|k| k.namespace != namespace);
        let mut st = lock(&self.inner.state);
        let before = st.pool.len();
        st.pool.retain(|s| s.key.0 != namespace);
        let gone = before - st.pool.len();
        st.sessions_alive -= gone;
    }

    /// Esquece um conteúdo (o arquivo foi trocado/relinkado).
    pub fn invalidate_content(&self, namespace: u64, content: &str) {
        lock(&self.inner.cache).retain(|k| !(k.namespace == namespace && &*k.content == content));
        let mut st = lock(&self.inner.state);
        let before = st.pool.len();
        st.pool
            .retain(|s| !(s.key.0 == namespace && &*s.key.1 == content));
        let gone = before - st.pool.len();
        st.sessions_alive -= gone;
    }

    /// Para os workers, resolve os pendentes como `Shutdown` e mata as sessões.
    pub fn shutdown(&self) {
        {
            let mut st = lock(&self.inner.state);
            st.shutdown = true;
            let pending: Vec<Request> = st.queue.drain().collect();
            for r in &pending {
                fail(&self.inner, &r.ticket, DecodeError::Shutdown);
            }
            let n = st.pool.len();
            st.pool.clear();
            st.sessions_alive -= n;
        }
        self.inner.cv.notify_all();
        let handles: Vec<_> = lock(&self.threads).drain(..).collect();
        for h in handles {
            let _ = h.join();
        }
    }
}

impl Drop for DecodeService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn fail(inner: &Inner, t: &FrameTicket, e: DecodeError) {
    let seq = inner.done_seq.fetch_add(1, Ordering::SeqCst) + 1;
    t.state.completed_seq.store(seq, Ordering::SeqCst);
    *lock(&t.state.outcome) = Outcome::Failed(e);
    t.state.cv.notify_all();
}

fn finish(inner: &Inner, t: &FrameTicket, frame: Arc<RawFrame>) {
    let seq = inner.done_seq.fetch_add(1, Ordering::SeqCst) + 1;
    t.state.completed_seq.store(seq, Ordering::SeqCst);
    *lock(&t.state.outcome) = Outcome::Done(frame);
    t.state.cv.notify_all();
}

fn evict_idle(inner: &Inner) -> usize {
    let mut st = lock(&inner.state);
    let now = Instant::now();
    let before = st.pool.len();
    let idle = inner.cfg.idle_timeout;
    st.pool.retain(|s| now.duration_since(s.last_used) < idle);
    let gone = before - st.pool.len();
    st.sessions_alive -= gone;
    st.metrics.sessions_evicted_idle += gone as u64;
    gone
}

fn worker(inner: &Arc<Inner>) {
    loop {
        let req = {
            let mut st = lock(&inner.state);
            loop {
                if st.shutdown {
                    return;
                }
                if let Some(r) = st.queue.pop() {
                    break r;
                }
                let tick = (inner.cfg.idle_timeout / 2)
                    .clamp(Duration::from_millis(20), Duration::from_secs(2));
                let (g, _) = inner
                    .cv
                    .wait_timeout(st, tick)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                st = g;
                drop(st);
                evict_idle(inner);
                st = lock(&inner.state);
            }
        };
        run_request(inner, req);
    }
}

fn run_request(inner: &Arc<Inner>, req: Request) {
    let waited = req.submitted.elapsed().as_micros() as u64;
    let t = &req.ticket;
    let started = Instant::now();
    let result = if t.state.abort.load(Ordering::SeqCst) {
        Err(abort_error(t))
    } else {
        match req.job {
            Job::Frame(f) => serve_frame(inner, &req.src, f, t),
            Job::Range(a, b) => serve_range(inner, &req.src, a, b, t),
        }
    };
    let spent = started.elapsed().as_micros() as u64;
    let mut st = lock(&inner.state);
    st.metrics.queue_wait_micros += waited;
    st.metrics.decode_micros += spent;
    if let Some(l) = req.lane
        && let Some(v) = st.lanes.get_mut(&l)
    {
        v.retain(|x| !Arc::ptr_eq(&x.state, &t.state));
        if v.is_empty() {
            st.lanes.remove(&l);
        }
    }
    // supersession vence mesmo que o quadro tenha chegado a ser decodificado
    // (e um aborto interno causado pela supersessão também vira `Superseded`: o motivo do abort é o
    // que conta, não em que ponto da execução ele foi observado — o erro era uma corrida)
    let result = match result {
        Ok(_) if t.state.superseded.load(Ordering::SeqCst) => Err(DecodeError::Superseded),
        Err(DecodeError::Cancelled) if t.state.superseded.load(Ordering::SeqCst) => {
            Err(DecodeError::Superseded)
        }
        other => other,
    };
    match &result {
        Ok(_) => st.metrics.completed[req.prio as usize] += 1,
        Err(DecodeError::Superseded) => st.metrics.superseded += 1,
        Err(DecodeError::Cancelled) => st.metrics.cancelled += 1,
        Err(_) => st.metrics.failed += 1,
    }
    drop(st);
    match result {
        Ok(f) => finish(inner, t, f),
        Err(e) => fail(inner, t, e),
    }
}

fn abort_error(t: &FrameTicket) -> DecodeError {
    if t.state.superseded.load(Ordering::SeqCst) {
        DecodeError::Superseded
    } else {
        DecodeError::Cancelled
    }
}

fn serve_range(
    inner: &Arc<Inner>,
    src: &Arc<VideoSource>,
    a: usize,
    b: usize,
    t: &FrameTicket,
) -> Result<Arc<RawFrame>, DecodeError> {
    let mut last = None;
    for i in a..=b {
        if t.state.abort.load(Ordering::SeqCst) {
            return Err(abort_error(t));
        }
        last = Some(serve_frame(inner, src, i, t)?);
    }
    last.ok_or(DecodeError::Cancelled)
}

fn session_key(src: &VideoSource) -> (u64, Arc<str>, u32) {
    (
        src.namespace,
        Arc::clone(&src.content),
        src.index.stream_index(),
    )
}

fn cache_put(inner: &Inner, src: &VideoSource, f: RawFrame) -> Arc<RawFrame> {
    let bytes = f.bytes.len() as u64;
    let key = FrameKey::of(src, f.pts);
    let arc = Arc::new(f);
    lock(&inner.cache).insert(key, Arc::clone(&arc), bytes);
    arc
}

/// Atende um quadro: cache → sessão do pool posicionada → sessão nova.
fn serve_frame(
    inner: &Arc<Inner>,
    src: &Arc<VideoSource>,
    frame: usize,
    t: &FrameTicket,
) -> Result<Arc<RawFrame>, DecodeError> {
    let entry = src.index.frame_by_index(frame).ok_or_else(|| {
        DecodeError::Media(MediaError::new(
            MediaErrorCode::MediaFrameNotFound,
            format!("frame {frame} is outside the index"),
        ))
    })?;
    let key = FrameKey::of(src, entry.pts);
    if let Some(hit) = lock(&inner.cache).get(&key) {
        return Ok(hit);
    }
    let abort = || t.state.abort.load(Ordering::SeqCst);
    let skey = session_key(src);
    let gap = inner.cfg.forward_gap;
    // sessão do pool que já está logo antes do quadro pedido?
    let taken = {
        let mut st = lock(&inner.state);
        let pick = st
            .pool
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.key == skey
                    && s.stream.next_index() <= frame
                    && frame - s.stream.next_index() <= gap
            })
            .max_by_key(|(_, s)| s.stream.next_index())
            .map(|(i, _)| i);
        match pick {
            Some(i) => {
                st.metrics.sessions_reused += 1;
                Some(st.pool.swap_remove(i))
            }
            None => None,
        }
    };
    let mut got: Option<Arc<RawFrame>> = None;
    let mut session = match taken {
        Some(s) => s,
        None => {
            let (s, first) = open_session(inner, src, frame, &skey, &abort)?;
            got = Some(first);
            s
        }
    };
    while session.stream.next_index() <= frame {
        if abort() {
            // sessão íntegra: volta ao pool na posição atual
            give_back(inner, session);
            return Err(abort_error(t));
        }
        match session.stream.next_frame(&|| false) {
            Ok(Some(f)) => {
                lock(&inner.state).metrics.frames_decoded += 1;
                let idx = f.index;
                let arc = cache_put(inner, src, f);
                if idx == frame {
                    got = Some(arc);
                }
            }
            Ok(None) => {
                drop_session(inner);
                return Err(DecodeError::Media(MediaError::new(
                    MediaErrorCode::MediaFrameNotFound,
                    format!("the decoder ended before frame {frame}"),
                )));
            }
            Err(e) => {
                drop_session(inner);
                return Err(DecodeError::Media(e));
            }
        }
    }
    give_back(inner, session);
    got.ok_or_else(|| {
        DecodeError::Media(MediaError::new(
            MediaErrorCode::MediaFrameNotFound,
            format!("frame {frame} was not produced"),
        ))
    })
}

fn give_back(inner: &Inner, mut s: Session) {
    s.last_used = Instant::now();
    let mut st = lock(&inner.state);
    if st.shutdown {
        st.sessions_alive -= 1;
        return;
    }
    st.pool.push(s);
}

/// Uma sessão morreu (erro/fim): sai da contagem.
fn drop_session(inner: &Inner) {
    lock(&inner.state).sessions_alive -= 1;
}

fn open_session(
    inner: &Arc<Inner>,
    src: &Arc<VideoSource>,
    frame: usize,
    skey: &(u64, Arc<str>, u32),
    abort: &dyn Fn() -> bool,
) -> Result<(Session, Arc<RawFrame>), DecodeError> {
    // reserva uma vaga (expulsando a sessão ociosa menos recente se o pool estiver cheio)
    {
        let mut st = lock(&inner.state);
        if st.sessions_alive >= inner.cfg.max_sessions {
            let victim = st
                .pool
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.last_used)
                .map(|(i, _)| i);
            if let Some(i) = victim {
                st.pool.swap_remove(i);
                st.sessions_alive -= 1;
                st.metrics.sessions_evicted_pressure += 1;
            }
        }
        st.sessions_alive += 1;
        st.metrics.sessions_opened += 1;
    }
    let mut last_err: Option<MediaError> = None;
    for attempt in 0u8..3 {
        if abort() {
            drop_session(inner);
            return Err(DecodeError::Cancelled);
        }
        match FrameStream::open(
            &inner.cfg.toolchain,
            &src.path,
            &src.index,
            src.width,
            src.height,
            frame,
            attempt,
            &inner.cfg.limits,
        ) {
            Ok(mut stream) => {
                // o 1º quadro prova que a sessão pousou certo; vem para o cache e fica "à frente"
                match stream.next_frame(abort) {
                    Ok(Some(f)) if f.index == frame => {
                        lock(&inner.state).metrics.frames_decoded += 1;
                        let first = cache_put(inner, src, f);
                        return Ok((
                            Session {
                                key: skey.clone(),
                                stream,
                                last_used: Instant::now(),
                            },
                            first,
                        ));
                    }
                    Ok(_) => {
                        last_err = Some(MediaError::new(
                            MediaErrorCode::MediaFrameNotFound,
                            "the session did not land on the requested frame",
                        ));
                    }
                    Err(e) if e.code == MediaErrorCode::MediaCancelled => {
                        drop_session(inner);
                        return Err(DecodeError::Cancelled);
                    }
                    Err(e) => last_err = Some(e),
                }
            }
            Err(e) => {
                last_err = Some(e);
                break;
            }
        }
    }
    drop_session(inner);
    Err(DecodeError::Media(last_err.unwrap_or_else(|| {
        MediaError::new(
            MediaErrorCode::MediaDecodeFailed,
            "cannot open a decode session",
        )
    })))
}
