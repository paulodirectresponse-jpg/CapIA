//! Detecção de cenas **local** e determinística (Fase 4, ADR-083): cortes secos, dissolves/fades,
//! rejeição de *flash* (falso positivo) e de movimento de câmera sem corte.
//!
//! Entrada: quadros RGB24 minúsculos a taxa constante (ver `capia_media::decode_small_frames`).
//! Por quadro guardamos um histograma (3×16 bins) e a luminância média; entre quadros medimos
//! diferença de pixel (média absoluta) e de histograma (L1). O histograma é o que separa **troca de
//! conteúdo** (dissolve, corte) de **translação** (pan/zoom suave): uma panorâmica preserva o
//! histograma, uma troca de cena não.

use serde::{Deserialize, Serialize};

pub const SCAN_WIDTH: u32 = 64;
pub const SCAN_HEIGHT: u32 = 36;
const BINS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryKind {
    Cut,
    Dissolve,
    Fade,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Boundary {
    /// Índice (quadro amostrado) do **primeiro** quadro da nova cena (corte) ou do meio da transição.
    pub frame: u64,
    pub kind: BoundaryKind,
    /// Duração da transição em quadros (1 para corte seco).
    pub span: u32,
    /// Confiança (0..1) — força do sinal sobre o limiar.
    pub score: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneParams {
    /// Limiar do escore de corte seco (mistura de pixel + histograma).
    pub cut_threshold: f32,
    /// Quantas vezes acima da atividade local o salto precisa estar.
    pub cut_ratio: f32,
    /// Diferença mínima de pixel para um salto ser corte: fade/dissolve mexem no histograma mas
    /// quase não na estrutura espacial por quadro.
    pub cut_min_pixel: f32,
    /// Salto só de histograma (conteúdo escuro/de baixo contraste): vale como corte se ≥ isto,
    /// se for isolado (os quadros seguintes voltam à calma) e destoar da atividade local.
    pub cut_hist_only: f32,
    /// Janela (quadros) de atividade local.
    pub local_window: usize,
    /// Flash: se o quadro `i-1` e um quadro até `flash_max` depois se parecem, o salto é transitório.
    pub flash_max: usize,
    pub flash_similarity: f32,
    /// Transição gradual: início quando o histograma passa deste valor entre quadros vizinhos…
    pub grad_low: f32,
    /// …e vale se a distância de histograma entre início e fim passar deste.
    pub grad_total: f32,
    /// …e o passo inicial precisa destoar da atividade local de histograma (descarta deriva lenta).
    pub grad_ratio: f32,
    pub grad_min_frames: usize,
    pub grad_max_frames: usize,
}

impl Default for SceneParams {
    fn default() -> Self {
        Self {
            cut_threshold: 0.22,
            cut_ratio: 3.5,
            cut_min_pixel: 0.08,
            cut_hist_only: 0.25,
            local_window: 8,
            flash_max: 4,
            flash_similarity: 0.10,
            grad_low: 0.015,
            grad_total: 0.30,
            grad_ratio: 2.5,
            grad_min_frames: 4,
            grad_max_frames: 90,
        }
    }
}

/// Características de um quadro (o quadro bruto é descartado depois do próximo par).
#[derive(Clone, Debug)]
struct Feat {
    hist: [f32; BINS * 3],
    luma: f32,
}

fn feat(rgb: &[u8]) -> Feat {
    let mut hist = [0f32; BINS * 3];
    let mut luma = 0f64;
    let px = rgb.len() / 3;
    for p in rgb.chunks_exact(3) {
        for (c, v) in p.iter().enumerate() {
            hist[c * BINS + (*v as usize * BINS / 256)] += 1.0;
        }
        luma += 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]);
    }
    let n = px.max(1) as f32;
    for h in &mut hist {
        *h /= n;
    }
    Feat {
        hist,
        luma: (luma / (px.max(1) as f64 * 255.0)) as f32,
    }
}

fn hist_dist(a: &Feat, b: &Feat) -> f32 {
    // cada canal soma 1 ⇒ L1 por canal ∈ [0,2]; média dos 3 canais / 2 ∈ [0,1]
    let s: f32 = a.hist.iter().zip(&b.hist).map(|(x, y)| (x - y).abs()).sum();
    s / 6.0
}

