//! Asset Gateway (PHASE5_MEMORY_GATEWAY §10..§21, §28, §30..§33, §39): **única** fronteira de
//! aquisição externa que não seja provider de IA. O Brain não tem `http.get`: pede
//! `gateway.search`/`gateway.fetch` por *needs* tipados e o orquestrador chama adapters.
//!
//! * adapters declaram hosts permitidos, capacidades e o mapeamento de proveniência;
//! * download em *staging* → hash/tamanho/tipo → proveniência → import pelo sistema de assets
//!   (atômico) — falha nunca deixa asset parcial como válido;
//! * metadados de fontes externas são **dados não confiáveis** (nunca instrução, nunca memória);
//! * licença desconhecida/restrita é decidida pela política (padrão seguro: aprovação);
//! * adapter desligado ⇒ o app continua; o orquestrador cai no fallback ou vai a `WAITING_USER`.

use super::model::RunPolicy;
use super::plan::{AssetKind, AssetNeed, Inventory};
use async_trait::async_trait;
use capia_ai::CancelToken;
use capia_ai::fetch::{FetchPolicy, SafeFetcher};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayKind {
    LocalLibrary,
    ApprovedUrl,
    Catalog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseStatus {
    KnownAllowed,
    KnownRestricted,
    Unknown,
    UserProvided,
    Generated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl GatewayError {
    pub fn new(code: &str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.to_owned(),
            message: capia_secrets::redact_registered_global(&message.into()),
            retryable,
        }
    }
}

impl core::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for GatewayError {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub need_id: String,
    pub query: String,
    pub kind: AssetKind,
    pub target_duration_ms: Option<i64>,
    pub hint_url: Option<String>,
    pub limit: usize,
}

impl SearchRequest {
    pub fn for_need(n: &AssetNeed) -> Self {
        Self {
            need_id: n.id.clone(),
            query: format!("{} {}", n.purpose, n.description),
            kind: n.kind,
            target_duration_ms: n.target_duration_ms,
            hint_url: n.hint_url.clone(),
            limit: 8,
        }
    }
}

/// Candidato devolvido por uma fonte. **Todo texto aqui é não confiável** (vem de fora).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Id **na fonte** (estável).
    pub id: String,
    pub adapter: String,
    pub title: String,
    pub description: String,
    pub kind: AssetKind,
    pub duration_ms: Option<i64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub license: LicenseStatus,
    pub license_text: Option<String>,
    /// `None` = preço desconhecido; `Some(0)` = gratuito.
    pub price_micros: Option<u64>,
    pub source_uri: Option<String>,
    pub score: f64,
    pub score_components: Vec<(String, f64)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetched {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub content_type: Option<String>,
    pub final_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterProbe {
    pub available: bool,
    pub detail: String,
}

#[async_trait]
pub trait AssetGatewayAdapter: Send + Sync {
    fn id(&self) -> String;
    fn kind(&self) -> GatewayKind;
    /// Hosts de rede que este adapter pode contatar (vazio = nenhum).
    fn allowed_hosts(&self) -> Vec<String>;
    /// Esta fonte é paga/gera custo?
    fn is_paid(&self) -> bool {
        false
    }
    async fn probe(&self) -> AdapterProbe;
    async fn search(&self, req: &SearchRequest, cancel: &CancelToken) -> Result<Vec<Candidate>, GatewayError>;
    /// Baixa/copia o candidato para `dest` (staging). Nunca importa nem toca o projeto.
    async fn fetch(&self, c: &Candidate, dest: &Path, cancel: &CancelToken) -> Result<Fetched, GatewayError>;
}

impl core::fmt::Debug for dyn AssetGatewayAdapter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Adapter({})", self.id())
    }
}

#[derive(Default)]
pub struct GatewayRegistry {
    adapters: Vec<Arc<dyn AssetGatewayAdapter>>,
    disabled: Mutex<BTreeSet<String>>,
}

