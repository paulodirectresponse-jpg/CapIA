//! Índice de quadros (ADR-054): PTS/DTS/duração/keyframe **reais** de cada pacote de vídeo, em
//! ordem de apresentação. Nada é derivado de `frame / fps` — funciona para CFR, VFR, B-frames,
//! GOP longo e timestamps irregulares. É um dado **derivado** (cache), nunca essencial.
//!
//! Formato binário versionado `CIDX` v1 (little-endian): cabeçalho de 64 bytes (magic, versão,
//! tamanho do cabeçalho, nº de quadros, *time base*, stream, PTS iniciais derivados, `start_pts`),
//! `n × 24` bytes de entradas (`pts i64, dts i64, duração u32, flags u8, 3 de padding`) e um
//! rodapé SHA-256 de tudo que o precede. Arquivo truncado/corrompido ⇒ `MEDIA_INDEX_INVALID`
//! (o chamador regenera); nenhum caminho entra em pânico.

use crate::error::{MediaError, MediaErrorCode};
use crate::failpoints::fp;
use crate::ffprobe::{checked_input_path, file_url_arg};
use crate::process::{Flow, StreamLimits, run_streaming};
use crate::toolchain::MediaToolchain;
use capia_time::{Rational, TICKS_PER_SECOND, Ticks};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

pub const INDEX_MAGIC: [u8; 4] = *b"CIDX";
pub const INDEX_VERSION: u32 = 1;
/// Versão do FORMATO + do algoritmo de extração (entra na `CacheKey`).
pub const INDEX_PRODUCER: &str = "frame-index/1";
/// Teto de quadros de um índice (24 B cada ⇒ 240 MB).
pub const MAX_INDEX_FRAMES: u64 = 10_000_000;
const HEADER_LEN: usize = 64;
const ENTRY_LEN: usize = 24;
const FOOTER_LEN: usize = 32;

fn invalid(msg: impl Into<String>) -> MediaError {
    MediaError::new(MediaErrorCode::MediaIndexInvalid, msg)
}

/// Um quadro (em unidades do *time base* do stream).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameEntry {
    pub pts: i64,
    pub dts: i64,
    /// 0 = desconhecida.
    pub duration: u32,
    pub key: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameIndex {
    time_base: Rational,
    stream_index: u32,
    start_pts: i64,
    pts_derived: u32,
    entries: Vec<FrameEntry>,
}