fn pix_dist(a: &[u8], b: &[u8]) -> f32 {
    let s: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    s as f32 / (a.len().max(1) as f32 * 255.0)
}

/// Escore combinado de "quão diferentes são dois quadros".
fn score(pd: f32, hd: f32) -> f32 {
    0.5 * pd + 0.5 * (hd * 2.0).min(1.0)
}

/// Acumulador em **streaming**: guarda só características por quadro (~200 B), nunca os quadros;
/// um anel dos últimos quadros permite o teste de flash sem reter a sequência (memória O(n·200 B)).
#[derive(Debug)]
pub struct SceneAnalyzer {
    params: SceneParams,
    ring: std::collections::VecDeque<Vec<u8>>, // últimos `flash_max + 1` quadros
    feats: Vec<Feat>,
    pd: Vec<f32>,
    hd: Vec<f32>,
    sc: Vec<f32>,
    /// `flash[i][l-1]` = escore entre o quadro `i-1` e o quadro `i+l`.
    flash: Vec<[f32; MAX_FLASH]>,
}

const MAX_FLASH: usize = 8;

impl SceneAnalyzer {
    pub fn new(params: SceneParams) -> Self {
        let mut params = params;
        params.flash_max = params.flash_max.min(MAX_FLASH);
        Self {
            params,
            ring: std::collections::VecDeque::new(),
            feats: Vec::new(),
            pd: Vec::new(),
            hd: Vec::new(),
            sc: Vec::new(),
            flash: Vec::new(),
        }
    }

    /// Séries brutas (pixel, histograma) por quadro — diagnóstico/ajuste.
    pub fn series(&self) -> (&[f32], &[f32]) {
        (&self.pd, &self.hd)
    }

    pub fn frames(&self) -> usize {
        self.feats.len()
    }

    /// Acrescenta o próximo quadro RGB24 `SCAN_WIDTH×SCAN_HEIGHT`.
    pub fn push(&mut self, rgb: &[u8]) {
        let f = feat(rgb);
        let j = self.feats.len();
        if let Some(prev) = self.ring.back() {
            let pdv = pix_dist(prev, rgb);
            let hdv = hist_dist(&self.feats[j - 1], &f);
            self.pd.push(pdv);
            self.hd.push(hdv);
            self.sc.push(score(pdv, hdv));
        } else {
            self.pd.push(0.0);
            self.hd.push(0.0);
            self.sc.push(0.0);
        }
        self.flash.push([f32::MAX; MAX_FLASH]);
        // quadro j é o "i+l" de i = j-l; precisa do quadro i-1 = j-l-1
        let ring_len = self.ring.len();
        for l in 1..=self.params.flash_max {
            if j > l {
                let i = j - l;
                let back = j - l - 1; // índice absoluto do quadro i-1
                let ring_start = j - ring_len; // índice absoluto do primeiro no anel
                if back >= ring_start {
                    let old = &self.ring[back - ring_start];
                    self.flash[i][l - 1] =
                        score(pix_dist(old, rgb), hist_dist(&self.feats[back], &f));
                }
            }
        }
        self.feats.push(f);
        self.ring.push_back(rgb.to_vec());
        while self.ring.len() > self.params.flash_max + 1 {
            self.ring.pop_front();
        }
    }

