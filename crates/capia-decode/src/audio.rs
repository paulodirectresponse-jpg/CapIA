//! Leitura de PCM por blocos com cache LRU por bytes (ADR-062). O seek vem do índice de frames de
//! áudio (`capia-media`): pedir 1 s em t = 300 s não custa 300 s de decode.

use crate::cache::{ByteLru, CacheStats};
use capia_media::{
    AudioIndex, AudioPcm, MediaError, MediaErrorCode, MediaToolchain, decode_audio_indexed,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Versão do backend de PCM: entra na chave (mudar o decoder/formato invalida o cache).
pub const PCM_BACKEND_VERSION: u32 = 1;
/// Amostras por canal de um bloco de cache (≈ 0,34 s a 48 kHz).
pub const PCM_BLOCK_FRAMES: u64 = 16_384;

#[derive(Clone, Debug)]
pub struct AudioSource {
    pub namespace: u64,
    pub content: Arc<str>,
    pub path: PathBuf,
    pub index: Arc<AudioIndex>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PcmKey {
    namespace: u64,
    content: Arc<str>,
    stream: u32,
    block: u64,
    backend: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PcmMetrics {
    pub cache: CacheStats,
    /// Chamadas ao decoder (cada uma cobre uma corrida contígua de blocos ausentes).
    pub decode_calls: u64,
    pub decoded_frames: u64,
}

#[derive(Debug)]
pub struct PcmCache {
    toolchain: MediaToolchain,
    cache: Mutex<ByteLru<PcmKey, Vec<f32>>>,
    decode_calls: AtomicU64,
    decoded_frames: AtomicU64,
    timeout: Duration,
}

impl PcmCache {
    pub fn new(toolchain: MediaToolchain, budget_bytes: u64) -> Self {
        Self {
            toolchain,
            cache: Mutex::new(ByteLru::new(budget_bytes)),
            decode_calls: AtomicU64::new(0),
            decoded_frames: AtomicU64::new(0),
            timeout: Duration::from_secs(600),
        }
    }

    pub fn metrics(&self) -> PcmMetrics {
        PcmMetrics {
            cache: self.lock().stats(),
            decode_calls: self.decode_calls.load(Ordering::SeqCst),
            decoded_frames: self.decoded_frames.load(Ordering::SeqCst),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ByteLru<PcmKey, Vec<f32>>> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn invalidate_content(&self, namespace: u64, content: &str) {
        self.lock()
            .retain(|k| !(k.namespace == namespace && &*k.content == content));
    }

    pub fn invalidate_namespace(&self, namespace: u64) {
        self.lock().retain(|k| k.namespace != namespace);
    }

    fn key(src: &AudioSource, block: u64) -> PcmKey {
        PcmKey {
            namespace: src.namespace,
            content: Arc::clone(&src.content),
            stream: src.index.stream_index(),
            block,
            backend: PCM_BACKEND_VERSION,
        }
    }

    /// PCM f32 intercalado, na taxa/canais **nativos** do stream, exato por amostra. Devolve menos
    /// que o pedido (ou nada) só quando passa do fim.
    pub fn read(
        &self,
        src: &AudioSource,
        start_sample: u64,
        frames: u64,
        cancel: &dyn Fn() -> bool,
    ) -> Result<AudioPcm, MediaError> {
        let ix = &src.index;
        let ch = ix.channels();
        let total = ix.total_samples();
        let end = start_sample.saturating_add(frames).min(total);
        let mut out = AudioPcm {
            sample_rate: ix.sample_rate(),
            channels: ch,
            start_sample,
            frames: 0,
            samples: Vec::new(),
        };
        if frames == 0 || start_sample >= end {
            return Ok(out);
        }
        let b0 = start_sample / PCM_BLOCK_FRAMES;
        let b1 = (end - 1) / PCM_BLOCK_FRAMES;
        let mut blocks: Vec<Option<Arc<Vec<f32>>>> = Vec::new();
        {
            let mut c = self.lock();
            for b in b0..=b1 {
                blocks.push(c.get(&Self::key(src, b)));
            }
        }
        // decodifica cada corrida contígua de blocos ausentes com UMA chamada
        let mut i = 0usize;
        while i < blocks.len() {
            if blocks[i].is_some() {
                i += 1;
                continue;
            }
            let mut j = i;
            while j + 1 < blocks.len() && blocks[j + 1].is_none() {
                j += 1;
            }
            let first_block = b0 + i as u64;
            let last_block = b0 + j as u64;
            let from = first_block * PCM_BLOCK_FRAMES;
            let to = ((last_block + 1) * PCM_BLOCK_FRAMES).min(total);
            self.decode_calls.fetch_add(1, Ordering::SeqCst);
            let pcm = decode_audio_indexed(
                &self.toolchain,
                &src.path,
                ix,
                from,
                to - from,
                ch,
                self.timeout,
                cancel,
            )?;
            if pcm.frames != to - from {
                return Err(MediaError::new(
                    MediaErrorCode::MediaDecodeFailed,
                    format!(
                        "the audio decoder returned {} of {} frames",
                        pcm.frames,
                        to - from
                    ),
                ));
            }
            self.decoded_frames.fetch_add(pcm.frames, Ordering::SeqCst);
            let mut c = self.lock();
            for (k, b) in (first_block..=last_block).enumerate() {
                let lo = (b * PCM_BLOCK_FRAMES - from) as usize * ch as usize;
                let hi = (((b + 1) * PCM_BLOCK_FRAMES).min(to) - from) as usize * ch as usize;
                let data = Arc::new(pcm.samples[lo..hi].to_vec());
                c.insert(
                    Self::key(src, b),
                    Arc::clone(&data),
                    (data.len() * 4) as u64,
                );
                blocks[i + k] = Some(data);
            }
            i = j + 1;
        }
        let mut samples = Vec::with_capacity(((end - start_sample) * u64::from(ch)) as usize);
        for (k, b) in (b0..=b1).enumerate() {
            let data = blocks[k].as_ref().map(|d| d.as_slice()).unwrap_or(&[]);
            let block_start = b * PCM_BLOCK_FRAMES;
            let lo = start_sample.max(block_start) - block_start;
            let hi = end.min(block_start + PCM_BLOCK_FRAMES) - block_start;
            samples.extend_from_slice(
                &data[(lo * u64::from(ch)) as usize..(hi * u64::from(ch)) as usize],
            );
        }
        out.frames = end - start_sample;
        out.samples = samples;
        Ok(out)
    }
}
