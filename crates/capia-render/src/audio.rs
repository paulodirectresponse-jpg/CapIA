//! Buffers de áudio f32 e reamostragem determinística (ADR-066).
//!
//! Só `+ − × ÷` e comparações IEEE em `f64`, mais `sin`/`cos` na construção do *sinc* e da janela
//! (diferença entre libms < 1e-15 — muito abaixo da tolerância dos goldens de áudio, 1e-4).

/// PCM f32 intercalado.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: u32,
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    pub fn silence(sample_rate: u32, channels: u32, frames: u64) -> Self {
        let n = (frames as usize).saturating_mul(channels as usize);
        Self {
            sample_rate,
            channels,
            samples: vec![0.0; n],
        }
    }

    pub fn frames(&self) -> u64 {
        if self.channels == 0 {
            0
        } else {
            (self.samples.len() / self.channels as usize) as u64
        }
    }
}

/// Metade da largura do *sinc* (zeros de cada lado).
const SINC_HALF: i64 = 16;

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let px = core::f64::consts::PI * x;
        px.sin() / px
    }
}

/// *Sinc* com janela de Hann. `cutoff ≤ 1` reduz a banda ao reamostrar para baixo.
fn kernel(dist: f64, cutoff: f64) -> f64 {
    let half = SINC_HALF as f64;
    let d = dist.abs();
    if d >= half {
        return 0.0;
    }
    let window = 0.5 * (1.0 + (core::f64::consts::PI * d / half).cos());
    cutoff * sinc(cutoff * dist) * window
}

/// Reamostra de `in_rate` para `out_rate`: devolve as amostras de saída `out_start..out_start+out_frames`
/// (índices na taxa de saída, contados do início da mídia). `input` contém amostras de fonte
/// intercaladas a partir do índice absoluto `in_origin`; fora dele vale zero (o chamador fornece
/// margem de `SINC_HALF` amostras dos dois lados). Taxas iguais ⇒ cópia exata.
pub fn resample_sinc(
    input: &[f32],
    channels: u32,
    in_rate: u32,
    out_rate: u32,
    in_origin: u64,
    out_start: u64,
    out_frames: u64,
) -> Vec<f32> {
    let ch = channels as usize;
    let n_in = (input.len() / ch.max(1)) as i64;
    let mut out = vec![0.0f32; (out_frames as usize) * ch];
    if ch == 0 {
        return out;
    }
    if in_rate == out_rate {
        for j in 0..out_frames as i64 {
            let idx = (out_start as i64 + j) - in_origin as i64;
            if (0..n_in).contains(&idx) {
                let (a, b) = (idx as usize * ch, j as usize * ch);
                out[b..b + ch].copy_from_slice(&input[a..a + ch]);
            }
        }
        return out;
    }
    let cutoff = if out_rate < in_rate {
        f64::from(out_rate) / f64::from(in_rate)
    } else {
        1.0
    };
    for j in 0..out_frames {
        // posição de fonte (racional): p = (out_start + j) · in_rate / out_rate
        let num = (u128::from(out_start) + u128::from(j)) * u128::from(in_rate);
        let den = u128::from(out_rate);
        let base = (num / den) as i64; // floor(p)
        let frac = (num % den) as f64 / den as f64;
        for c in 0..ch {
            let mut acc = 0.0f64;
            for k in (base - SINC_HALF + 1)..=(base + SINC_HALF) {
                let idx = k - in_origin as i64;
                if idx < 0 || idx >= n_in {
                    continue;
                }
                let dist = (k as f64) - (base as f64 + frac);
                acc += f64::from(input[idx as usize * ch + c]) * kernel(dist, cutoff);
            }
            out[j as usize * ch + c] = acc as f32;
        }
    }
    out
}

