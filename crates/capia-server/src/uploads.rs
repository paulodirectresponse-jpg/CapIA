//! Uploads seguros (PHASE6_COMPLETION §8): streaming para staging, teto de tamanho/cota/tempo/
//! concorrência, hash SHA-256 durante a escrita, *sniff* do conteúdo (nunca confiar em filename ou
//! MIME do cliente), deduplicação por conteúdo e `rename` atômico só ao final. O que entra no
//! projeto passa depois pelo sistema de assets (hash + probe + catálogo na mesma transação).

use crate::auth::now_ms;
use crate::core::{CallCtx, Core};
use crate::error::{ApiErr, ApiResult};
use crate::mac::{random_hex, sha256_hex};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;

const CHUNK: usize = 64 * 1024;
const HEAD: usize = 4096;
const MAX_DOC_BYTES: u64 = 32 << 20;
const MAX_IMAGE_BYTES: u64 = 64 << 20;
pub const MAX_INLINE_BYTES: usize = 8 << 20;

/// Nome seguro: só o último segmento, caracteres conservadores, sem ponto inicial, ≤ 120 bytes.
pub fn sanitize_filename(raw: &str) -> String {
    let base = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches('.');
    let mut out: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    out.truncate(120);
    let out = out.trim().to_owned();
    if out.is_empty() || out.chars().all(|c| c == '.' || c == '_') {
        "file".to_owned()
    } else {
        out
    }
}

fn ext(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Classe do conteúdo pelos bytes iniciais (não pelo nome/MIME). `None` = não suportado.
pub fn sniff(head: &[u8], filename: &str) -> Option<&'static str> {
    let e = ext(filename);
    let h = head;
    if h.len() >= 12 && &h[4..8] == b"ftyp" {
        return Some("video");
    }
    if h.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("video");
    }
    if h.len() >= 12 && &h[0..4] == b"RIFF" {
        return match &h[8..12] {
            b"AVI " => Some("video"),
            b"WAVE" => Some("audio"),
            b"WEBP" => Some("image"),
            _ => None,
        };
    }
    if h.starts_with(b"ID3") || (h.len() >= 2 && h[0] == 0xFF && h[1] & 0xE0 == 0xE0) {
        return Some("audio");
    }
    if h.starts_with(b"fLaC") || h.starts_with(b"OggS") {
        return Some("audio");
    }
    if h.starts_with(&[0x89, b'P', b'N', b'G'])
        || h.starts_with(&[0xFF, 0xD8, 0xFF])
        || h.starts_with(b"GIF8")
    {
        return Some("image");
    }
    if h.starts_with(b"%PDF-") {
        return Some("document");
    }
    if h.starts_with(b"PK\x03\x04") && e == "docx" {
        return Some("document");
    }
    if matches!(e.as_str(), "txt" | "md" | "markdown")
        && !h.contains(&0)
        && std::str::from_utf8(h).is_ok()
    {
        return Some("document");
    }
    None
}

fn kind_limit(kind: &str, max_media: u64) -> u64 {
    match kind {
        "document" => MAX_DOC_BYTES,
        "image" => MAX_IMAGE_BYTES,
        _ => max_media,
    }
}

fn io_err(e: &std::io::Error) -> ApiErr {
    use std::io::ErrorKind::{PermissionDenied, StorageFull};
    match e.kind() {
        StorageFull => ApiErr::new(
            507,
            "STORAGE_FULL",
            "the server has no space left for this upload",
        ),
        PermissionDenied => ApiErr::unavailable(
            "STORAGE_UNAVAILABLE",
            "the upload directory is not writable",
        ),
        _ => ApiErr::unavailable(
            "STORAGE_UNAVAILABLE",
            format!("storage error: {}", e.kind()),
        ),
    }
}