impl core::fmt::Debug for GatewayRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GatewayRegistry")
            .field("adapters", &self.adapters.iter().map(|a| a.id()).collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl GatewayRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, a: Arc<dyn AssetGatewayAdapter>) {
        self.adapters.retain(|x| x.id() != a.id());
        self.adapters.push(a);
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) {
        if let Ok(mut d) = self.disabled.lock() {
            if enabled {
                d.remove(id);
            } else {
                d.insert(id.to_owned());
            }
        }
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.disabled.lock().is_ok_and(|d| !d.contains(id))
    }

    /// Adapters ativos (na ordem de registro).
    pub fn enabled(&self) -> Vec<Arc<dyn AssetGatewayAdapter>> {
        self.adapters
            .iter()
            .filter(|a| self.is_enabled(&a.id()))
            .cloned()
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn AssetGatewayAdapter>> {
        self.adapters.iter().find(|a| a.id() == id).cloned()
    }

    pub fn ids(&self) -> Vec<String> {
        self.adapters.iter().map(|a| a.id()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }
}

// ---- política de licença -----------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseVerdict {
    Allow,
    NeedsApproval,
    Reject,
}

pub fn license_verdict(l: LicenseStatus, p: &RunPolicy) -> LicenseVerdict {
    match l {
        LicenseStatus::KnownAllowed | LicenseStatus::UserProvided | LicenseStatus::Generated => LicenseVerdict::Allow,
        LicenseStatus::Unknown => {
            if p.unknown_license_requires_approval {
                LicenseVerdict::NeedsApproval
            } else {
                LicenseVerdict::Allow
            }
        }
        LicenseStatus::KnownRestricted => {
            if p.reject_restricted_license {
                LicenseVerdict::Reject
            } else {
                LicenseVerdict::NeedsApproval
            }
        }
    }
}

// ---- ranqueamento ------------------------------------------------------------------------------

fn tokens(s: &str) -> BTreeSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(str::to_owned)
        .collect()
}

/// Pontua um candidato para uma *need* (componentes persistidos para explicar a escolha).
pub fn rank_candidate(c: &mut Candidate, n: &AssetNeed) {
    let want = tokens(&format!("{} {}", n.purpose, n.description));
    let have = tokens(&format!("{} {}", c.title, c.description));
    let overlap = want.intersection(&have).count() as f64;
    let semantic = if want.is_empty() { 0.0 } else { overlap / want.len() as f64 };
    let kind = if c.kind == n.kind { 1.0 } else { 0.0 };
    let duration = match (n.target_duration_ms, c.duration_ms) {
        (Some(t), Some(d)) if d >= t => 1.0,
        (Some(t), Some(d)) => d as f64 / t.max(1) as f64,
        _ => 0.5,
    };
    let license = match c.license {
        LicenseStatus::KnownAllowed | LicenseStatus::UserProvided | LicenseStatus::Generated => 1.0,
        LicenseStatus::Unknown => 0.4,
        LicenseStatus::KnownRestricted => 0.0,
    };
    let cost = match c.price_micros {
        Some(0) => 1.0,
        Some(_) => 0.5,
        None => 0.3,
    };
    c.score_components = vec![
        ("semantic".into(), semantic),
        ("kind".into(), kind),
        ("duration".into(), duration),
        ("license".into(), license),
        ("cost".into(), cost),
    ];
    c.score = semantic * 4.0 + kind * 2.0 + duration + license + cost * 0.5;
}

/// Reuso de assets **do projeto**: ranqueia o inventário contra a *need* (antes de adquirir).
pub fn rank_inventory(n: &AssetNeed, inv: &Inventory, exclude: &BTreeSet<String>) -> Vec<(String, f64)> {
    let want = tokens(&format!("{} {}", n.purpose, n.description));
    let mut out: Vec<(String, f64)> = inv
        .assets
        .values()
        .filter(|a| a.online && !exclude.contains(&a.id))
        .filter(|a| match n.kind {
            AssetKind::Video => a.has_video && !a.is_image,
            AssetKind::Image => a.is_image,
            AssetKind::Audio => a.has_audio && !a.has_video,
        })
        .filter_map(|a| {
            let have = tokens(&a.name);
            let hits = want.intersection(&have).count() as f64;
            let dur_ok = match (n.target_duration_ms, a.duration_ticks) {
                (Some(t), Some(d)) => d >= t * super::plan::TICKS_PER_MS,
                _ => true,
            };
            (hits > 0.0 && dur_ok).then(|| (a.id.clone(), hits))
        })
        .collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(core::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
    out
}

// ---- adapters ----------------------------------------------------------------------------------

const MEDIA_EXT: &[&str] = &["mp4", "mov", "mkv", "webm", "m4v", "png", "jpg", "jpeg", "wav", "mp3", "m4a", "aac", "flac"];

fn kind_of_ext(ext: &str) -> AssetKind {
    match ext {
        "png" | "jpg" | "jpeg" => AssetKind::Image,
        "wav" | "mp3" | "m4a" | "aac" | "flac" => AssetKind::Audio,
        _ => AssetKind::Video,
    }
}

fn sha256_file(path: &Path) -> Result<(String, u64), GatewayError> {
    use sha2::{Digest, Sha256};
    use std::io::Read as _;
    let mut f = std::fs::File::open(path).map_err(|e| GatewayError::new("IO", format!("open: {e}"), false))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| GatewayError::new("IO", format!("read: {e}"), false))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        h.update(&buf[..n]);
    }
    let d = h.finalize();
    let mut s = String::from("sha256:");
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    Ok((s, total))
}

