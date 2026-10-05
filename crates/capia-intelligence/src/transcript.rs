//! Transcrição de um asset: áudio **mono 16 kHz** comprimido por trechos (nunca o vídeo), STT
//! pelo provider roteado (local ou nuvem, conforme o Brain Profile e a privacidade), costura dos
//! trechos em microssegundos inteiros e cache determinístico por conteúdo+parâmetros+modelo.

use crate::ctx::IntelCtx;
use crate::error::{IntelError, IntelResult};
use crate::records::{self, KIND_TRANSCRIPT, now_ms};
use capia_ai::dispatcher::TaskCtx;
use capia_ai::stt::{Segment, SttRequest, Transcript, Word};
use capia_media::{FfprobeBackend, MediaProbe, SttAudioFormat, extract_audio_chunk};
use capia_time::{TICKS_PER_SECOND, Ticks};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Teto de bytes por trecho enviado (FLAC mono 16 kHz de 10 min ≈ 10–20 MB; teto de segurança).
pub const MAX_CHUNK_BYTES: usize = 48 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscribeParams {
    pub asset_id: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_chunk")]
    pub chunk_secs: u32,
    #[serde(default = "default_true")]
    pub word_timestamps: bool,
    /// Vocabulário (nomes de marca etc.). Dado, nunca instrução.
    #[serde(default)]
    pub vocabulary: Option<String>,
    /// Ignora o cache.
    #[serde(default)]
    pub force: bool,
}

fn default_chunk() -> u32 {
    600
}

fn default_true() -> bool {
    true
}