impl FrameIndex {
    /// Valida e **ordena por apresentação** `(pts, dts)`. Duplicatas de PTS são mantidas (são
    /// quadros lógicos distintos).
    pub fn new(
        time_base: Rational,
        stream_index: u32,
        pts_derived: u32,
        mut entries: Vec<FrameEntry>,
    ) -> Result<Self, MediaError> {
        if !time_base.is_positive()
            || u32::try_from(time_base.num()).is_err()
            || u32::try_from(time_base.den()).is_err()
        {
            return Err(invalid("invalid time base"));
        }
        if entries.is_empty() {
            return Err(invalid("the stream has no frames"));
        }
        if entries.len() as u64 > MAX_INDEX_FRAMES {
            return Err(MediaError::new(
                MediaErrorCode::MediaLimitExceeded,
                format!("more than {MAX_INDEX_FRAMES} frames"),
            ));
        }
        entries.sort_by_key(|e| (e.pts, e.dts));
        let start_pts = entries[0].pts;
        // a diferença de qualquer PTS para o início precisa caber em i64 (aritmética checked)
        if entries
            .last()
            .is_some_and(|l| l.pts.checked_sub(start_pts).is_none())
        {
            return Err(invalid("timestamp range overflows"));
        }
        Ok(Self {
            time_base,
            stream_index,
            start_pts,
            pts_derived,
            entries,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn time_base(&self) -> Rational {
        self.time_base
    }

    pub fn stream_index(&self) -> u32 {
        self.stream_index
    }

    pub fn start_pts(&self) -> i64 {
        self.start_pts
    }

    pub fn pts_derived(&self) -> u32 {
        self.pts_derived
    }

    pub fn entries(&self) -> &[FrameEntry] {
        &self.entries
    }

    pub fn keyframe_count(&self) -> usize {
        self.entries.iter().filter(|e| e.key).count()
    }

    pub fn frame_by_index(&self, i: usize) -> Option<&FrameEntry> {
        self.entries.get(i)
    }

    /// Tempo (em ticks, **relativo ao primeiro quadro apresentado**) do quadro `i`, arredondado
    /// half-up quando o *time base* não divide o tick.
    pub fn time_of(&self, i: usize) -> Option<Ticks> {
        let e = self.entries.get(i)?;
        self.pts_to_ticks(e.pts)
    }

    fn pts_to_ticks(&self, pts: i64) -> Option<Ticks> {
        let rel = i128::from(pts) - i128::from(self.start_pts);
        let num = rel * i128::from(self.time_base.num()) * i128::from(TICKS_PER_SECOND);
        let den = i128::from(self.time_base.den());
        // half-up (floor((2n + d) / 2d))
        let t = (2 * num + den).div_euclid(2 * den);
        i64::try_from(t).ok().map(Ticks)
    }

    /// Compara o PTS de uma entrada com `t` **sem arredondar** (aritmética exata em i128).
    fn cmp_to(&self, e: &FrameEntry, t: Ticks) -> core::cmp::Ordering {
        let rel = i128::from(e.pts) - i128::from(self.start_pts);
        let lhs = rel * i128::from(self.time_base.num()) * i128::from(TICKS_PER_SECOND);
        let rhs = i128::from(t.0) * i128::from(self.time_base.den());
        lhs.cmp(&rhs)
    }

    /// Duração do trecho coberto: do primeiro quadro ao fim do último (PTS + duração).
    pub fn duration(&self) -> Option<Ticks> {
        let last = self.entries.last()?;
        let end = i128::from(last.pts) + i128::from(last.duration);
        let rel = end - i128::from(self.start_pts);
        let num = rel * i128::from(self.time_base.num()) * i128::from(TICKS_PER_SECOND);
        let den = i128::from(self.time_base.den());
        i64::try_from((2 * num + den).div_euclid(2 * den))
            .ok()
            .map(Ticks)
    }

    /// Último quadro com `tempo ≤ t` (empate de PTS ⇒ o último dos iguais). `None` se `t` é
    /// anterior ao primeiro quadro.
    pub fn frame_at_or_before(&self, t: Ticks) -> Option<usize> {
        let n = self
            .entries
            .partition_point(|e| self.cmp_to(e, t) != core::cmp::Ordering::Greater);
        n.checked_sub(1)
    }

    /// Primeiro quadro com `tempo ≥ t`. `None` se `t` é posterior ao último.
    pub fn frame_at_or_after(&self, t: Ticks) -> Option<usize> {
        let n = self
            .entries
            .partition_point(|e| self.cmp_to(e, t) == core::cmp::Ordering::Less);
        (n < self.entries.len()).then_some(n)
    }

    /// Quadro mais próximo de `t`; empate ⇒ o **anterior**.
    pub fn nearest_frame(&self, t: Ticks) -> usize {
        match (self.frame_at_or_before(t), self.frame_at_or_after(t)) {
            (Some(b), Some(a)) if a != b => {
                let (tb, ta) = (
                    self.time_of(b).map_or(0, |x| x.0),
                    self.time_of(a).map_or(0, |x| x.0),
                );
                if (t.0 - tb).abs() <= (ta - t.0).abs() {
                    b
                } else {
                    a
                }
            }
            (Some(b), _) => b,
            (None, Some(a)) => a,
            (None, None) => 0,
        }
    }

    /// Último keyframe em ordem de apresentação com `índice ≤ i` (ponto de partida do decode).
    pub fn keyframe_before(&self, i: usize) -> Option<usize> {
        let upto = i.min(self.entries.len().saturating_sub(1));
        (0..=upto).rev().find(|&k| self.entries[k].key)
    }

    // ---- formato binário -------------------------------------------------------------------

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.entries.len() * ENTRY_LEN + FOOTER_LEN);
        out.extend_from_slice(&INDEX_MAGIC);
        out.extend_from_slice(&INDEX_VERSION.to_le_bytes());
        out.extend_from_slice(&(HEADER_LEN as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // flags reservadas
        out.extend_from_slice(&(self.entries.len() as u64).to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(self.time_base.num())
                .unwrap_or(1)
                .to_le_bytes(),
        );
        out.extend_from_slice(
            &u32::try_from(self.time_base.den())
                .unwrap_or(1)
                .to_le_bytes(),
        );
        out.extend_from_slice(&self.stream_index.to_le_bytes());
        out.extend_from_slice(&self.pts_derived.to_le_bytes());
        out.extend_from_slice(&self.start_pts.to_le_bytes());
        out.resize(HEADER_LEN, 0);
        for e in &self.entries {
            out.extend_from_slice(&e.pts.to_le_bytes());
            out.extend_from_slice(&e.dts.to_le_bytes());
            out.extend_from_slice(&e.duration.to_le_bytes());
            out.push(u8::from(e.key));
            out.extend_from_slice(&[0, 0, 0]);
        }
        let sum = Sha256::digest(&out);
        out.extend_from_slice(&sum);
        out
    }

    /// Decodifica **validando tudo** (tamanho, versão, contagem, checksum, ordem, padding).
    pub fn decode(bytes: &[u8]) -> Result<Self, MediaError> {
        if bytes.len() < HEADER_LEN + FOOTER_LEN {
            return Err(invalid("index file is truncated"));
        }
        if bytes[..4] != INDEX_MAGIC {
            return Err(invalid("not a frame index (bad magic)"));
        }
        let u32_at =
            |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        let u64_at = |o: usize| {
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes[o..o + 8]);
            u64::from_le_bytes(b)
        };
        let i64_at = |o: usize| i64::from_le_bytes(u64_at(o).to_le_bytes());
        if u32_at(4) != INDEX_VERSION {
            return Err(invalid(format!("unsupported index version {}", u32_at(4))));
        }
        if u32_at(8) as usize != HEADER_LEN {
            return Err(invalid("unexpected header length"));
        }
        let count = u64_at(16);
        if count == 0 || count > MAX_INDEX_FRAMES {
            return Err(invalid("implausible frame count"));
        }
        let expected = (count as usize)
            .checked_mul(ENTRY_LEN)
            .and_then(|b| b.checked_add(HEADER_LEN + FOOTER_LEN))
            .ok_or_else(|| invalid("size overflow"))?;
        if bytes.len() != expected {
            return Err(invalid(
                "index file has the wrong size (truncated or padded)",
            ));
        }
        let body = &bytes[..bytes.len() - FOOTER_LEN];
        if Sha256::digest(body).as_slice() != &bytes[bytes.len() - FOOTER_LEN..] {
            return Err(invalid("index checksum mismatch"));
        }
        let tb = Rational::new(i64::from(u32_at(24)), i64::from(u32_at(28)))
            .map_err(|_| invalid("invalid time base"))?;
        let mut entries = Vec::with_capacity(count as usize);
        for k in 0..count as usize {
            let o = HEADER_LEN + k * ENTRY_LEN;
            if bytes[o + 21..o + 24] != [0, 0, 0] || bytes[o + 20] > 1 {
                return Err(invalid("corrupt entry flags"));
            }
            entries.push(FrameEntry {
                pts: i64_at(o),
                dts: i64_at(o + 8),
                duration: u32_at(o + 16),
                key: bytes[o + 20] == 1,
            });
        }
        if entries
            .windows(2)
            .any(|w| (w[0].pts, w[0].dts) > (w[1].pts, w[1].dts))
        {
            return Err(invalid("entries are not in presentation order"));
        }
        let idx = Self::new(tb, u32_at(32), u32_at(36), entries)?;
        if idx.start_pts != i64_at(40) {
            return Err(invalid("start_pts does not match the entries"));
        }
        Ok(idx)
    }
}

// ---------------------------------------------------------------------------------------------
// extração (ffprobe em streaming; nunca JSON gigante)
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct IndexOptions {
    pub timeout: Duration,
    pub max_frames: u64,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(3600),
            max_frames: MAX_INDEX_FRAMES,
        }
    }
}