/// Biblioteca **local** do usuário (uma pasta): sem rede; o "download" é uma cópia para staging.
#[derive(Debug)]
pub struct LocalLibraryAdapter {
    id: String,
    root: PathBuf,
}

impl LocalLibraryAdapter {
    pub fn new(id: &str, root: PathBuf) -> Self {
        Self { id: id.to_owned(), root }
    }
}

#[async_trait]
impl AssetGatewayAdapter for LocalLibraryAdapter {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn kind(&self) -> GatewayKind {
        GatewayKind::LocalLibrary
    }

    fn allowed_hosts(&self) -> Vec<String> {
        Vec::new()
    }

    async fn probe(&self) -> AdapterProbe {
        AdapterProbe {
            available: self.root.is_dir(),
            detail: if self.root.is_dir() { "ok".into() } else { "library folder not found".into() },
        }
    }

    async fn search(&self, req: &SearchRequest, _c: &CancelToken) -> Result<Vec<Candidate>, GatewayError> {
        let want = tokens(&req.query);
        let mut out = Vec::new();
        let mut stack = vec![(self.root.clone(), 0u8)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten().take(5_000) {
                let p = e.path();
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_symlink() {
                    continue; // nunca segue links para fora da biblioteca
                }
                if ft.is_dir() {
                    if depth < 2 {
                        stack.push((p, depth + 1));
                    }
                    continue;
                }
                let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
                if !MEDIA_EXT.contains(&ext.as_str()) {
                    continue;
                }
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if tokens(stem).intersection(&want).count() == 0 {
                    continue;
                }
                let rel = p.strip_prefix(&self.root).unwrap_or(&p).display().to_string();
                out.push(Candidate {
                    id: rel.clone(),
                    adapter: self.id.clone(),
                    title: stem.to_owned(),
                    description: String::new(),
                    kind: kind_of_ext(&ext),
                    duration_ms: None,
                    width: None,
                    height: None,
                    license: LicenseStatus::UserProvided,
                    license_text: None,
                    price_micros: Some(0),
                    source_uri: Some(format!("library:{rel}")),
                    score: 0.0,
                    score_components: vec![],
                });
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out.truncate(req.limit.max(1));
        Ok(out)
    }

    async fn fetch(&self, c: &Candidate, dest: &Path, _cancel: &CancelToken) -> Result<Fetched, GatewayError> {
        let rel = Path::new(&c.id);
        // anti path-traversal: o id é relativo e não pode escapar da pasta
        if rel.is_absolute() || rel.components().any(|x| matches!(x, std::path::Component::ParentDir)) {
            return Err(GatewayError::new("NOT_ALLOWED", "the library id escapes the library folder", false));
        }
        let src = self.root.join(rel);
        let canon = src.canonicalize().map_err(|e| GatewayError::new("NOT_FOUND", format!("library file: {e}"), false))?;
        let root = self.root.canonicalize().map_err(|e| GatewayError::new("IO", e.to_string(), false))?;
        if !canon.starts_with(&root) {
            return Err(GatewayError::new("NOT_ALLOWED", "the library file is outside the library", false));
        }
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| GatewayError::new("IO", e.to_string(), false))?;
        }
        let part = dest.with_extension("part");
        std::fs::copy(&canon, &part).map_err(|e| GatewayError::new("IO", format!("copy: {e}"), false))?;
        std::fs::rename(&part, dest).map_err(|e| GatewayError::new("IO", e.to_string(), false))?;
        let (sha, bytes) = sha256_file(dest)?;
        Ok(Fetched { path: dest.to_path_buf(), bytes, sha256: sha, content_type: None, final_url: None })
    }
}

/// URL **aprovada**: só hosts da allow-list; não pesquisa (a URL vem da *need*/do usuário).
pub struct ApprovedUrlAdapter {
    id: String,
    fetcher: SafeFetcher,
}

impl core::fmt::Debug for ApprovedUrlAdapter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ApprovedUrlAdapter").field("id", &self.id).finish_non_exhaustive()
    }
}