impl TranscribeParams {
    pub fn new(asset_id: impl Into<String>) -> Self {
        Self {
            asset_id: asset_id.into(),
            language: None,
            chunk_secs: default_chunk(),
            word_timestamps: true,
            vocabulary: None,
            force: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscriptRecord {
    pub id: String,
    pub asset_id: String,
    pub transcript: Transcript,
    pub endpoint_id: String,
    pub provider_id: String,
    pub model_id: String,
    /// `true` se o áudio não saiu da máquina.
    pub local: bool,
    /// Custo total **conhecido** (micro-unidades); `None` = desconhecido (≠ zero).
    pub cost_micros: Option<u64>,
    pub created_ms: u64,
    pub from_cache: bool,
}

/// µs → Ticks (705,6 ticks por µs), arredondando para o mais próximo, determinístico.
pub fn ticks_from_us(us: i64) -> Ticks {
    Ticks((i128::from(us) * i128::from(TICKS_PER_SECOND) + 500_000).div_euclid(1_000_000) as i64)
}

/// Ticks → µs (arredonda para o mais próximo).
pub fn us_from_ticks(t: Ticks) -> i64 {
    ((i128::from(t.0) * 1_000_000 + i128::from(TICKS_PER_SECOND) / 2)
        .div_euclid(i128::from(TICKS_PER_SECOND))) as i64
}

#[derive(Clone, Debug)]
pub struct AssetFile {
    pub id: String,
    pub path: PathBuf,
    pub duration: Ticks,
    pub has_audio: bool,
    pub has_video: bool,
}

/// Localiza o arquivo de um asset pelo catálogo do engine (leitura).
pub fn asset_file(ctx: &IntelCtx, asset_id: &str) -> IntelResult<AssetFile> {
    let rows = ctx.engine.read("assets.list", serde_json::json!({}))?;
    let row = rows
        .as_array()
        .and_then(|a| a.iter().find(|r| r["id"].as_str() == Some(asset_id)))
        .ok_or_else(|| IntelError::new("ASSET_NOT_FOUND", format!("asset {asset_id} not found")))?;
    if row["status"].as_str().is_some_and(|s| s != "online") {
        return Err(IntelError::new(
            "ASSET_OFFLINE",
            format!("asset {asset_id} is not available (relink it first)"),
        ));
    }
    let path = row["path"]
        .as_str()
        .ok_or_else(|| IntelError::new("ASSET_OFFLINE", "the asset has no file path"))?;
    Ok(AssetFile {
        id: asset_id.to_owned(),
        path: PathBuf::from(path),
        duration: Ticks(row["duration"].as_i64().unwrap_or(0)),
        has_audio: row["has_audio"].as_bool().unwrap_or(true),
        has_video: row["has_video"].as_bool().unwrap_or(false),
    })
}

pub type Progress<'a> = &'a (dyn Fn(u32, u32) + Send + Sync);

/// Transcreve (ou devolve do cache) o áudio de um asset.
pub async fn transcribe_asset(
    ctx: &IntelCtx,
    task: &TaskCtx,
    p: &TranscribeParams,
    progress: Progress<'_>,
) -> IntelResult<TranscriptRecord> {
    let file = asset_file(ctx, &p.asset_id)?;
    if !file.has_audio {
        return Err(IntelError::new("NO_AUDIO", "the asset has no audio stream"));
    }
    let tc = ctx
        .engine
        .toolchain()
        .ok_or_else(|| IntelError::new("FFMPEG_NOT_FOUND", "ffmpeg is not available"))?;

    // O destino da chamada define a chave do cache: mesmo conteúdo + parâmetros + modelo.
    let route = ctx.stt_route();
    let decisions = ctx.ai.route_preview(&route).map_err(IntelError::from)?;
    let first = decisions
        .first()
        .ok_or_else(|| IntelError::new("NO_CAPABLE_MODEL", "no speech-to-text endpoint"))?;
    let id = format!(
        "tr_{}",
        records::key(&[
            &p.asset_id,
            &first.endpoint_id,
            &first.model_id,
            p.language.as_deref().unwrap_or("auto"),
            &p.chunk_secs.to_string(),
            if p.word_timestamps { "w" } else { "s" },
            p.vocabulary.as_deref().unwrap_or(""),
        ])
    );
    if !p.force {
        if let Some(mut rec) = ctx
            .records()?
            .latest::<TranscriptRecord>(KIND_TRANSCRIPT, &id)?
        {
            rec.from_cache = true;
            return Ok(rec);
        }
    }

    let probe = FfprobeBackend::new(tc.clone());
    let info = {
        let path = file.path.clone();
        tokio::task::spawn_blocking(move || probe.probe(&path))
            .await
            .map_err(|e| IntelError::new("INTERNAL", e.to_string()))??
    };
    let audio_index = info
        .audio()
        .map(|a| a.index)
        .ok_or_else(|| IntelError::new("NO_AUDIO", "the asset has no audio stream"))?;
    let total = info.duration.unwrap_or(file.duration);
    if total.0 <= 0 {
        return Err(IntelError::new("MEDIA_EMPTY", "the asset has no duration"));
    }
    let chunk = Ticks(i64::from(p.chunk_secs.clamp(30, 1800)) * TICKS_PER_SECOND);
    let n_chunks = u32::try_from((total.0 + chunk.0 - 1) / chunk.0).unwrap_or(u32::MAX);

    let mut segments: Vec<Segment> = Vec::new();
    let mut language: Option<String> = p.language.clone();
    let mut cost: Option<u64> = Some(0);
    let mut local_all = true;
    let mut chosen = (
        first.endpoint_id.clone(),
        first.provider_id.clone(),
        first.model_id.clone(),
    );
    for i in 0..n_chunks {
        if task.cancel.is_cancelled() {
            return Err(IntelError::cancelled());
        }
        progress(i, n_chunks);
        let start = Ticks(i64::from(i) * chunk.0);
        let len = Ticks((total.0 - start.0).min(chunk.0));
        let bytes = {
            let (tc, path, cancel) = (tc.clone(), file.path.clone(), task.cancel.clone());
            tokio::task::spawn_blocking(move || {
                extract_audio_chunk(
                    &tc,
                    &path,
                    audio_index,
                    start,
                    len,
                    SttAudioFormat::Flac,
                    MAX_CHUNK_BYTES,
                    Duration::from_secs(600),
                    &move || cancel.is_cancelled(),
                )
            })
            .await
            .map_err(|e| IntelError::new("INTERNAL", e.to_string()))?
        };
        let bytes = match bytes {
            Ok(b) => b,
            // trecho final vazio (duração arredondada além do áudio real) não é erro
            Err(e) if i + 1 == n_chunks && i > 0 && e.message.contains("empty") => continue,
            Err(e) => return Err(e.into()),
        };
        let req = SttRequest {
            model: String::new(),
            audio: bytes,
            mime: SttAudioFormat::Flac.mime().to_owned(),
            filename: format!("chunk{i}.{}", SttAudioFormat::Flac.extension()),
            language: language.clone(),
            word_timestamps: p.word_timestamps,
            prompt: p.vocabulary.clone(),
        };
        let out = ctx.ai.transcribe(task, req, route.clone()).await?;
        chosen = (
            out.decision.endpoint_id.clone(),
            out.decision.provider_id.clone(),
            out.decision.model_id.clone(),
        );
        local_all &= ctx.is_local(&out.decision.endpoint_id);
        cost = match (cost, out.cost.known) {
            (Some(c), true) => Some(c + out.cost.micros),
            _ => None,
        };
        if language.is_none() {
            language.clone_from(&out.transcript.language);
        }
        let offset_us = us_from_ticks(start);
        for mut s in out.transcript.segments {
            s.start_us += offset_us;
            s.end_us += offset_us;
            for w in &mut s.words {
                shift(w, offset_us);
            }
            segments.push(s);
        }
    }
    progress(n_chunks, n_chunks);
    segments.sort_by_key(|s| (s.start_us, s.end_us));
    let transcript = Transcript {
        schema_version: capia_ai::stt::TRANSCRIPT_SCHEMA_VERSION,
        language,
        duration_us: Some(us_from_ticks(total)),
        segments,
    };
    transcript
        .validate()
        .map_err(|m| IntelError::new("INVALID_PROVIDER_RESPONSE", m))?;
    let rec = TranscriptRecord {
        id: id.clone(),
        asset_id: p.asset_id.clone(),
        transcript,
        endpoint_id: chosen.0,
        provider_id: chosen.1,
        model_id: chosen.2,
        local: local_all,
        cost_micros: cost,
        created_ms: now_ms(),
        from_cache: false,
    };
    ctx.records()?
        .put(KIND_TRANSCRIPT, &id, 1, Some(&p.asset_id), &rec)?;
    Ok(rec)
}

fn shift(w: &mut Word, by: i64) {
    w.start_us += by;
    w.end_us += by;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_conversion_roundtrips_at_microsecond_resolution() {
        for us in [0i64, 1, 999, 1_000_000, 123_456_789, 36_000_000_000] {
            assert_eq!(us_from_ticks(ticks_from_us(us)), us);
        }
        assert_eq!(ticks_from_us(1_000_000), Ticks(TICKS_PER_SECOND));
    }
}