struct Guard<'a>(&'a std::sync::atomic::AtomicUsize);
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Core {
    pub fn uploads_dir(&self) -> PathBuf {
        self.cfg.data_dir.join("uploads")
    }

    fn upload_dir(&self, id: &str) -> PathBuf {
        self.uploads_dir().join(id)
    }

    pub fn read_upload_meta(&self, id: &str) -> Option<Value> {
        if !id.starts_with("upl_") || id.len() > 40 {
            return None;
        }
        let t = std::fs::read_to_string(self.upload_dir(id).join("meta.json")).ok()?;
        serde_json::from_str(&t).ok()
    }

    pub fn write_upload_meta(&self, id: &str, meta: &Value) -> ApiResult<()> {
        let dir = self.upload_dir(id);
        let tmp = dir.join("meta.json.tmp");
        std::fs::write(&tmp, meta.to_string()).map_err(|e| io_err(&e))?;
        std::fs::rename(&tmp, dir.join("meta.json")).map_err(|e| io_err(&e))
    }

    /// Caminho do blob de um upload que ainda está em staging (`None` se já importado/inexistente).
    pub fn upload_blob(&self, id: &str) -> Option<(PathBuf, Value)> {
        let m = self.read_upload_meta(id)?;
        let name = m["filename"].as_str()?;
        Some((self.upload_dir(id).join(name), m))
    }

    pub fn list_uploads(&self) -> Vec<Value> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.uploads_dir()) {
            for e in rd.flatten() {
                if let Some(m) = e
                    .file_name()
                    .to_str()
                    .and_then(|n| self.read_upload_meta(n))
                {
                    out.push(m);
                }
            }
        }
        out.sort_by(|a, b| a["upload_id"].as_str().cmp(&b["upload_id"].as_str()));
        out
    }

    fn staged_bytes(&self) -> u64 {
        self.list_uploads()
            .iter()
            .filter_map(|m| m["size"].as_u64())
            .sum()
    }

    pub fn delete_upload(&self, id: &str) -> bool {
        self.read_upload_meta(id).is_some() && std::fs::remove_dir_all(self.upload_dir(id)).is_ok()
    }

    /// Núcleo comum do upload (streaming ou inline): escreve `*.part`, faz hash e sniff, aplica
    /// teto/cota/tempo e publica por `rename`. Idempotente por conteúdo (token + hash + nome).
    pub fn store_upload(
        &self,
        ctx: &CallCtx,
        filename: &str,
        expected_sha: Option<&str>,
        reader: &mut dyn Read,
        declared_len: Option<u64>,
    ) -> ApiResult<Value> {
        let token_id = ctx
            .principal
            .as_ref()
            .map(|p| p.token_id.clone())
            .unwrap_or_default();
        if self.uploads_active.fetch_add(1, Ordering::SeqCst) >= self.cfg.max_concurrent_uploads {
            self.uploads_active.fetch_sub(1, Ordering::SeqCst);
            return Err(
                ApiErr::new(429, "TOO_MANY_UPLOADS", "too many uploads in flight").with_retry(2),
            );
        }
        let _g = Guard(&self.uploads_active);
        if let Some(n) = declared_len
            && n > self.cfg.max_upload_bytes
        {
            return Err(ApiErr::new(
                413,
                "UPLOAD_TOO_LARGE",
                format!("the upload exceeds {} bytes", self.cfg.max_upload_bytes),
            ));
        }
        let used = self.staged_bytes();
        if used.saturating_add(declared_len.unwrap_or(0)) > self.cfg.upload_quota_bytes {
            return Err(ApiErr::new(
                507,
                "STORAGE_QUOTA_EXCEEDED",
                "the staging quota is exhausted; delete uploads or import them first",
            ));
        }
        let name = sanitize_filename(filename);
        let id = format!("upl_{}", random_hex(8).map_err(ApiErr::internal)?);
        let dir = self.upload_dir(&id);
        std::fs::create_dir_all(&dir).map_err(|e| io_err(&e))?;
        let part = dir.join(format!("{name}.part"));
        let cleanup = |dir: &Path| {
            let _ = std::fs::remove_dir_all(dir);
        };
        let mut f = match std::fs::File::create(&part) {
            Ok(f) => f,
            Err(e) => {
                cleanup(&dir);
                return Err(io_err(&e));
            }
        };
        let t0 = Instant::now();
        let mut hasher = Sha256::new();
        let mut head = Vec::with_capacity(HEAD);
        let mut size: u64 = 0;
        let mut buf = vec![0u8; CHUNK];
        loop {
            if t0.elapsed() > self.cfg.upload_timeout {
                cleanup(&dir);
                return Err(ApiErr::new(
                    408,
                    "UPLOAD_TIMEOUT",
                    "the upload took too long",
                ));
            }
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    cleanup(&dir);
                    return Err(match e.kind() {
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                            ApiErr::new(408, "UPLOAD_TIMEOUT", "the client stopped sending data")
                        }
                        _ => ApiErr::bad_request("the upload was interrupted"),
                    });
                }
            };
            size += n as u64;
            if size > self.cfg.max_upload_bytes {
                cleanup(&dir);
                return Err(ApiErr::new(
                    413,
                    "UPLOAD_TOO_LARGE",
                    "the upload exceeds the size limit",
                ));
            }
            if used.saturating_add(size) > self.cfg.upload_quota_bytes {
                cleanup(&dir);
                return Err(ApiErr::new(
                    507,
                    "STORAGE_QUOTA_EXCEEDED",
                    "the staging quota is exhausted",
                ));
            }
            if head.len() < HEAD {
                let take = (HEAD - head.len()).min(n);
                head.extend_from_slice(&buf[..take]);
            }
            hasher.update(&buf[..n]);
            if let Err(e) = f.write_all(&buf[..n]) {
                cleanup(&dir);
                return Err(io_err(&e));
            }
        }
        if let Err(e) = f.sync_all() {
            cleanup(&dir);
            return Err(io_err(&e));
        }
        drop(f);
        if size == 0 {
            cleanup(&dir);
            return Err(ApiErr::invalid("the upload is empty"));
        }
        let sha = crate::mac::hex(&hasher.finalize());
        if let Some(exp) = expected_sha
            && !exp.eq_ignore_ascii_case(&sha)
        {
            cleanup(&dir);
            return Err(ApiErr::new(
                422,
                "CHECKSUM_MISMATCH",
                "the SHA-256 of the received bytes does not match `expected_sha256`",
            ));
        }
        let Some(kind) = sniff(&head, &name) else {
            cleanup(&dir);
            return Err(ApiErr::new(
                415,
                "UNSUPPORTED_MEDIA_TYPE",
                "the content is not a supported media/document type (checked by content, not by name)",
            ));
        };
        if size > kind_limit(kind, self.cfg.max_upload_bytes) {
            cleanup(&dir);
            return Err(ApiErr::new(
                413,
                "UPLOAD_TOO_LARGE",
                format!(
                    "a {kind} upload is limited to {} bytes",
                    kind_limit(kind, self.cfg.max_upload_bytes)
                ),
            ));
        }
        // idempotência por conteúdo: mesmo token + mesmo hash + mesmo nome ⇒ o upload existente
        if let Some(existing) = self.list_uploads().into_iter().find(|m| {
            m["sha256"] == sha.as_str()
                && m["filename"] == name.as_str()
                && m["token_id"] == token_id.as_str()
                && m["state"] == "staged"
        }) {
            cleanup(&dir);
            let mut v = existing;
            v["deduplicated"] = json!(true);
            return Ok(v);
        }
        std::fs::rename(&part, dir.join(&name)).map_err(|e| {
            cleanup(&dir);
            io_err(&e)
        })?;
        let meta = json!({
            "upload_id": id, "filename": name, "size": size, "sha256": sha, "kind": kind,
            "token_id": token_id, "created_ms": now_ms(), "state": "staged",
        });
        self.write_upload_meta(&id, &meta)
            .inspect_err(|_| cleanup(&dir))?;
        Ok(meta)
    }

    /// `uploads.create` (streaming): mesmo pipeline do catálogo, com o leitor do transporte.
    pub fn upload_stream(
        &self,
        ctx: &CallCtx,
        filename: &str,
        expected_sha: Option<&str>,
        content_length: u64,
        reader: &mut dyn Read,
    ) -> ApiResult<crate::core::Reply> {
        let def = crate::catalog::find("uploads.create")
            .ok_or_else(|| ApiErr::internal("uploads.create is missing from the catalog"))?;
        let mut params = json!({"filename": filename});
        if let Some(s) = expected_sha {
            params["expected_sha256"] = json!(s);
        }
        self.call_def(ctx, def, params, |c, _d, _p| {
            c.store_upload(ctx, filename, expected_sha, reader, Some(content_length))
        })
    }

    /// `uploads.create_inline`: base64 pequeno.
    pub fn upload_inline(
        &self,
        ctx: &CallCtx,
        filename: &str,
        b64: &str,
        expected_sha: Option<&str>,
    ) -> ApiResult<Value> {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .map_err(|_| ApiErr::invalid("`content_base64` is not valid base64"))?;
        if bytes.len() > MAX_INLINE_BYTES {
            return Err(ApiErr::new(
                413,
                "UPLOAD_TOO_LARGE",
                "inline uploads are limited to 8 MiB",
            ));
        }
        let len = bytes.len() as u64;
        self.store_upload(
            ctx,
            filename,
            expected_sha,
            &mut bytes.as_slice(),
            Some(len),
        )
    }
}