impl ApprovedUrlAdapter {
    pub fn new(id: &str, policy: FetchPolicy) -> Result<Self, GatewayError> {
        let fetcher = SafeFetcher::new(policy).map_err(|e| GatewayError::new("CONFIG", e.message, false))?;
        Ok(Self { id: id.to_owned(), fetcher })
    }
}

#[async_trait]
impl AssetGatewayAdapter for ApprovedUrlAdapter {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn kind(&self) -> GatewayKind {
        GatewayKind::ApprovedUrl
    }

    fn allowed_hosts(&self) -> Vec<String> {
        self.fetcher.policy().allowed_hosts.clone()
    }

    async fn probe(&self) -> AdapterProbe {
        AdapterProbe { available: true, detail: "no network call on probe".into() }
    }

    async fn search(&self, req: &SearchRequest, _c: &CancelToken) -> Result<Vec<Candidate>, GatewayError> {
        let Some(url) = &req.hint_url else { return Ok(Vec::new()) };
        // a URL é validada ANTES de virar candidato (esquema/host/SSRF)
        self.fetcher
            .check_url(url)
            .map_err(|e| GatewayError::new("NOT_ALLOWED", e.message, false))?;
        Ok(vec![Candidate {
            id: url.clone(),
            adapter: self.id.clone(),
            title: req.need_id.clone(),
            description: req.query.clone(),
            kind: req.kind,
            duration_ms: req.target_duration_ms,
            width: None,
            height: None,
            license: LicenseStatus::Unknown,
            license_text: None,
            price_micros: Some(0),
            source_uri: Some(url.clone()),
            score: 0.0,
            score_components: vec![],
        }])
    }

    async fn fetch(&self, c: &Candidate, dest: &Path, cancel: &CancelToken) -> Result<Fetched, GatewayError> {
        let r = self
            .fetcher
            .download(&c.id, dest, cancel)
            .await
            .map_err(|e| GatewayError::new(e.code.as_str(), e.message, matches!(e.code, capia_ai::ErrorCode::ProviderUnavailable)))?;
        Ok(Fetched {
            path: r.path,
            bytes: r.bytes,
            sha256: r.sha256,
            content_type: r.content_type,
            final_url: Some(r.final_url),
        })
    }
}

/// Catálogo roteirizado (Replay/mock): determinístico, sem rede. Serve ao corpus de autonomia e a
/// qualquer fonte "pesquisável" cujo contrato externo ainda não esteja disponível.
#[derive(Debug)]
pub struct CatalogEntry {
    pub candidate: Candidate,
    /// Conteúdo que o "download" escreve.
    pub payload: Vec<u8>,
}

pub struct ReplayCatalogAdapter {
    id: String,
    paid: bool,
    entries: Vec<CatalogEntry>,
    pub fetch_count: AtomicU32,
    pub search_count: AtomicU32,
    fail_search: Mutex<Option<GatewayError>>,
    fail_fetch: Mutex<Option<GatewayError>>,
}

impl core::fmt::Debug for ReplayCatalogAdapter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReplayCatalogAdapter").field("id", &self.id).finish_non_exhaustive()
    }
}

impl ReplayCatalogAdapter {
    pub fn new(id: &str, entries: Vec<CatalogEntry>) -> Self {
        Self {
            id: id.to_owned(),
            paid: false,
            entries,
            fetch_count: AtomicU32::new(0),
            search_count: AtomicU32::new(0),
            fail_search: Mutex::new(None),
            fail_fetch: Mutex::new(None),
        }
    }

    pub fn paid(mut self) -> Self {
        self.paid = true;
        self
    }

    pub fn fail_next_search(&self, e: GatewayError) {
        if let Ok(mut g) = self.fail_search.lock() {
            *g = Some(e);
        }
    }

    pub fn fail_next_fetch(&self, e: GatewayError) {
        if let Ok(mut g) = self.fail_fetch.lock() {
            *g = Some(e);
        }
    }
}

#[async_trait]
impl AssetGatewayAdapter for ReplayCatalogAdapter {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn kind(&self) -> GatewayKind {
        GatewayKind::Catalog
    }

    fn allowed_hosts(&self) -> Vec<String> {
        Vec::new()
    }

    fn is_paid(&self) -> bool {
        self.paid
    }

    async fn probe(&self) -> AdapterProbe {
        AdapterProbe { available: true, detail: "replay catalog".into() }
    }

