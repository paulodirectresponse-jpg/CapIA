use crate::types::{
    CODE_PANICKED, CancelToken, JobCtx, JobError, JobId, JobSink, JobSnapshot, JobSpec, JobState,
    Priority, Progress, ProgressCell, SubmitError,
};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type JobFn = Box<dyn FnOnce(&JobCtx) -> Result<Value, JobError> + Send>;

#[derive(Clone, Debug)]
pub struct ExecutorConfig {
    /// Workers fixos (≥ 1).
    pub workers: usize,
    /// Capacidade da fila de cada categoria (interactive, normal, background).
    pub queue_capacity: [usize; 3],
    /// Créditos por ciclo (interactive, normal, background): em cada ciclo de `Σ` escolhas o
    /// background recebe ≥ 1 vaga se houver fila — **sem starvation**.
    pub credits: [u32; 3],
    /// Quantos jobs terminados continuam consultáveis em memória.
    pub keep_finished: usize,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism().map_or(2, usize::from);
        Self {
            workers: cpus.clamp(2, 4),
            queue_capacity: [256, 1024, 4096],
            credits: [6, 3, 1],
            keep_finished: 256,
        }
    }
}

#[derive(Debug)]
struct JobShared {
    snapshot: Mutex<JobSnapshot>,
    done: Condvar,
    token: CancelToken,
    progress: Arc<ProgressCell>,
    /// O cancelamento veio do desligamento do executor (⇒ `interrupted`, não `cancelled`).
    interrupt: AtomicBool,
}

impl JobShared {
    fn view(&self) -> JobSnapshot {
        let mut s = lock(&self.snapshot).clone();
        if !s.state.is_terminal() {
            s.progress = Progress {
                done: self.progress.done.load(Ordering::SeqCst),
                total: self.progress.total.load(Ordering::SeqCst),
            };
        }
        s
    }
}

struct Entry {
    shared: Arc<JobShared>,
    f: JobFn,
}

struct Inner {
    queues: [VecDeque<Entry>; 3],
    credits_left: [u32; 3],
    jobs: HashMap<JobId, Arc<JobShared>>,
    finished: VecDeque<JobId>,
    dedup: HashMap<String, JobId>,
    shutdown: bool,
}

struct Core {
    inner: Mutex<Inner>,
    wake: Condvar,
    cfg: ExecutorConfig,
    sink: Option<Arc<dyn JobSink>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // um job que entrou em pânico não pode envenenar o executor
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn new_id() -> JobId {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() & 0xFFFF_FFFF_FFFF_FFFF);
    JobId(format!(
        "job_{nanos:016x}{:04x}",
        COUNTER.fetch_add(1, Ordering::SeqCst) & 0xFFFF
    ))
}

impl Core {
    fn record(&self, snap: &JobSnapshot) {
        if let Some(s) = &self.sink {
            s.record(snap);
        }
    }

    /// Escolhe o próximo job: interactive > normal > background **dentro dos créditos do ciclo**.
    fn pick(&self, inner: &mut Inner) -> Option<Entry> {
        if inner.queues.iter().all(VecDeque::is_empty) {
            return None;
        }
        for attempt in 0..2 {
            for p in Priority::ALL {
                let i = p.index();
                if inner.credits_left[i] > 0 && !inner.queues[i].is_empty() {
                    inner.credits_left[i] -= 1;
                    return inner.queues[i].pop_front();
                }
            }
            if attempt == 0 {
                inner.credits_left = self.cfg.credits;
            }
        }
        None
    }

    fn finish(&self, shared: &Arc<JobShared>, key: Option<&str>, id: &JobId) {
        {
            let mut inner = lock(&self.inner);
            if let Some(k) = key
                && inner.dedup.get(k) == Some(id)
            {
                inner.dedup.remove(k);
            }
            inner.finished.push_back(id.clone());
            while inner.finished.len() > self.cfg.keep_finished {
                if let Some(old) = inner.finished.pop_front() {
                    inner.jobs.remove(&old);
                }
            }
        }
        shared.done.notify_all();
    }

