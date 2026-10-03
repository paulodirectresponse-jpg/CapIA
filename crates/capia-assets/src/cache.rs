//! Cache derivado (ADR-049): chave determinística pelo **conteúdo**, diretório separado do
//! projeto, escrita atômica, **descartável** — nada essencial vive só aqui.

use crate::error::{AssetError, AssetErrorCode};
use crate::hash::{ContentHash, hash_file};
use crate::record::{AssetKind, AssetRecord};
use capia_media::{MediaErrorCode, MediaToolchain, ThumbnailRequest, extract_frame_png};
use capia_time::Ticks;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CacheKey {
    op: String,
    /// Primeiros 16 hex do hash do CONTEÚDO: agrupa as entradas de um asset (GC/invalidação).
    content16: String,
    hex: String,
}

fn bad(msg: &str) -> AssetError {
    AssetError::new(AssetErrorCode::AssetPathInvalid, msg.to_owned())
}

fn io_err(what: &str, e: std::io::Error) -> AssetError {
    AssetError::new(AssetErrorCode::AssetIo, format!("{what}: {e}"))
}

fn field(h: &mut Sha256, s: &str) {
    // prefixado por tamanho: "ab"+"c" ≠ "a"+"bc"
    h.update((s.len() as u64).to_le_bytes());
    h.update(s.as_bytes());
}

impl CacheKey {
    /// `op` e `ext` viram nomes de diretório/arquivo: só `[a-z0-9_-]`.
    pub fn new(
        content: &ContentHash,
        op: &str,
        params: &[(&str, &str)],
        producer: &str,
    ) -> Result<Self, AssetError> {
        if op.is_empty()
            || op.len() > 32
            || !op
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        {
            return Err(bad("invalid cache operation name"));
        }
        let mut sorted: Vec<_> = params.to_vec();
        sorted.sort_unstable();
        let mut h = Sha256::new();
        field(&mut h, "capia-cache-v2");
        field(&mut h, content.as_str());
        field(&mut h, op);
        field(&mut h, producer);
        for (k, v) in sorted {
            field(&mut h, k);
            field(&mut h, v);
        }
        let digest = h.finalize();
        let hex = digest.iter().fold(String::new(), |mut s, b| {
            use core::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        });
        Ok(Self {
            op: op.to_owned(),
            content16: content.hex()[..16].to_owned(),
            hex,
        })
    }

    pub fn hex(&self) -> &str {
        &self.hex
    }

    pub fn op(&self) -> &str {
        &self.op
    }

    pub fn content16(&self) -> &str {
        &self.content16
    }
}

/// Resultado de [`CacheDir::produce`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Produced {
    pub path: PathBuf,
    /// `true` se já existia uma entrada **válida** (nada foi gerado).
    pub hit: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacheUsage {
    pub files: u64,
    pub bytes: u64,
    /// op → (arquivos, bytes)
    pub by_op: BTreeMap<String, (u64, u64)>,
    /// Sobras de produção interrompida (`.tmp/`).
    pub temp_files: u64,
    pub temp_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    pub removed_files: u64,
    pub removed_bytes: u64,
}

/// Diretório de cache. Apagá-lo inteiro é sempre seguro.
#[derive(Clone, Debug)]
pub struct CacheDir {
    root: PathBuf,
}

static TMP: AtomicU64 = AtomicU64::new(0);
/// Chaves em produção neste processo (+ condvar para acordar quem espera).
static HELD: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
static HELD_CV: Condvar = Condvar::new();
/// Lock de disco abandonado há mais que isto é considerado de um processo morto.
const STALE_LOCK: Duration = Duration::from_secs(600);

/// Exclusão por chave: em-processo (mutex) + arquivo `.lock` (entre processos).
#[derive(Debug)]
pub struct KeyLock {
    key: PathBuf,
    file: PathBuf,
}