/// Acumula linhas CSV de `packet=pts,dts,duration,flags` sem guardar a saída inteira.
struct PacketParser {
    carry: Vec<u8>,
    entries: Vec<FrameEntry>,
    derived: u32,
    skipped: u64,
    lines: u64,
    max_frames: u64,
}

impl PacketParser {
    fn feed(&mut self, chunk: &[u8]) -> Result<(), MediaError> {
        let mut start = 0;
        for (i, b) in chunk.iter().enumerate() {
            if *b == b'\n' {
                if self.carry.is_empty() {
                    self.line(&chunk[start..i])?;
                } else {
                    self.carry.extend_from_slice(&chunk[start..i]);
                    let line = std::mem::take(&mut self.carry);
                    self.line(&line)?;
                }
                start = i + 1;
            }
        }
        self.carry.extend_from_slice(&chunk[start..]);
        if self.carry.len() > 512 {
            return Err(invalid("a packet line is implausibly long"));
        }
        Ok(())
    }

    fn line(&mut self, raw: &[u8]) -> Result<(), MediaError> {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            return Ok(());
        }
        self.lines += 1;
        let text = std::str::from_utf8(line).map_err(|_| invalid("non-UTF-8 packet line"))?;
        let mut f = text.split(',');
        let (pts, dts, dur, flags) = (f.next(), f.next(), f.next(), f.next());
        let (Some(pts), Some(dts), Some(dur), Some(flags)) = (pts, dts, dur, flags) else {
            self.skipped += 1;
            return Ok(());
        };
        let num = |s: &str| -> Option<i64> {
            if s == "N/A" {
                None
            } else {
                s.trim().parse().ok()
            }
        };
        let (pts_v, dts_v) = (num(pts), num(dts));
        let (pts_f, derived) = match (pts_v, dts_v) {
            (Some(p), _) => (p, false),
            (None, Some(d)) => (d, true),
            (None, None) => {
                self.skipped += 1;
                return Ok(());
            }
        };
        if self.entries.len() as u64 >= self.max_frames {
            return Err(MediaError::new(
                MediaErrorCode::MediaLimitExceeded,
                format!("more than {} frames", self.max_frames),
            ));
        }
        self.derived += u32::from(derived);
        self.entries.push(FrameEntry {
            pts: pts_f,
            dts: dts_v.unwrap_or(pts_f),
            duration: num(dur).and_then(|d| u32::try_from(d).ok()).unwrap_or(0),
            key: flags.starts_with('K'),
        });
        Ok(())
    }
}