    fn run(self: &Arc<Self>, entry: Entry) {
        let Entry { shared, f } = entry;
        let (id, key) = {
            let mut s = lock(&shared.snapshot);
            s.state = JobState::Running;
            s.started_ms = Some(now_ms());
            (s.id.clone(), s.dedup_key.clone())
        };
        self.record(&shared.view());
        let last = Mutex::new(Instant::now());
        let core = Arc::clone(self);
        let sh2 = Arc::clone(&shared);
        let ctx = JobCtx {
            id: id.clone(),
            token: shared.token.clone(),
            progress: Arc::clone(&shared.progress),
            on_progress: Box::new(move || {
                // persistência de progresso com limite de taxa (≤ 4/s)
                let mut l = lock(&last);
                if l.elapsed() >= Duration::from_millis(250) {
                    *l = Instant::now();
                    drop(l);
                    core.record(&sh2.view());
                }
            }),
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| f(&ctx)));
        drop(ctx);
        {
            let mut s = lock(&shared.snapshot);
            s.finished_ms = Some(now_ms());
            s.progress = Progress {
                done: shared.progress.done.load(Ordering::SeqCst),
                total: shared.progress.total.load(Ordering::SeqCst),
            };
            match outcome {
                Ok(Ok(v)) => {
                    s.state = JobState::Completed;
                    s.result = Some(v);
                }
                Ok(Err(e)) if e.is_cancelled() => {
                    s.state = if shared.interrupt.load(Ordering::SeqCst) {
                        JobState::Interrupted
                    } else {
                        JobState::Cancelled
                    };
                    s.error = Some(e);
                }
                Ok(Err(e)) => {
                    s.state = JobState::Failed;
                    s.error = Some(e);
                }
                Err(_) => {
                    s.state = JobState::Failed;
                    s.error = Some(JobError::new(CODE_PANICKED, "the job panicked"));
                }
            }
        }
        self.record(&shared.view());
        self.finish(&shared, key.as_deref(), &id);
    }
}

