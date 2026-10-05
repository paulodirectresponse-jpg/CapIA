//! Transcript canônico (independe de provider). Tempos em **microssegundos inteiros** — a conversão
//! para Ticks (705.600.000/s) é determinística e feita por quem conhece `capia-time`
//! (`us * 7056 / 10`, arredondando para o mais próximo).

use serde::{Deserialize, Serialize};

pub const TRANSCRIPT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub start_us: i64,
    pub end_us: i64,
    pub text: String,
    #[serde(default)]
    pub confidence: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_us: i64,
    pub end_us: i64,
    pub text: String,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub words: Vec<Word>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Transcript {
    #[serde(default)]
    pub schema_version: u32,
    pub language: Option<String>,
    #[serde(default)]
    pub duration_us: Option<i64>,
    pub segments: Vec<Segment>,
}

impl Transcript {
    pub fn text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn has_word_timestamps(&self) -> bool {
        self.segments.iter().any(|s| !s.words.is_empty())
    }

    /// Valida invariantes de tempo (monotonia, fim ≥ início).
    pub fn validate(&self) -> Result<(), String> {
        let mut last_end = i64::MIN;
        for (i, s) in self.segments.iter().enumerate() {
            if s.start_us < 0 || s.end_us < s.start_us {
                return Err(format!("segment {i}: invalid time range"));
            }
            if s.start_us < last_end.saturating_sub(50_000) {
                return Err(format!("segment {i}: not in chronological order"));
            }
            last_end = last_end.max(s.end_us);
            for (j, w) in s.words.iter().enumerate() {
                if w.start_us < 0 || w.end_us < w.start_us {
                    return Err(format!("segment {i} word {j}: invalid time range"));
                }
            }
        }
        Ok(())
    }
}

/// Segundos (float do provider) → microssegundos inteiros, arredondando; entrada inválida ⇒ 0.
pub fn secs_to_us(s: f64) -> i64 {
    if !s.is_finite() || s < 0.0 {
        return 0;
    }
    let us = (s * 1_000_000.0).round();
    if us > 1.0e15 {
        1_000_000_000_000_000
    } else {
        us as i64
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SttRequest {
    pub model: String,
    pub audio: Vec<u8>,
    pub mime: String,
    pub filename: String,
    pub language: Option<String>,
    pub word_timestamps: bool,
    /// Dica de vocabulário (nunca instruções).
    pub prompt: Option<String>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn seconds_to_us_is_deterministic_and_safe() {
        assert_eq!(secs_to_us(1.5), 1_500_000);
        assert_eq!(secs_to_us(0.0000004), 0);
        assert_eq!(secs_to_us(f64::NAN), 0);
        assert_eq!(secs_to_us(-3.0), 0);
        assert_eq!(secs_to_us(1.0e300), 1_000_000_000_000_000);
    }

    #[test]
    fn validate_catches_bad_order() {
        let mut t = Transcript {
            schema_version: 1,
            language: None,
            duration_us: None,
            segments: vec![
                Segment {
                    start_us: 0,
                    end_us: 1_000_000,
                    text: "a".into(),
                    confidence: None,
                    speaker: None,
                    words: vec![],
                },
                Segment {
                    start_us: 2_000_000,
                    end_us: 3_000_000,
                    text: "b".into(),
                    confidence: None,
                    speaker: None,
                    words: vec![],
                },
            ],
        };
        assert!(t.validate().is_ok());
        t.segments[1].start_us = 100;
        assert!(t.validate().is_err());
        assert_eq!(t.text(), "a b");
    }
}