    async fn search(&self, req: &SearchRequest, cancel: &CancelToken) -> Result<Vec<Candidate>, GatewayError> {
        self.search_count.fetch_add(1, Ordering::SeqCst);
        if cancel.is_cancelled() {
            return Err(GatewayError::new("CANCELLED", "cancelled", false));
        }
        if let Some(e) = self.fail_search.lock().ok().and_then(|mut g| g.take()) {
            return Err(e);
        }
        let want = tokens(&req.query);
        let mut v: Vec<Candidate> = self
            .entries
            .iter()
            .map(|e| e.candidate.clone())
            .filter(|c| c.kind == req.kind)
            .filter(|c| want.is_empty() || !tokens(&format!("{} {}", c.title, c.description)).is_disjoint(&want))
            .collect();
        v.truncate(req.limit.max(1));
        Ok(v)
    }

    async fn fetch(&self, c: &Candidate, dest: &Path, cancel: &CancelToken) -> Result<Fetched, GatewayError> {
        self.fetch_count.fetch_add(1, Ordering::SeqCst);
        if cancel.is_cancelled() {
            return Err(GatewayError::new("CANCELLED", "cancelled", false));
        }
        if let Some(e) = self.fail_fetch.lock().ok().and_then(|mut g| g.take()) {
            return Err(e);
        }
        let entry = self
            .entries
            .iter()
            .find(|e| e.candidate.id == c.id)
            .ok_or_else(|| GatewayError::new("NOT_FOUND", "unknown catalog entry", false))?;
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| GatewayError::new("IO", e.to_string(), false))?;
        }
        let part = dest.with_extension("part");
        std::fs::write(&part, &entry.payload).map_err(|e| GatewayError::new("IO", e.to_string(), false))?;
        std::fs::rename(&part, dest).map_err(|e| GatewayError::new("IO", e.to_string(), false))?;
        let (sha, bytes) = sha256_file(dest)?;
        Ok(Fetched { path: dest.to_path_buf(), bytes, sha256: sha, content_type: Some("video/mp4".into()), final_url: None })
    }
}