/// Hash de verificação rápida (exposto para os testes).
pub fn sha_of(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_are_reduced_to_a_safe_basename() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(
            sanitize_filename("C:\\Users\\x\\clip final.mp4"),
            "clip final.mp4"
        );
        assert_eq!(sanitize_filename(".hidden"), "hidden");
        assert_eq!(sanitize_filename("a\0b<>|?.mp4"), "a_b____.mp4");
        assert_eq!(sanitize_filename(".."), "file");
        assert_eq!(sanitize_filename(""), "file");
        assert!(sanitize_filename(&"x".repeat(500)).len() <= 120);
    }

    #[test]
    fn sniffing_trusts_bytes_not_names() {
        let mp4 = b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom";
        assert_eq!(sniff(mp4, "evil.txt"), Some("video"));
        assert_eq!(sniff(b"MZ\x90\0\x03\0\0\0", "movie.mp4"), None);
        assert_eq!(sniff(b"#!/bin/sh\nrm -rf /\n", "x.mp4"), None);
        assert_eq!(sniff(b"hello\n", "brief.txt"), Some("document"));
        assert_eq!(sniff(b"hello\0\n", "brief.txt"), None);
        assert_eq!(sniff(b"PK\x03\x04....", "a.zip"), None);
        assert_eq!(sniff(b"PK\x03\x04....", "a.docx"), Some("document"));
        assert_eq!(sniff(b"%PDF-1.7", "a.bin"), Some("document"));
        assert_eq!(
            sniff(&[0x89, b'P', b'N', b'G', 0, 0, 0, 0, 0, 0, 0, 0], "a"),
            Some("image")
        );
    }
}