/// Constrói o índice de `stream_index` (índice **absoluto** do stream) lendo os pacotes com o
/// ffprobe em streaming. `progress(done, total_hint)`; `cancel()` mata o processo.
#[allow(clippy::too_many_arguments)]
pub fn build_frame_index(
    toolchain: &MediaToolchain,
    path: &Path,
    stream_index: u32,
    time_base: Rational,
    frames_hint: u64,
    opts: &IndexOptions,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<FrameIndex, MediaError> {
    let abs = checked_input_path(path)?;
    let mut args: Vec<OsString> = ["-v", "error", "-select_streams"]
        .iter()
        .map(OsString::from)
        .collect();
    args.push(OsString::from(stream_index.to_string()));
    for a in [
        "-show_entries",
        "packet=pts,dts,duration,flags",
        "-of",
        "csv=p=0",
        "-protocol_whitelist",
        "file",
        "-i",
    ] {
        args.push(OsString::from(a));
    }
    args.push(file_url_arg(&abs));
    let mut parser = PacketParser {
        carry: Vec::new(),
        entries: Vec::new(),
        derived: 0,
        skipped: 0,
        lines: 0,
        max_frames: opts.max_frames,
    };
    let mut reported = 0u64;
    let out = run_streaming(
        &toolchain.ffprobe,
        &args,
        &StreamLimits::new(opts.timeout),
        cancel,
        &mut |chunk| {
            fp!("index_running");
            parser.feed(chunk)?;
            let n = parser.entries.len() as u64;
            if n >= reported + 4096 {
                reported = n;
                progress(n, frames_hint);
            }
            Ok(Flow::Continue)
        },
    )?;
    if let Some(status) = out.status
        && !status.success()
    {
        return Err(MediaError::new(
            MediaErrorCode::MediaProbeFailed,
            "ffprobe could not read the packets of this stream",
        ));
    }
    if !parser.carry.is_empty() {
        let line = std::mem::take(&mut parser.carry);
        parser.line(&line)?;
    }
    // mais de 1 % de linhas inutilizáveis ⇒ o stream não é indexável com confiança
    if parser.skipped * 100 > parser.lines.max(1) {
        return Err(invalid(format!(
            "{} of {} packets have no usable timestamp",
            parser.skipped, parser.lines
        )));
    }
    progress(
        parser.entries.len() as u64,
        frames_hint.max(parser.entries.len() as u64),
    );
    FrameIndex::new(time_base, stream_index, parser.derived, parser.entries)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn e(pts: i64, dts: i64, key: bool) -> FrameEntry {
        FrameEntry {
            pts,
            dts,
            duration: 1,
            key,
        }
    }

    /// 25 fps em time base 1/25: pts = n.
    fn cfr(n: i64) -> FrameIndex {
        FrameIndex::new(
            Rational::new(1, 25).unwrap(),
            0,
            0,
            (0..n).map(|i| e(i, i - 2, i % 10 == 0)).collect(),
        )
        .unwrap()
    }

    fn t(sec_num: i64, sec_den: i64) -> Ticks {
        Ticks(sec_num * TICKS_PER_SECOND / sec_den)
    }

    #[test]
    fn lookups_follow_real_timestamps_with_exact_boundaries() {
        let ix = cfr(50);
        assert_eq!(ix.len(), 50);
        assert_eq!(ix.time_of(25), Some(t(1, 1)));
        // exatamente no quadro
        assert_eq!(ix.frame_at_or_before(t(1, 1)), Some(25));
        assert_eq!(ix.frame_at_or_after(t(1, 1)), Some(25));
        // 1 tick antes do quadro 25 ⇒ antes: 24; depois: 25
        let just_before = Ticks(t(1, 1).0 - 1);
        assert_eq!(ix.frame_at_or_before(just_before), Some(24));
        assert_eq!(ix.frame_at_or_after(just_before), Some(25));
        // 1 tick depois ⇒ antes: 25; depois: 26
        let just_after = Ticks(t(1, 1).0 + 1);
        assert_eq!(ix.frame_at_or_before(just_after), Some(25));
        assert_eq!(ix.frame_at_or_after(just_after), Some(26));
        // limites
        assert_eq!(ix.frame_at_or_before(Ticks(-1)), None);
        assert_eq!(ix.frame_at_or_before(Ticks::ZERO), Some(0));
        assert_eq!(ix.frame_at_or_after(t(49, 25)), Some(49));
        assert_eq!(ix.frame_at_or_after(Ticks(t(49, 25).0 + 1)), None);
        assert_eq!(ix.frame_at_or_before(Ticks(i64::MAX)), Some(49));
        // nearest: empate vai para o anterior
        let mid = Ticks((t(1, 1).0 + t(26, 25).0) / 2);
        assert_eq!(ix.nearest_frame(mid), 25);
        assert_eq!(ix.nearest_frame(Ticks(mid.0 + 1)), 26);
        assert_eq!(ix.nearest_frame(Ticks(-5)), 0);
        assert_eq!(ix.nearest_frame(Ticks(i64::MAX)), 49);
    }

    #[test]
    fn keyframe_lookup_and_by_index() {
        let ix = cfr(35);
        assert_eq!(ix.keyframe_before(0), Some(0));
        assert_eq!(ix.keyframe_before(9), Some(0));
        assert_eq!(ix.keyframe_before(10), Some(10));
        assert_eq!(ix.keyframe_before(34), Some(30));
        assert_eq!(
            ix.keyframe_before(10_000),
            Some(30),
            "index clamps to the last frame"
        );
        assert_eq!(ix.keyframe_count(), 4);
        assert!(ix.frame_by_index(34).is_some() && ix.frame_by_index(35).is_none());
    }

    #[test]
    fn vfr_gaps_hold_the_previous_frame() {
        // timestamps irregulares (time base 1/1000): 0, 40, 80, 400, 440, 1000
        let pts = [0, 40, 80, 400, 440, 1000];
        let ix = FrameIndex::new(
            Rational::new(1, 1000).unwrap(),
            0,
            0,
            pts.iter().map(|&p| e(p, p, p == 0)).collect(),
        )
        .unwrap();
        let ms = |m: i64| Ticks(m * TICKS_PER_SECOND / 1000);
        assert_eq!(
            ix.frame_at_or_before(ms(399)),
            Some(2),
            "gap: still frame at 80 ms"
        );
        assert_eq!(ix.frame_at_or_before(ms(400)), Some(3));
        assert_eq!(ix.frame_at_or_before(ms(999)), Some(4));
        assert_eq!(ix.frame_at_or_after(ms(81)), Some(3));
        assert_ne!(
            ix.frame_at_or_before(ms(200)),
            Some(5),
            "never derived from index/fps"
        );
        // a regra "índice / fps" daria outra resposta: 200 ms · 25 fps = frame 5
        assert_eq!(ix.nearest_frame(ms(230)), 2);
        assert_eq!(ix.nearest_frame(ms(250)), 3);
    }

    #[test]
    fn b_frames_and_negative_dts_are_ordered_by_presentation_time() {
        // ordem de decodificação I P B B (pts 0, 3, 1, 2; dts -1, 2, 0, 1)
        let ix = FrameIndex::new(
            Rational::new(1, 25).unwrap(),
            0,
            0,
            vec![
                e(0, -1, true),
                e(3, 2, false),
                e(1, 0, false),
                e(2, 1, false),
            ],
        )
        .unwrap();
        let pts: Vec<i64> = ix.entries().iter().map(|x| x.pts).collect();
        assert_eq!(pts, [0, 1, 2, 3]);
        assert_eq!(ix.keyframe_before(3), Some(0));
    }

    #[test]
    fn duplicate_and_non_monotonic_pts_are_kept_as_distinct_frames() {
        let ix = FrameIndex::new(
            Rational::new(1, 25).unwrap(),
            0,
            0,
            vec![
                e(5, 5, false),
                e(5, 4, false),
                e(2, 2, true),
                e(7, 7, false),
            ],
        )
        .unwrap();
        assert_eq!(ix.len(), 4);
        let t5 = Ticks(3 * TICKS_PER_SECOND / 25);
        // dois quadros no mesmo PTS: o "at or before" devolve o ÚLTIMO dos iguais, o "after" o primeiro
        assert_eq!(ix.frame_at_or_before(t5), Some(2));
        assert_eq!(ix.frame_at_or_after(t5), Some(1));
    }

    #[test]
    fn start_offset_is_relative_to_the_first_presented_frame() {
        let ix = FrameIndex::new(
            Rational::new(1, 1000).unwrap(),
            0,
            0,
            vec![e(1500, 1500, true), e(1540, 1540, false)],
        )
        .unwrap();
        assert_eq!(ix.time_of(0), Some(Ticks::ZERO));
        assert_eq!(ix.start_pts(), 1500);
        assert_eq!(ix.time_of(1), Some(Ticks(40 * TICKS_PER_SECOND / 1000)));
    }

    #[test]
    fn extreme_timestamps_never_overflow() {
        let r = FrameIndex::new(
            Rational::new(1, 90_000).unwrap(),
            0,
            0,
            vec![e(i64::MIN, 0, true), e(i64::MAX, 0, false)],
        );
        assert!(r.is_err(), "range overflow is rejected");
        let ix = FrameIndex::new(
            Rational::new(1, 90_000).unwrap(),
            0,
            0,
            vec![e(0, 0, true), e(i64::MAX / 2, 0, false)],
        )
        .unwrap();
        // conversão fora do alcance de Ticks ⇒ None, sem pânico
        assert!(ix.time_of(1).is_none());
        let _ = ix.frame_at_or_before(Ticks(i64::MAX));
        let _ = ix.nearest_frame(Ticks(i64::MIN));
    }

    #[test]
    fn rejects_empty_and_bad_time_bases() {
        assert!(FrameIndex::new(Rational::new(1, 25).unwrap(), 0, 0, vec![]).is_err());
        assert!(FrameIndex::new(Rational::ZERO, 0, 0, vec![e(0, 0, true)]).is_err());
    }

    #[test]
    fn binary_format_round_trips_exactly() {
        let ix = cfr(1000);
        let bytes = ix.encode();
        assert_eq!(bytes.len(), 64 + 24 * 1000 + 32);
        assert_eq!(&bytes[..4], b"CIDX");
        assert_eq!(FrameIndex::decode(&bytes).unwrap(), ix);
    }

    #[test]
    fn every_truncation_and_every_single_byte_flip_is_rejected_without_panic() {
        let ix = cfr(20);
        let bytes = ix.encode();
        for len in 0..bytes.len() {
            assert!(
                FrameIndex::decode(&bytes[..len]).is_err(),
                "truncated at {len}"
            );
        }
        for i in 0..bytes.len() {
            let mut b = bytes.clone();
            b[i] ^= 0x01;
            assert!(
                FrameIndex::decode(&b).is_err(),
                "flip at byte {i} was accepted"
            );
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(FrameIndex::decode(&longer).is_err());
        // contagem absurda no cabeçalho não pode alocar
        let mut huge = bytes;
        huge[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(FrameIndex::decode(&huge).is_err());
    }

    #[test]
    fn the_packet_parser_streams_across_arbitrary_chunk_boundaries() {
        let text = "0,-2,1,K__\n1,N/A,1,___\nN/A,3,1,___\n2,0,N/A,___\n";
        for cut in 0..text.len() {
            let mut p = PacketParser {
                carry: Vec::new(),
                entries: Vec::new(),
                derived: 0,
                skipped: 0,
                lines: 0,
                max_frames: 100,
            };
            p.feed(&text.as_bytes()[..cut]).unwrap();
            p.feed(&text.as_bytes()[cut..]).unwrap();
            assert_eq!(p.entries.len(), 4, "cut {cut}");
            assert_eq!(p.derived, 1);
            assert!(p.entries[0].key && !p.entries[1].key);
            assert_eq!(p.entries[2].pts, 3, "pts derived from dts");
            assert_eq!(p.entries[3].duration, 0);
        }
    }

    #[test]
    fn the_parser_rejects_floods_and_garbage_lines() {
        let mut p = PacketParser {
            carry: Vec::new(),
            entries: Vec::new(),
            derived: 0,
            skipped: 0,
            lines: 0,
            max_frames: 2,
        };
        assert!(
            p.feed(b"0,0,1,K\n1,1,1,_\n2,2,1,_\n").is_err(),
            "frame limit"
        );
        let mut p = PacketParser {
            carry: Vec::new(),
            entries: Vec::new(),
            derived: 0,
            skipped: 0,
            lines: 0,
            max_frames: 10,
        };
        assert!(
            p.feed(&vec![b'x'; 600]).is_err(),
            "line without newline is bounded"
        );
        let mut p = PacketParser {
            carry: Vec::new(),
            entries: Vec::new(),
            derived: 0,
            skipped: 0,
            lines: 0,
            max_frames: 10,
        };
        p.feed(b"garbage\n,,,\nN/A,N/A,1,_\n").unwrap();
        assert_eq!((p.entries.len(), p.skipped), (0, 3));
        assert!(p.feed(&[0xff, 0xfe, b'\n']).is_err());
    }
}