/// Varispeed: `out[j] = interp(src, base + dir·j·speed)` com interpolação **linear**; `dir = −1`
/// se `reversed`. `base` e as posições são índices absolutos de amostra (mesma taxa de `src`);
/// `src` começa no índice `src_origin`. Fora de `src` vale zero. `speed = 1` e sem reverso ⇒ cópia
/// exata (fração 0).
#[allow(clippy::too_many_arguments)]
pub fn resample_linear_position(
    src: &[f32],
    channels: u32,
    src_origin: u64,
    base: i64,
    speed_num: i64,
    speed_den: i64,
    reversed: bool,
    out_frames: u64,
) -> Vec<f32> {
    let ch = channels as usize;
    let n_src = (src.len() / ch.max(1)) as i64;
    let mut out = vec![0.0f32; (out_frames as usize) * ch];
    if ch == 0 || speed_den <= 0 || speed_num <= 0 {
        return out;
    }
    let (num, den) = (i128::from(speed_num), i128::from(speed_den));
    for j in 0..out_frames {
        let steps = i128::from(j) * num; // j · speed = steps / den
        let (whole, frac_num) = if reversed {
            // posição = base − steps/den = base − ceil(steps/den) + (1 − frac) quando há resto
            let w = steps / den;
            let r = steps % den;
            if r == 0 { (-w, 0) } else { (-w - 1, den - r) }
        } else {
            (steps / den, steps % den)
        };
        let p = i128::from(base) + whole;
        let frac = frac_num as f64 / den as f64;
        let i0 = p as i64 - src_origin as i64;
        for c in 0..ch {
            let at = |i: i64| -> f64 {
                if (0..n_src).contains(&i) {
                    f64::from(src[i as usize * ch + c])
                } else {
                    0.0
                }
            };
            let v = if frac == 0.0 {
                at(i0)
            } else {
                at(i0) * (1.0 - frac) + at(i0 + 1) * frac
            };
            out[j as usize * ch + c] = v as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn sine(rate: u32, freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * core::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin() as f32)
            .collect()
    }

    #[test]
    fn equal_rates_copy_exactly_and_zero_fill_outside() {
        let src: Vec<f32> = (0..10).map(|i| i as f32).collect();
        let out = resample_sinc(&src, 1, 48_000, 48_000, 100, 98, 14);
        assert_eq!(&out[..2], &[0.0, 0.0]);
        assert_eq!(&out[2..12], &src[..]);
        assert_eq!(&out[12..], &[0.0, 0.0]);
    }

    #[test]
    fn sinc_preserves_a_tone_when_resampling_44100_to_48000() {
        let src = sine(44_100, 440.0, 20_000);
        let out = resample_sinc(&src, 1, 44_100, 48_000, 0, 100, 4_000);
        let want = sine(48_000, 440.0, 4_100);
        let mut worst = 0.0f32;
        for (j, v) in out.iter().enumerate() {
            worst = worst.max((v - want[100 + j]).abs());
        }
        assert!(worst < 2e-3, "worst error {worst}");
        // determinismo: duas execuções idênticas
        assert_eq!(out, resample_sinc(&src, 1, 44_100, 48_000, 0, 100, 4_000));
    }

    #[test]
    fn dc_is_preserved_and_downsampling_does_not_alias_wildly() {
        let src = vec![0.5f32; 4_000];
        let out = resample_sinc(&src, 1, 48_000, 44_100, 0, 100, 1_000);
        assert!(out.iter().all(|v| (v - 0.5).abs() < 2e-3));
    }

    #[test]
    fn varispeed_matches_the_definition() {
        let src: Vec<f32> = (0..20).map(|i| i as f32).collect();
        // 1×: cópia exata
        assert_eq!(
            resample_linear_position(&src, 1, 0, 3, 1, 1, false, 5),
            vec![3.0, 4.0, 5.0, 6.0, 7.0]
        );
        // 2×: pula uma
        assert_eq!(
            resample_linear_position(&src, 1, 0, 2, 2, 1, false, 4),
            vec![2.0, 4.0, 6.0, 8.0]
        );
        // 0,5×: interpola no meio
        assert_eq!(
            resample_linear_position(&src, 1, 0, 2, 1, 2, false, 4),
            vec![2.0, 2.5, 3.0, 3.5]
        );
        // reverso 1×: anda para trás
        assert_eq!(
            resample_linear_position(&src, 1, 0, 10, 1, 1, true, 4),
            vec![10.0, 9.0, 8.0, 7.0]
        );
        // reverso 0,5×
        assert_eq!(
            resample_linear_position(&src, 1, 0, 10, 1, 2, true, 3),
            vec![10.0, 9.5, 9.0]
        );
        // fora da fonte = silêncio; multicanal intercalado
        let st = vec![1.0, -1.0, 2.0, -2.0];
        assert_eq!(
            resample_linear_position(&st, 2, 0, -1, 1, 1, false, 3),
            vec![0.0, 0.0, 1.0, -1.0, 2.0, -2.0]
        );
        // entradas inválidas não entram em pânico
        assert_eq!(
            resample_linear_position(&src, 1, 0, 0, 0, 1, false, 2),
            vec![0.0, 0.0]
        );
    }

    #[test]
    fn silence_has_the_right_shape() {
        let s = AudioBuffer::silence(48_000, 2, 10);
        assert_eq!((s.frames(), s.samples.len()), (10, 20));
    }
}
