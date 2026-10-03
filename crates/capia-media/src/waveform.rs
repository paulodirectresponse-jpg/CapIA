//! Waveform multirresolução (ADR-055): pirâmide de *buckets* `(min, max, rms)` sobre o áudio
//! mixado em mono, em f32. Dado **derivado** (cache): formato binário `CWFM` v1 versionado, com
//! checksum SHA-256; truncado/corrompido ⇒ `MEDIA_INDEX_INVALID` e o chamador regenera.
//!
//! Layout (little-endian): magic(4) versão(u32) sample_rate(u32) total_samples(u64) base_block(u32)
//! níveis(u32) · por nível: buckets(u64) + `n × 12` bytes (min,max,rms f32) · SHA-256 de tudo.

use crate::decode::decode_audio_blocks;
use crate::error::{MediaError, MediaErrorCode};
use crate::toolchain::MediaToolchain;
use capia_time::Ticks;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

pub const WAVEFORM_MAGIC: [u8; 4] = *b"CWFM";
pub const WAVEFORM_VERSION: u32 = 1;
/// Versão do formato + do algoritmo (entra na `CacheKey`).
pub const WAVEFORM_PRODUCER: &str = "waveform/1";
/// Amostras por bucket do nível 0.
pub const BASE_BLOCK: u32 = 64;
/// Cada nível agrega 4 do anterior.
const FANOUT: u64 = 4;
/// Teto de amostras (≈ 1.000 h a 48 kHz).
pub const MAX_WAVEFORM_SAMPLES: u64 = 1_000 * 3600 * 48_000;
const HEADER: usize = 28;