impl Drop for KeyLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        held.remove(&self.key);
        HELD_CV.notify_all();
    }
}

impl CacheDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Padrão: `<projeto>.capia-cache/` ao lado do arquivo do projeto (ADR-049 §3).
    pub fn beside_project(project: &Path) -> Self {
        let mut name = project.file_name().unwrap_or_default().to_os_string();
        name.push("-cache");
        Self::new(project.with_file_name(name))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_for(&self, key: &CacheKey, ext: &str) -> Result<PathBuf, AssetError> {
        if ext.is_empty() || ext.len() > 8 || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(bad("invalid cache extension"));
        }
        Ok(self
            .root
            .join(key.op())
            .join(key.content16())
            .join(format!("{}.{ext}", key.hex)))
    }

    fn temp_dir(&self) -> PathBuf {
        self.root.join(".tmp")
    }

    /// Caminho de um arquivo temporário novo (mesmo volume do cache ⇒ `rename` atômico).
    pub fn temp_path(&self, ext: &str) -> Result<PathBuf, AssetError> {
        if ext.is_empty() || ext.len() > 8 || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(bad("invalid cache extension"));
        }
        let dir = self.temp_dir();
        std::fs::create_dir_all(&dir).map_err(|e| io_err("cannot create the cache temp dir", e))?;
        Ok(dir.join(format!(
            "{}-{}.{ext}",
            std::process::id(),
            TMP.fetch_add(1, Ordering::SeqCst)
        )))
    }

    /// Entrada existente (arquivo regular) ou `None`. **Não valida o conteúdo**: para artefatos
    /// com formato próprio use [`CacheDir::get_valid`].
    pub fn get(&self, key: &CacheKey, ext: &str) -> Option<PathBuf> {
        let p = self.path_for(key, ext).ok()?;
        std::fs::metadata(&p).ok()?.is_file().then_some(p)
    }

    /// Entrada existente **e válida**; uma entrada que não passa na validação (truncada, checksum
    /// errado, versão antiga) é apagada e conta como *miss*.
    pub fn get_valid(
        &self,
        key: &CacheKey,
        ext: &str,
        validate: &dyn Fn(&Path) -> Result<(), AssetError>,
    ) -> Option<PathBuf> {
        let p = self.get(key, ext)?;
        if validate(&p).is_ok() {
            Some(p)
        } else {
            let _ = std::fs::remove_file(&p);
            None
        }
    }

    /// Adquire o lock da chave (espera até `cancel()` ou 10 min). Duas produções simultâneas da
    /// mesma chave se serializam; a segunda encontra a entrada pronta (veja `produce`).
    pub fn lock(
        &self,
        key: &CacheKey,
        ext: &str,
        cancel: &dyn Fn() -> bool,
    ) -> Result<KeyLock, AssetError> {
        let final_path = self.path_for(key, ext)?;
        {
            let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
            while held.contains(&final_path) {
                if cancel() {
                    return Err(AssetError::new(
                        AssetErrorCode::AssetCancelled,
                        "cancelled while waiting for the cache lock",
                    ));
                }
                held = HELD_CV
                    .wait_timeout(held, Duration::from_millis(10))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            held.insert(final_path.clone());
        }
        // a partir daqui o `KeyLock` (mesmo em erro) solta a chave em-processo
        let mut lock = KeyLock {
            key: final_path.clone(),
            file: PathBuf::new(),
        };
        let lock_dir = self.root.join(".locks");
        std::fs::create_dir_all(&lock_dir).map_err(|e| io_err("cannot create the lock dir", e))?;
        let file = lock_dir.join(format!("{}.lock", key.hex));
        let started = Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)
            {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", std::process::id());
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&file)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > STALE_LOCK);
                    if stale {
                        let _ = std::fs::remove_file(&file);
                        continue;
                    }
                    if cancel() || started.elapsed() > STALE_LOCK {
                        return Err(AssetError::new(
                            AssetErrorCode::AssetCancelled,
                            "cancelled while waiting for the cache lock",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(io_err("cannot take the cache lock", e)),
            }
        }
        lock.file = file;
        Ok(lock)
    }

    /// Produz (ou reaproveita) a entrada de `key`, **atomicamente**:
    ///
    /// Passos: toma o lock da chave; se já há entrada válida devolve `hit`; senão `write(temp)`,
    /// `validate(temp)`, `fsync` e `rename` para o destino final. Qualquer falha ou cancelamento
    /// apaga o temporário e **nunca** publica o final; um final anterior bom nunca é tocado antes
    /// do `rename`.
    pub fn produce(
        &self,
        key: &CacheKey,
        ext: &str,
        cancel: &dyn Fn() -> bool,
        validate: &dyn Fn(&Path) -> Result<(), AssetError>,
        write: &dyn Fn(&Path) -> Result<(), AssetError>,
    ) -> Result<Produced, AssetError> {
        let final_path = self.path_for(key, ext)?;
        let _lock = self.lock(key, ext, cancel)?;
        if let Some(path) = self.get_valid(key, ext, validate) {
            return Ok(Produced { path, hit: true });
        }
        let tmp = self.temp_path(ext)?;
        let cleanup = |e: AssetError| {
            let _ = std::fs::remove_file(&tmp);
            e
        };
        write(&tmp).map_err(cleanup)?;
        if cancel() {
            return Err(cleanup(AssetError::new(
                AssetErrorCode::AssetCancelled,
                "cancelled before publishing the cache entry",
            )));
        }
        validate(&tmp).map_err(cleanup)?;
        std::fs::File::open(&tmp)
            .and_then(|f| f.sync_all())
            .map_err(|e| cleanup(io_err("cannot flush the cache entry", e)))?;
        let dir = final_path
            .parent()
            .ok_or_else(|| cleanup(bad("invalid cache path")))?;
        std::fs::create_dir_all(dir)
            .map_err(|e| cleanup(io_err("cannot create the cache directory", e)))?;
        std::fs::rename(&tmp, &final_path)
            .map_err(|e| cleanup(io_err("cannot publish the cache entry", e)))?;
        Ok(Produced {
            path: final_path,
            hit: false,
        })
    }

    /// Escrita atômica de bytes prontos (sem validação própria).
    pub fn put(&self, key: &CacheKey, ext: &str, bytes: &[u8]) -> Result<PathBuf, AssetError> {
        self.produce(key, ext, &|| false, &|_| Ok(()), &|tmp| {
            std::fs::write(tmp, bytes).map_err(|e| io_err("cache write failed", e))
        })
        .map(|p| p.path)
    }

    /// Uso do cache (nº de arquivos/bytes por operação e sobras temporárias).
    pub fn usage(&self) -> Result<CacheUsage, AssetError> {
        let mut u = CacheUsage::default();
        let Ok(top) = std::fs::read_dir(&self.root) else {
            return Ok(u);
        };
        for op in top.flatten() {
            let name = op.file_name().to_string_lossy().into_owned();
            if name == ".locks" {
                continue;
            }
            if name == ".tmp" {
                for f in walk_files(&op.path()) {
                    u.temp_files += 1;
                    u.temp_bytes += f.1;
                }
                continue;
            }
            for f in walk_files(&op.path()) {
                u.files += 1;
                u.bytes += f.1;
                let e = u.by_op.entry(name.clone()).or_default();
                e.0 += 1;
                e.1 += f.1;
            }
        }
        Ok(u)
    }

    /// Remove as entradas cujo conteúdo (prefixo de 16 hex) **não** está em `live`. Entradas de
    /// assets ainda no projeto ficam. Idempotente; o projeto continua válido.
    pub fn remove_unused(&self, live: &HashSet<String>) -> Result<GcReport, AssetError> {
        let mut r = GcReport::default();
        let Ok(top) = std::fs::read_dir(&self.root) else {
            return Ok(r);
        };
        for op in top.flatten() {
            let name = op.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !op.path().is_dir() {
                continue;
            }
            let Ok(groups) = std::fs::read_dir(op.path()) else {
                continue;
            };
            for g in groups.flatten() {
                let content = g.file_name().to_string_lossy().into_owned();
                if live.contains(&content) {
                    continue;
                }
                for f in walk_files(&g.path()) {
                    r.removed_files += 1;
                    r.removed_bytes += f.1;
                }
                let _ = std::fs::remove_dir_all(g.path());
            }
        }
        Ok(r)
    }

    /// Invalida **tudo** derivado de um conteúdo (force relink, arquivo modificado).
    pub fn invalidate_content(&self, content: &ContentHash) -> Result<GcReport, AssetError> {
        let c16 = &content.hex()[..16];
        let mut r = GcReport::default();
        let Ok(top) = std::fs::read_dir(&self.root) else {
            return Ok(r);
        };
        for op in top.flatten() {
            if op.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let dir = op.path().join(c16);
            for f in walk_files(&dir) {
                r.removed_files += 1;
                r.removed_bytes += f.1;
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
        Ok(r)
    }

    /// Apaga temporários de produções interrompidas mais velhos que `older_than`.
    pub fn sweep_temp(&self, older_than: Duration) -> u64 {
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(self.temp_dir()) {
            for e in rd.flatten() {
                let old = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|a| a >= older_than);
                if old && std::fs::remove_file(e.path()).is_ok() {
                    n += 1;
                }
            }
        }
        n
    }

    /// Apaga todo o cache (idempotente).
    pub fn clear(&self) -> Result<(), AssetError> {
        match std::fs::remove_dir_all(&self.root) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AssetError::new(
                AssetErrorCode::AssetIo,
                format!("cannot clear the cache: {e}"),
            )),
        }
    }
}