    pub fn finish(self) -> Vec<Boundary> {
        let p = self.params;
        let n = self.feats.len();
        if n < 3 {
            return Vec::new();
        }
        let (feats, pd, hd, sc, flash) = (&self.feats, &self.pd, &self.hd, &self.sc, &self.flash);
        let local = |i: usize| -> f32 {
            let from = i.saturating_sub(p.local_window).max(1);
            if from >= i {
                return 0.0;
            }
            let mut v: Vec<f32> = sc[from..i].to_vec();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            v[v.len() / 2]
        };
        let local_hd = |i: usize| -> f32 {
            let from = i.saturating_sub(p.local_window).max(1);
            if from >= i {
                return 0.0;
            }
            let mut v: Vec<f32> = hd[from..i].to_vec();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            v[v.len() / 2]
        };
        let is_cut = |i: usize| -> bool {
            if pd[i] >= p.cut_min_pixel {
                return sc[i] >= p.cut_threshold && sc[i] >= p.cut_ratio * local(i).max(0.01);
            }
            // só histograma: isolado e fora do padrão local (descarta passos de dissolve/fade)
            hd[i] >= p.cut_hist_only
                && hd[i] >= p.cut_ratio * local_hd(i).max(0.01)
                && (i + 1 >= n || hd[i + 1] < 0.4 * hd[i])
                && (i + 2 >= n || hd[i + 2] < 0.5 * hd[i])
        };

        let mut out: Vec<Boundary> = Vec::new();
        let mut blocked_until = 0usize; // índices < blocked_until não geram novas fronteiras
        let mut i = 1usize;
        while i < n {
            if i < blocked_until {
                i += 1;
                continue;
            }
            // ---- corte seco ---------------------------------------------------------------
            if is_cut(i) {
                // flash/transiente: o quadro i-1 reaparece logo depois?
                let mut flash_len = None;
                for l in 1..=p.flash_max {
                    if i + l < n && flash[i][l - 1] < p.flash_similarity {
                        flash_len = Some(l);
                        break;
                    }
                }
                if let Some(l) = flash_len {
                    blocked_until = i + l + 1; // ignora o salto de entrada e o de saída
                    i += 1;
                    continue;
                }
                out.push(Boundary {
                    frame: i as u64,
                    kind: BoundaryKind::Cut,
                    span: 1,
                    score: ((sc[i] / p.cut_threshold) - 1.0).clamp(0.0, 1.0),
                });
                blocked_until = i + 2;
                i += 1;
                continue;
            }
            // ---- transição gradual (twin comparison por histograma) ----------------------------
            if hd[i] >= p.grad_low && pd[i] < p.cut_min_pixel && hd[i] >= p.grad_ratio * local_hd(i)
            {
                let start = i - 1;
                let mut end = i;
                let mut calm = 0usize;
                let (mut sum, mut cnt) = (hd[i], 1usize);
                let mut cut_inside: Option<usize> = None;
                let mut j = i + 1;
                while j < n && j - start <= p.grad_max_frames {
                    // histerese: fundo (movimento lento) bem abaixo da média da transição = calmo
                    let floor = p.grad_low.max(0.35 * sum / cnt as f32);
                    if hd[j] >= floor {
                        end = j;
                        calm = 0;
                        sum += hd[j];
                        cnt += 1;
                    } else {
                        calm += 1;
                        // segurar preto/branco (fade) não encerra a transição
                        let held = feats[j].luma < 0.08 || feats[j].luma > 0.92;
                        if calm >= if held { 4 } else { 2 } {
                            break;
                        }
                    }
                    // um salto grande no meio descaracteriza a transição (é um corte)
                    if is_cut(j) {
                        cut_inside = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(c) = cut_inside {
                    i = c; // o salto é um corte: reavalia a partir dele
                    continue;
                }
                let len = end - start;
                let total = hist_dist(&feats[start], &feats[end]);
                // fade-in no começo / fade-out no fim do material não separa duas cenas
                let at_edge = start <= 1 || end + 2 >= n;
                if len >= p.grad_min_frames && total >= p.grad_total && !at_edge {
                    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
                    for f in &feats[start..=end] {
                        lo = lo.min(f.luma);
                        hi = hi.max(f.luma);
                    }
                    let through_black = lo < 0.08 || hi > 0.92;
                    out.push(Boundary {
                        frame: (start + len / 2) as u64,
                        kind: if through_black {
                            BoundaryKind::Fade
                        } else {
                            BoundaryKind::Dissolve
                        },
                        span: len as u32,
                        score: ((total / p.grad_total) - 1.0).clamp(0.0, 1.0),
                    });
                    blocked_until = end + 2;
                    i = end + 1;
                    continue;
                }
            }
            i += 1;
        }
        out
    }
}

/// Detecta as fronteiras de cena de uma sequência de quadros RGB24 `SCAN_WIDTH×SCAN_HEIGHT`.
pub fn detect(frames: &[Vec<u8>], p: &SceneParams) -> Vec<Boundary> {
    let mut a = SceneAnalyzer::new(*p);
    for f in frames {
        a.push(f);
    }
    a.finish()
}

/// Varre um arquivo de vídeo (ffmpeg, taxa constante) e detecta as cenas. Streaming: memória
/// proporcional ao número de quadros × ~200 B.
pub fn detect_media(
    tc: &capia_media::MediaToolchain,
    path: &std::path::Path,
    stream_index: u32,
    fps: (u32, u32),
    params: &SceneParams,
    timeout: std::time::Duration,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<Boundary>, capia_media::MediaError> {
    let mut a = SceneAnalyzer::new(*params);
    capia_media::decode_small_frames(
        tc,
        path,
        stream_index,
        fps.0,
        fps.1,
        SCAN_WIDTH,
        SCAN_HEIGHT,
        timeout,
        cancel,
        &mut |_, rgb| {
            a.push(rgb);
            Ok(capia_media::Flow::Continue)
        },
    )?;
    Ok(a.finish())
}

/// Fronteira anotada à mão/por construção (corpus de referência).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotated {
    pub frame: u64,
    pub kind: BoundaryKind,
    /// Tolerância (± quadros) para casar a detecção com esta anotação.
    pub tolerance: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneEval {
    pub truth: usize,
    pub detected: usize,
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
    /// `(FP + FN) / truth` — a métrica do critério da Fase 4 (≤ 5 %).
    pub error_rate: f64,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    /// Erro de contagem relativo `|detected − truth| / truth`.
    pub count_error: f64,
}

/// Casamento guloso por proximidade (cada detecção casa no máximo uma anotação).
pub fn evaluate(truth: &[Annotated], detected: &[Boundary]) -> SceneEval {
    let mut used = vec![false; detected.len()];
    let mut tp = 0usize;
    for t in truth {
        let mut best: Option<(usize, u64)> = None;
        for (k, d) in detected.iter().enumerate() {
            if used[k] {
                continue;
            }
            let dist = d.frame.abs_diff(t.frame);
            if dist <= u64::from(t.tolerance) && best.is_none_or(|(_, bd)| dist < bd) {
                best = Some((k, dist));
            }
        }
        if let Some((k, _)) = best {
            used[k] = true;
            tp += 1;
        }
    }
    let fp = detected.len() - tp;
    let fn_ = truth.len() - tp;
    let ratio = |a: usize, b: usize| if b == 0 { 1.0 } else { a as f64 / b as f64 };
    let precision = ratio(tp, detected.len());
    let recall = ratio(tp, truth.len());
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };
    SceneEval {
        truth: truth.len(),
        detected: detected.len(),
        true_positives: tp,
        false_positives: fp,
        false_negatives: fn_,
        error_rate: if truth.is_empty() {
            fp as f64
        } else {
            (fp + fn_) as f64 / truth.len() as f64
        },
        precision,
        recall,
        f1,
        count_error: if truth.is_empty() {
            detected.len() as f64
        } else {
            detected.len().abs_diff(truth.len()) as f64 / truth.len() as f64
        },
    }
}

/// Tempo (ticks) de um quadro amostrado a `fps_num/fps_den`.
pub fn frame_ticks(frame: u64, fps_num: u32, fps_den: u32) -> i64 {
    let t = u128::from(frame) * capia_time::TICKS_PER_SECOND as u128 * u128::from(fps_den)
        / u128::from(fps_num.max(1));
    i64::try_from(t).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    const W: usize = SCAN_WIDTH as usize;
    const H: usize = SCAN_HEIGHT as usize;

    /// Textura determinística (gradiente + xadrez) parametrizada por `seed` e deslocamento `dx`.
    fn tex(seed: u32, dx: usize, tint: [u8; 3]) -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 3);
        for y in 0..H {
            for x in 0..W {
                let xx = (x + dx) % (W * 4);
                let checker = ((xx / 8 + y / 8 + seed as usize) % 2) as u8;
                let g = (((xx % W) * 255 / W) as u32 * (1 + seed % 3) % 256) as u8;
                // gradiente vertical espalha o histograma (vídeo real não é feito de 2 cores)
                let gy = (y * 255 / H) as u8;
                v.push(
                    tint[0]
                        .saturating_add(checker * 60)
                        .saturating_add(g / 4)
                        .saturating_add(gy / 3),
                );
                v.push(tint[1].saturating_add(checker * 30).saturating_add(gy / 4));
                v.push(tint[2].saturating_add(g / 3).saturating_add(gy / 5));
            }
        }
        v
    }