fn worker(core: &Arc<Core>) {
    loop {
        let entry = {
            let mut inner = lock(&core.inner);
            loop {
                if let Some(e) = core.pick(&mut inner) {
                    break Some(e);
                }
                if inner.shutdown {
                    break None;
                }
                inner = core
                    .wake
                    .wait(inner)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        match entry {
            Some(e) => core.run(e),
            None => return,
        }
    }
}

/// Resultado de `submit`: o handle e se o job foi **deduplicado** (já havia um ativo igual).
#[derive(Debug)]
pub struct Submitted {
    pub handle: JobHandle,
    pub deduplicated: bool,
}

#[derive(Clone, Debug)]
pub struct JobHandle {
    shared: Arc<JobShared>,
}

impl JobHandle {
    pub fn id(&self) -> JobId {
        lock(&self.shared.snapshot).id.clone()
    }

    pub fn snapshot(&self) -> JobSnapshot {
        self.shared.view()
    }

    pub fn is_done(&self) -> bool {
        lock(&self.shared.snapshot).state.is_terminal()
    }

    /// Bloqueia até o job terminar.
    pub fn wait(&self) -> JobSnapshot {
        let mut s = lock(&self.shared.snapshot);
        while !s.state.is_terminal() {
            s = self
                .shared
                .done
                .wait(s)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        drop(s);
        self.shared.view()
    }

    pub fn wait_timeout(&self, d: Duration) -> Option<JobSnapshot> {
        let deadline = Instant::now() + d;
        let mut s = lock(&self.shared.snapshot);
        while !s.state.is_terminal() {
            let left = deadline.checked_duration_since(Instant::now())?;
            let (g, to) = self
                .shared
                .done
                .wait_timeout(s, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            s = g;
            if to.timed_out() && !s.state.is_terminal() {
                return None;
            }
        }
        drop(s);
        Some(self.shared.view())
    }

    pub fn token(&self) -> CancelToken {
        self.shared.token.clone()
    }
}

pub struct Executor {
    core: Arc<Core>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl core::fmt::Debug for Executor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Executor")
            .field("workers", &self.core.cfg.workers)
            .finish_non_exhaustive()
    }
}

impl Executor {
    pub fn new(cfg: ExecutorConfig, sink: Option<Arc<dyn JobSink>>) -> Self {
        let cfg = ExecutorConfig {
            workers: cfg.workers.max(1),
            credits: [
                cfg.credits[0].max(1),
                cfg.credits[1].max(1),
                cfg.credits[2].max(1),
            ],
            ..cfg
        };
        let core = Arc::new(Core {
            inner: Mutex::new(Inner {
                queues: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
                credits_left: cfg.credits,
                jobs: HashMap::new(),
                finished: VecDeque::new(),
                dedup: HashMap::new(),
                shutdown: false,
            }),
            wake: Condvar::new(),
            cfg,
            sink,
        });
        let handles = (0..core.cfg.workers)
            .filter_map(|n| {
                let c = Arc::clone(&core);
                std::thread::Builder::new()
                    .name(format!("capia-job-{n}"))
                    .spawn(move || worker(&c))
                    .ok()
            })
            .collect();
        Self {
            core,
            workers: Mutex::new(handles),
        }
    }

    pub fn config(&self) -> &ExecutorConfig {
        &self.core.cfg
    }

    pub fn submit<F>(&self, spec: JobSpec, f: F) -> Result<Submitted, SubmitError>
    where
        F: FnOnce(&JobCtx) -> Result<Value, JobError> + Send + 'static,
    {
        let shared;
        {
            let mut inner = lock(&self.core.inner);
            if inner.shutdown {
                return Err(SubmitError::ShuttingDown);
            }
            if let Some(key) = &spec.dedup_key
                && let Some(existing) = inner
                    .dedup
                    .get(key)
                    .and_then(|id| inner.jobs.get(id))
                    .filter(|s| !lock(&s.snapshot).state.is_terminal())
            {
                return Ok(Submitted {
                    handle: JobHandle {
                        shared: Arc::clone(existing),
                    },
                    deduplicated: true,
                });
            }
            let q = spec.priority.index();
            let cap = self.core.cfg.queue_capacity[q];
            if inner.queues[q].len() >= cap {
                return Err(SubmitError::QueueFull {
                    priority: spec.priority,
                    capacity: cap,
                });
            }
            let id = new_id();
            shared = Arc::new(JobShared {
                snapshot: Mutex::new(JobSnapshot {
                    id: id.clone(),
                    kind: spec.kind,
                    priority: spec.priority,
                    state: JobState::Queued,
                    progress: Progress::default(),
                    dedup_key: spec.dedup_key.clone(),
                    label: spec.label,
                    params: spec.params,
                    result: None,
                    error: None,
                    created_ms: now_ms(),
                    started_ms: None,
                    finished_ms: None,
                }),
                done: Condvar::new(),
                token: CancelToken::new(),
                progress: Arc::new(ProgressCell::default()),
                interrupt: AtomicBool::new(false),
            });
            if let Some(key) = spec.dedup_key {
                inner.dedup.insert(key, id.clone());
            }
            inner.jobs.insert(id, Arc::clone(&shared));
            inner.queues[q].push_back(Entry {
                shared: Arc::clone(&shared),
                f: Box::new(f),
            });
        }
        self.core.record(&shared.view());
        self.core.wake.notify_one();
        Ok(Submitted {
            handle: JobHandle { shared },
            deduplicated: false,
        })
    }

    pub fn get(&self, id: &JobId) -> Option<JobSnapshot> {
        lock(&self.core.inner).jobs.get(id).map(|s| s.view())
    }

    pub fn handle(&self, id: &JobId) -> Option<JobHandle> {
        lock(&self.core.inner).jobs.get(id).map(|s| JobHandle {
            shared: Arc::clone(s),
        })
    }

    /// Jobs conhecidos (ativos e os últimos terminados), mais antigos primeiro.
    pub fn list(&self) -> Vec<JobSnapshot> {
        let inner = lock(&self.core.inner);
        let mut v: Vec<JobSnapshot> = inner.jobs.values().map(|s| s.view()).collect();
        drop(inner);
        v.sort_by(|a, b| (a.created_ms, &a.id).cmp(&(b.created_ms, &b.id)));
        v
    }

    /// Cancela: job na fila sai da fila na hora (`cancelled`); job em execução recebe o token
    /// (quem o repassa ao processo filho o mata). `false` se já terminou ou é desconhecido.
    pub fn cancel(&self, id: &JobId) -> bool {
        let mut inner = lock(&self.core.inner);
        let Some(shared) = inner.jobs.get(id).cloned() else {
            return false;
        };
        let q_pos = Priority::ALL.iter().find_map(|p| {
            inner.queues[p.index()]
                .iter()
                .position(|e| lock(&e.shared.snapshot).id == *id)
                .map(|pos| (p.index(), pos))
        });
        if let Some((q, pos)) = q_pos {
            inner.queues[q].remove(pos);
            drop(inner);
            let key = {
                let mut s = lock(&shared.snapshot);
                s.state = JobState::Cancelled;
                s.finished_ms = Some(now_ms());
                s.error = Some(JobError::cancelled());
                s.dedup_key.clone()
            };
            self.core.record(&shared.view());
            self.core.finish(&shared, key.as_deref(), id);
            return true;
        }
        drop(inner);
        if lock(&shared.snapshot).state.is_terminal() {
            return false;
        }
        shared.token.cancel();
        true
    }

    /// Desliga: o que está na fila vira `interrupted`; o que roda recebe cancelamento (e termina
    /// `interrupted`); espera os workers. Idempotente.
    pub fn shutdown(&self) {
        let (queued, running): (Vec<Arc<JobShared>>, Vec<Arc<JobShared>>) = {
            let mut inner = lock(&self.core.inner);
            inner.shutdown = true;
            let mut queued = Vec::new();
            for q in &mut inner.queues {
                queued.extend(q.drain(..).map(|e| e.shared));
            }
            let running = inner
                .jobs
                .values()
                .filter(|s| lock(&s.snapshot).state == JobState::Running)
                .cloned()
                .collect();
            (queued, running)
        };
        for s in &running {
            s.interrupt.store(true, Ordering::SeqCst);
            s.token.cancel();
        }
        for s in queued {
            let (id, key) = {
                let mut g = lock(&s.snapshot);
                g.state = JobState::Interrupted;
                g.finished_ms = Some(now_ms());
                (g.id.clone(), g.dedup_key.clone())
            };
            self.core.record(&s.view());
            self.core.finish(&s, key.as_deref(), &id);
        }
        self.core.wake.notify_all();
        let handles: Vec<JoinHandle<()>> = lock(&self.workers).drain(..).collect();
        for h in handles {
            let _ = h.join();
        }
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        self.shutdown();
    }
}