/// Registro de proveniência (JSON persistido em `ai_provenance`). Nunca contém segredo.
#[allow(clippy::too_many_arguments)]
pub fn provenance_json(
    kind: &str,
    run_id: &str,
    need_id: &str,
    adapter: &str,
    source_uri: Option<&str>,
    license: LicenseStatus,
    license_text: Option<&str>,
    content_hash: &str,
    cost_micros: Option<u64>,
    approval_ref: Option<&str>,
    extra: Value,
) -> Value {
    json!({
        "kind": kind, "run_id": run_id, "need_id": need_id, "adapter": adapter,
        "source_uri": source_uri, "license": {"status": license, "text": license_text},
        "content_hash": content_hash, "cost_micros": cost_micros, "approval_ref": approval_ref,
        "fetched_ms": crate::records::now_ms(), "extra": extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autonomy::plan::AssetInfo;

    fn need() -> AssetNeed {
        AssetNeed {
            id: "n1".into(),
            kind: AssetKind::Video,
            purpose: "b-roll produto".into(),
            description: "pessoa usando o produto na cozinha".into(),
            source_priority: vec![],
            required: true,
            estimated_cost_micros: None,
            constraints: vec![],
            target_duration_ms: Some(4000),
            hint_url: None,
            status: crate::autonomy::plan::NeedStatus::Open,
            resolved_asset_id: None,
        }
    }

    fn cand(id: &str, title: &str, license: LicenseStatus, price: Option<u64>) -> Candidate {
        Candidate {
            id: id.into(), adapter: "cat".into(), title: title.into(), description: "pessoa cozinha produto".into(),
            kind: AssetKind::Video, duration_ms: Some(6000), width: None, height: None, license,
            license_text: None, price_micros: price, source_uri: None, score: 0.0, score_components: vec![],
        }
    }

    #[test]
    fn license_policy_defaults_are_the_safe_ones() {
        let p = RunPolicy::default();
        assert_eq!(license_verdict(LicenseStatus::KnownAllowed, &p), LicenseVerdict::Allow);
        assert_eq!(license_verdict(LicenseStatus::Generated, &p), LicenseVerdict::Allow);
        assert_eq!(license_verdict(LicenseStatus::Unknown, &p), LicenseVerdict::NeedsApproval);
        assert_eq!(license_verdict(LicenseStatus::KnownRestricted, &p), LicenseVerdict::Reject);
        let lax = RunPolicy { unknown_license_requires_approval: false, reject_restricted_license: false, ..RunPolicy::default() };
        assert_eq!(license_verdict(LicenseStatus::Unknown, &lax), LicenseVerdict::Allow);
        assert_eq!(license_verdict(LicenseStatus::KnownRestricted, &lax), LicenseVerdict::NeedsApproval);
    }

    #[test]
    fn ranking_prefers_semantic_fit_known_license_and_free() {
        let n = need();
        let mut a = cand("a", "pessoa cozinha produto", LicenseStatus::KnownAllowed, Some(0));
        let mut b = cand("b", "paisagem drone", LicenseStatus::KnownAllowed, Some(0));
        b.description = "montanha ao amanhecer".into();
        let mut c = cand("c", "pessoa cozinha produto", LicenseStatus::Unknown, None);
        for x in [&mut a, &mut b, &mut c] {
            rank_candidate(x, &n);
        }
        assert!(a.score > b.score && a.score > c.score);
        assert_eq!(a.score_components.len(), 5, "components are kept to explain the choice");
    }

    #[test]
    fn inventory_reuse_ignores_offline_wrong_kind_and_excluded() {
        let mut inv = Inventory::default();
        let mk = |id: &str, name: &str, online: bool| AssetInfo {
            id: id.into(), name: name.into(), duration_ticks: Some(10_000 * super::super::plan::TICKS_PER_MS),
            has_video: true, has_audio: true, online, is_image: false,
        };
        inv.assets.insert("ok".into(), mk("ok", "cozinha produto.mp4", true));
        inv.assets.insert("off".into(), mk("off", "cozinha produto 2.mp4", false));
        inv.assets.insert("none".into(), mk("none", "praia.mp4", true));
        let r = rank_inventory(&need(), &inv, &BTreeSet::new());
        assert_eq!(r.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), vec!["ok"]);
        let ex: BTreeSet<String> = ["ok".to_owned()].into();
        assert!(rank_inventory(&need(), &inv, &ex).is_empty());
    }

    #[tokio::test]
    async fn local_library_blocks_traversal_and_symlinks_and_copies_with_hash() {
        let d = std::env::temp_dir().join(format!("capia-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("lib")).unwrap();
        std::fs::write(d.join("lib/cozinha produto.mp4"), b"abc123").unwrap();
        std::fs::write(d.join("secret.txt"), b"nope").unwrap();
        let a = LocalLibraryAdapter::new("lib", d.join("lib"));
        let cancel = CancelToken::new();
        let req = SearchRequest { need_id: "n1".into(), query: "cozinha produto".into(), kind: AssetKind::Video, target_duration_ms: None, hint_url: None, limit: 5 };
        let found = a.search(&req, &cancel).await.unwrap();
        assert_eq!(found.len(), 1);
        let got = a.fetch(&found[0], &d.join("stage/x.mp4"), &cancel).await.unwrap();
        assert_eq!(std::fs::read(&got.path).unwrap(), b"abc123");
        assert!(got.sha256.starts_with("sha256:"));
        let mut evil = found[0].clone();
        evil.id = "../secret.txt".into();
        assert_eq!(a.fetch(&evil, &d.join("stage/y.mp4"), &cancel).await.unwrap_err().code, "NOT_ALLOWED");
        evil.id = d.join("secret.txt").display().to_string();
        assert!(a.fetch(&evil, &d.join("stage/z.mp4"), &cancel).await.is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.join("secret.txt"), d.join("lib/cozinha link.mp4")).unwrap();
            let found = a.search(&req, &cancel).await.unwrap();
            assert!(found.iter().all(|c| !c.id.contains("link")), "symlinks are never listed");
        }
    }

    #[tokio::test]
    async fn the_registry_disables_adapters_without_breaking_the_rest() {
        let a: Arc<dyn AssetGatewayAdapter> = Arc::new(ReplayCatalogAdapter::new("cat", vec![]));
        let b: Arc<dyn AssetGatewayAdapter> = Arc::new(LocalLibraryAdapter::new("lib", std::env::temp_dir()));
        let mut reg = GatewayRegistry::new();
        reg.register(a);
        reg.register(b);
        assert_eq!(reg.enabled().len(), 2);
        reg.set_enabled("cat", false);
        assert_eq!(reg.enabled().iter().map(|x| x.id()).collect::<Vec<_>>(), vec!["lib".to_owned()]);
        reg.set_enabled("cat", true);
        assert_eq!(reg.enabled().len(), 2);
    }
}