    fn blend(a: &[u8], b: &[u8], t: f32) -> Vec<u8> {
        a.iter()
            .zip(b)
            .map(|(x, y)| (f32::from(*x) * (1.0 - t) + f32::from(*y) * t) as u8)
            .collect()
    }

    fn scene(n: usize, seed: u32, tint: [u8; 3]) -> Vec<Vec<u8>> {
        // ruído/movimento pequeno: desloca 1px por quadro
        (0..n).map(|i| tex(seed, i, tint)).collect()
    }

    #[test]
    fn hard_cuts_are_found_at_the_right_frame() {
        let mut f = scene(30, 0, [10, 10, 10]);
        f.extend(scene(30, 1, [120, 60, 20]));
        f.extend(scene(30, 2, [30, 140, 160]));
        let b = detect(&f, &SceneParams::default());
        let cuts: Vec<u64> = b
            .iter()
            .filter(|x| x.kind == BoundaryKind::Cut)
            .map(|x| x.frame)
            .collect();
        assert_eq!(cuts, vec![30, 60], "{b:?}");
    }

    #[test]
    fn a_two_frame_flash_is_not_a_cut() {
        let mut f = scene(60, 0, [10, 10, 10]);
        let white = vec![255u8; W * H * 3];
        f[30] = white.clone();
        f[31] = white;
        let b = detect(&f, &SceneParams::default());
        assert!(b.is_empty(), "flash virou fronteira: {b:?}");
    }