/// (caminho, bytes) de todos os arquivos regulares sob `dir` (sem seguir symlinks).
fn walk_files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                stack.push(e.path());
            } else if t.is_file() {
                out.push((e.path(), e.metadata().map(|m| m.len()).unwrap_or(0)));
            }
        }
    }
    out
}

/// Miniatura de um frame (ADR-049 §4), como entrada do cache chaveada por conteúdo. Em cache
/// ausente confere o hash do arquivo **antes** de gerar: nunca guarda, sob o hash vigente, o
/// quadro de um arquivo diferente.
pub fn ensure_thumbnail(
    toolchain: &MediaToolchain,
    record: &AssetRecord,
    file: &Path,
    at: Ticks,
    max_dim: u32,
    cache: &CacheDir,
) -> Result<PathBuf, AssetError> {
    if record.kind == AssetKind::Audio {
        return Err(AssetError::new(
            AssetErrorCode::Media(MediaErrorCode::MediaUnsupportedFormat),
            "audio assets have no video frame to thumbnail",
        ));
    }
    // imagens não têm linha do tempo: sempre o quadro 0
    let at = if record.kind == AssetKind::Image {
        Ticks::ZERO
    } else {
        at
    };
    let producer = format!("thumb/1;{}", toolchain.version);
    let key = CacheKey::new(
        &record.content_hash,
        "thumbnail",
        &[("at", &at.0.to_string()), ("max", &max_dim.to_string())],
        &producer,
    )?;
    if let Some(p) = cache.get(&key, "png") {
        return Ok(p);
    }
    let digest = hash_file(file)?;
    if digest.hash != record.content_hash {
        return Err(AssetError::new(
            AssetErrorCode::AssetHashMismatch,
            "the file no longer has the content of this asset; no thumbnail generated",
        ));
    }
    let png = extract_frame_png(toolchain, file, ThumbnailRequest { at, max_dim })?;
    cache.put(&key, "png", &png)
}