fn invalid(m: impl Into<String>) -> MediaError {
    MediaError::new(MediaErrorCode::MediaIndexInvalid, m)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Peak {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Waveform {
    sample_rate: u32,
    total_samples: u64,
    levels: Vec<Vec<Peak>>,
}

impl Waveform {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn total_samples(&self) -> u64 {
        self.total_samples
    }

    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Amostras por bucket do nível `l`.
    pub fn block_size(&self, l: usize) -> u64 {
        u64::from(BASE_BLOCK) * FANOUT.pow(l as u32)
    }

    pub fn level(&self, l: usize) -> Option<&[Peak]> {
        self.levels.get(l).map(Vec::as_slice)
    }

    pub fn duration(&self) -> Ticks {
        crate::decode::samples_to_ticks(self.total_samples, self.sample_rate)
    }

    /// Picos de `[start, end)` em, **no máximo**, `buckets` pontos (nível mais fino cujo bucket
    /// cabe no passo pedido). Vazio se o intervalo é vazio/fora da mídia.
    pub fn query(&self, start: Ticks, end: Ticks, buckets: usize) -> Vec<Peak> {
        if buckets == 0 || end.0 <= start.0 || self.levels.is_empty() {
            return Vec::new();
        }
        let s = crate::decode::ticks_to_samples(start, self.sample_rate).min(self.total_samples);
        let e = crate::decode::ticks_to_samples(end, self.sample_rate).min(self.total_samples);
        if e <= s {
            return Vec::new();
        }
        let span = e - s;
        let per_bucket = span.div_ceil(buckets as u64).max(1);
        // maior nível cujo bloco ≤ per_bucket (nível 0 se nenhum)
        let mut l = 0usize;
        while l + 1 < self.levels.len() && self.block_size(l + 1) <= per_bucket {
            l += 1;
        }
        let bs = self.block_size(l);
        let first = (s / bs) as usize;
        let last = e.div_ceil(bs) as usize;
        let lvl = &self.levels[l];
        let slice = &lvl[first.min(lvl.len())..last.min(lvl.len())];
        // reduz para ≤ buckets agrupando vizinhos
        let group = slice.len().div_ceil(buckets).max(1);
        slice.chunks(group).map(merge).collect()
    }

    pub fn encode(&self) -> Vec<u8> {
        let cap: usize = self.levels.iter().map(|l| 8 + l.len() * 12).sum();
        let mut out = Vec::with_capacity(HEADER + cap + 32);
        out.extend_from_slice(&WAVEFORM_MAGIC);
        out.extend_from_slice(&WAVEFORM_VERSION.to_le_bytes());
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&self.total_samples.to_le_bytes());
        out.extend_from_slice(&BASE_BLOCK.to_le_bytes());
        out.extend_from_slice(&(self.levels.len() as u32).to_le_bytes());
        for l in &self.levels {
            out.extend_from_slice(&(l.len() as u64).to_le_bytes());
            for p in l {
                out.extend_from_slice(&p.min.to_le_bytes());
                out.extend_from_slice(&p.max.to_le_bytes());
                out.extend_from_slice(&p.rms.to_le_bytes());
            }
        }
        let sum = Sha256::digest(&out);
        out.extend_from_slice(&sum);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MediaError> {
        if bytes.len() < HEADER + 32 {
            return Err(invalid("waveform is truncated"));
        }
        let (body, sum) = bytes.split_at(bytes.len() - 32);
        if Sha256::digest(body).as_slice() != sum {
            return Err(invalid("waveform checksum mismatch"));
        }
        if body[..4] != WAVEFORM_MAGIC {
            return Err(invalid("not a waveform file"));
        }
        let u32_at =
            |o: usize| u32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
        let u64_at = |o: usize| {
            let mut b = [0u8; 8];
            b.copy_from_slice(&body[o..o + 8]);
            u64::from_le_bytes(b)
        };
        if u32_at(4) != WAVEFORM_VERSION {
            return Err(invalid("unsupported waveform version"));
        }
        let sample_rate = u32_at(8);
        let total_samples = u64_at(12);
        if sample_rate == 0 || total_samples > MAX_WAVEFORM_SAMPLES || u32_at(20) != BASE_BLOCK {
            return Err(invalid("invalid waveform header"));
        }
        let n_levels = u32_at(24) as usize;
        if n_levels > 32 {
            return Err(invalid("too many levels"));
        }
        let mut off = HEADER;
        let mut levels = Vec::with_capacity(n_levels);
        for l in 0..n_levels {
            if off + 8 > body.len() {
                return Err(invalid("level header out of bounds"));
            }
            let n = u64_at(off);
            off += 8;
            let bs = u64::from(BASE_BLOCK) * FANOUT.pow(l as u32);
            if n != total_samples.div_ceil(bs) {
                return Err(invalid("bucket count does not match total samples"));
            }
            let bytes_needed = usize::try_from(n)
                .ok()
                .and_then(|n| n.checked_mul(12))
                .ok_or_else(|| invalid("level size overflows"))?;
            if off.checked_add(bytes_needed) != Some(off + bytes_needed)
                || off + bytes_needed > body.len()
            {
                return Err(invalid("level data out of bounds"));
            }
            let mut v = Vec::with_capacity(n as usize);
            for c in body[off..off + bytes_needed].chunks_exact(12) {
                let f = |i: usize| f32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
                let p = Peak {
                    min: f(0),
                    max: f(4),
                    rms: f(8),
                };
                if !(p.min.is_finite() && p.max.is_finite() && p.rms.is_finite()) || p.min > p.max {
                    return Err(invalid("non-finite or inverted peak"));
                }
                v.push(p);
            }
            off += bytes_needed;
            levels.push(v);
        }
        if off != body.len() {
            return Err(invalid("trailing bytes after the last level"));
        }
        Ok(Self {
            sample_rate,
            total_samples,
            levels,
        })
    }
}

/// Combina buckets vizinhos do nível (rms ponderado; o último bucket pode ser parcial e é tratado
/// como cheio — erro de consulta de no máximo um bucket, nunca de dados gravados).
fn merge(c: &[Peak]) -> Peak {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    let mut sq = 0f64;
    for p in c {
        min = min.min(p.min);
        max = max.max(p.max);
        sq += f64::from(p.rms) * f64::from(p.rms);
    }
    Peak {
        min,
        max,
        rms: (sq / c.len().max(1) as f64).sqrt() as f32,
    }
}

/// Constrói a pirâmide em streaming (memória O(buckets)).
#[derive(Debug)]
pub struct WaveformBuilder {
    sample_rate: u32,
    total: u64,
    cur_min: f32,
    cur_max: f32,
    cur_sq: f64,
    cur_n: u32,
    level0: Vec<Peak>,
}

impl WaveformBuilder {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            total: 0,
            cur_min: f32::INFINITY,
            cur_max: f32::NEG_INFINITY,
            cur_sq: 0.0,
            cur_n: 0,
            level0: Vec::new(),
        }
    }

    pub fn push(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        if self.total + samples.len() as u64 > MAX_WAVEFORM_SAMPLES {
            return Err(MediaError::new(
                MediaErrorCode::MediaLimitExceeded,
                "audio is too long for a waveform",
            ));
        }
        for &s in samples {
            // NaN/inf viram silêncio: nunca contaminam a pirâmide
            let s = if s.is_finite() {
                s.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            self.cur_min = self.cur_min.min(s);
            self.cur_max = self.cur_max.max(s);
            self.cur_sq += f64::from(s) * f64::from(s);
            self.cur_n += 1;
            if self.cur_n == BASE_BLOCK {
                self.flush();
            }
        }
        self.total += samples.len() as u64;
        Ok(())
    }

    fn flush(&mut self) {
        if self.cur_n > 0 {
            self.level0.push(Peak {
                min: self.cur_min,
                max: self.cur_max,
                rms: (self.cur_sq / f64::from(self.cur_n)).sqrt() as f32,
            });
        }
        self.cur_min = f32::INFINITY;
        self.cur_max = f32::NEG_INFINITY;
        self.cur_sq = 0.0;
        self.cur_n = 0;
    }

    pub fn finish(mut self) -> Result<Waveform, MediaError> {
        self.flush();
        if self.total == 0 {
            return Err(invalid("the audio stream has no samples"));
        }
        let total = self.total;
        let mut levels = vec![self.level0];
        loop {
            let l = levels.len() - 1;
            if levels[l].len() <= 1 {
                break;
            }
            let child_bs = u64::from(BASE_BLOCK) * FANOUT.pow(l as u32);
            let prev = &levels[l];
            let parent: Vec<Peak> = prev
                .chunks(FANOUT as usize)
                .enumerate()
                .map(|(i, c)| {
                    let mut min = f32::INFINITY;
                    let mut max = f32::NEG_INFINITY;
                    let mut sq = 0f64;
                    let mut n = 0f64;
                    for (j, p) in c.iter().enumerate() {
                        let idx = (i * FANOUT as usize + j) as u64;
                        let begin = idx * child_bs;
                        let samples = (total - begin).min(child_bs) as f64;
                        min = min.min(p.min);
                        max = max.max(p.max);
                        sq += f64::from(p.rms) * f64::from(p.rms) * samples;
                        n += samples;
                    }
                    Peak {
                        min,
                        max,
                        rms: (sq / n).sqrt() as f32,
                    }
                })
                .collect();
            levels.push(parent);
        }
        Ok(Waveform {
            sample_rate: self.sample_rate,
            total_samples: total,
            levels,
        })
    }
}

/// Decodifica o stream de áudio inteiro (mono, taxa nativa) e gera o waveform. `progress(amostras)`.
/// Se o decode terminar com erro, **nada** é devolvido (nunca um waveform parcial).
pub fn generate_waveform(
    tc: &MediaToolchain,
    path: &Path,
    stream_index: u32,
    sample_rate: u32,
    timeout: Duration,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64),
) -> Result<Waveform, MediaError> {
    let mut b = WaveformBuilder::new(sample_rate);
    let mut seen = 0u64;
    decode_audio_blocks(
        tc,
        path,
        stream_index,
        sample_rate,
        1,
        timeout,
        cancel,
        &mut |block| {
            b.push(block)?;
            seen += block.len() as u64;
            progress(seen);
            Ok(())
        },
    )?;
    b.finish()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use capia_time::TICKS_PER_SECOND;

    fn build(samples: &[f32]) -> Waveform {
        let mut b = WaveformBuilder::new(48_000);
        // empurra em pedaços irregulares: o resultado não pode depender do fatiamento
        for c in samples.chunks(1000) {
            b.push(c).unwrap();
        }
        b.finish().unwrap()
    }

    fn square(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| if (i / 100) % 2 == 0 { 0.5 } else { -0.25 })
            .collect()
    }

    #[test]
    fn levels_cover_all_samples_with_exact_min_max() {
        let s = square(100_003);
        let w = build(&s);
        assert_eq!(w.total_samples(), 100_003);
        let l0 = w.level(0).unwrap();
        assert_eq!(l0.len(), 100_003usize.div_ceil(64));
        let top = w.level(w.level_count() - 1).unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!((top[0].min, top[0].max), (-0.25, 0.5));
        for l in 0..w.level_count() {
            let bs = w.block_size(l) as usize;
            for (i, p) in w.level(l).unwrap().iter().enumerate() {
                let chunk = &s[i * bs..((i + 1) * bs).min(s.len())];
                let mn = chunk.iter().cloned().fold(f32::INFINITY, f32::min);
                let mx = chunk.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                assert_eq!((p.min, p.max), (mn, mx), "level {l} bucket {i}");
                let rms = (chunk.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>()
                    / chunk.len() as f64)
                    .sqrt() as f32;
                assert!(
                    (p.rms - rms).abs() < 1e-4,
                    "level {l} bucket {i}: {} vs {rms}",
                    p.rms
                );
            }
        }
    }

    #[test]
    fn binary_round_trip_and_corruption_detection() {
        let w = build(&square(5000));
        let bytes = w.encode();
        assert_eq!(Waveform::decode(&bytes).unwrap(), w);
        for cut in 0..bytes.len() {
            assert!(
                Waveform::decode(&bytes[..cut]).is_err(),
                "truncated at {cut}"
            );
        }
        for i in (0..bytes.len()).step_by(7) {
            let mut b = bytes.clone();
            b[i] ^= 0x40;
            assert!(Waveform::decode(&b).is_err(), "flip at {i}");
        }
    }

    #[test]
    fn non_finite_input_never_poisons_the_pyramid() {
        let mut s = square(1000);
        s[10] = f32::NAN;
        s[11] = f32::INFINITY;
        s[12] = 40.0;
        let w = build(&s);
        for l in 0..w.level_count() {
            for p in w.level(l).unwrap() {
                assert!(p.min.is_finite() && p.max.is_finite() && p.rms.is_finite());
                assert!(p.max <= 1.0 && p.min >= -1.0);
            }
        }
    }

    #[test]
    fn query_returns_at_most_the_requested_buckets_and_the_right_range() {
        let s = square(48_000 * 4);
        let w = build(&s);
        let all = w.query(Ticks(0), w.duration(), 100);
        assert!(!all.is_empty() && all.len() <= 100);
        assert!(all.iter().all(|p| p.min >= -0.25 && p.max <= 0.5));
        // um trecho curto usa nível fino e enxerga a transição exata
        let one = w.query(Ticks(0), Ticks(TICKS_PER_SECOND / 48_000 * 50), 1);
        assert_eq!(one.len(), 1);
        assert_eq!((one[0].min, one[0].max), (0.5, 0.5));
        assert!(w.query(Ticks(10), Ticks(10), 5).is_empty());
        assert!(w.query(Ticks(0), w.duration(), 0).is_empty());
        assert!(w.query(Ticks(i64::MAX - 1), Ticks(i64::MAX), 5).is_empty());
    }

    #[test]
    fn empty_audio_is_an_error() {
        assert!(WaveformBuilder::new(48_000).finish().is_err());
    }
}