    #[test]
    fn camera_pan_without_cut_is_ignored() {
        // 90 quadros deslocando 3px por quadro (translação preserva o histograma)
        let f: Vec<Vec<u8>> = (0..90).map(|i| tex(0, i * 3, [10, 10, 10])).collect();
        assert!(detect(&f, &SceneParams::default()).is_empty());
    }

    #[test]
    fn static_scene_is_quiet() {
        let f: Vec<Vec<u8>> = (0..60).map(|_| tex(3, 0, [40, 40, 40])).collect();
        assert!(detect(&f, &SceneParams::default()).is_empty());
    }

    #[test]
    fn a_dissolve_is_found_once_near_its_middle() {
        let a = scene(60, 0, [10, 10, 10]);
        let b = scene(60, 1, [150, 80, 40]);
        let mut f: Vec<Vec<u8>> = a[..30].to_vec();
        for k in 0..12 {
            let t = (k + 1) as f32 / 13.0;
            f.push(blend(&a[30 + k], &b[k], t));
        }
        f.extend_from_slice(&b[12..]);
        let bd = detect(&f, &SceneParams::default());
        assert_eq!(bd.len(), 1, "{bd:?}");
        assert_eq!(bd[0].kind, BoundaryKind::Dissolve);
        assert!((bd[0].frame as i64 - 36).abs() <= 3, "{bd:?}");
    }

    #[test]
    fn a_fade_through_black_is_classified_as_fade() {
        let a = scene(60, 0, [120, 90, 60]);
        let b = scene(60, 2, [60, 140, 160]);
        let black = vec![0u8; W * H * 3];
        let mut f: Vec<Vec<u8>> = a[..30].to_vec();
        for k in 0..16 {
            f.push(blend(&a[30 + k], &black, (k + 1) as f32 / 16.0));
        }
        for (k, frame) in b.iter().take(16).enumerate() {
            f.push(blend(&black, frame, (k + 1) as f32 / 16.0));
        }
        f.extend_from_slice(&b[16..]);
        let bd = detect(&f, &SceneParams::default());
        assert!(bd.iter().any(|x| x.kind == BoundaryKind::Fade), "{bd:?}");
        assert!(
            bd.iter().all(|x| x.kind != BoundaryKind::Cut),
            "fade não pode virar corte: {bd:?}"
        );
    }

    #[test]
    fn deterministic_and_tiny_inputs_are_safe() {
        let f = scene(40, 0, [10, 10, 10]);
        assert_eq!(
            detect(&f, &SceneParams::default()),
            detect(&f, &SceneParams::default())
        );
        assert!(detect(&[], &SceneParams::default()).is_empty());
        assert!(detect(&f[..2], &SceneParams::default()).is_empty());
    }

    #[test]
    fn frame_time_is_exact_integer_ticks() {
        assert_eq!(frame_ticks(30, 30, 1), capia_time::TICKS_PER_SECOND);
        assert_eq!(frame_ticks(0, 30000, 1001), 0);
        assert_eq!(
            frame_ticks(30000, 30000, 1001),
            capia_time::TICKS_PER_SECOND * 1001
        );
    }
}
