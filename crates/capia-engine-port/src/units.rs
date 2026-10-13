//! Timebase na fronteira. A inteligência (legado) fala em **705 600 000 ticks/s**; o editor
//! OpenCut usa **120 000 ticks/s**. Razão exata: `705 600 000 / 120 000 = 5 880`.
//!
//! Exato em 120 000 t/s: 24, 25, 30, 48, 50, 60 fps e as razões NTSC 24000/1001, 30000/1001,
//! 60000/1001 (período em ticks inteiro). **Não** exato: amostras de áudio 44 100 Hz (2,72 t/amostra)
//! e 48 000 Hz (2,5 t/amostra) — vide ADR de timebase.

use crate::PortError;

pub const LEGACY_TICKS_PER_SECOND: i64 = 705_600_000;
pub const EDITOR_TICKS_PER_SECOND: i64 = 120_000;
pub const RATIO: i64 = LEGACY_TICKS_PER_SECOND / EDITOR_TICKS_PER_SECOND;

/// Legado → editor. `Err` se não for divisível (perda de precisão seria silenciosa).
pub fn to_editor_ticks(legacy: i64) -> Result<i64, PortError> {
    if legacy % RATIO == 0 {
        Ok(legacy / RATIO)
    } else {
        Err(PortError::new(
            "TIMEBASE_INEXACT",
            format!(
                "{legacy} legacy ticks is not a multiple of {RATIO} (sub-tick precision would be lost)"
            ),
        ))
    }
}

/// Editor → legado (sempre exato).
pub fn to_legacy_ticks(editor: i64) -> i64 {
    editor.saturating_mul(RATIO)
}

/// Segundos (float vindos do DTO do editor, ex. duração de asset) → ticks do editor, arredondando
/// ao tick inteiro mais próximo.
pub fn seconds_to_editor_ticks(seconds: f64) -> i64 {
    (seconds * EDITOR_TICKS_PER_SECOND as f64).round() as i64
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn ratio_is_exact() {
        assert_eq!(RATIO, 5_880);
        assert_eq!(RATIO * EDITOR_TICKS_PER_SECOND, LEGACY_TICKS_PER_SECOND);
    }

    #[test]
    fn common_frame_periods_are_exact_in_editor_ticks() {
        for (num, den) in [
            (24, 1),
            (25, 1),
            (30, 1),
            (48, 1),
            (50, 1),
            (60, 1),
            (24_000, 1001),
            (30_000, 1001),
            (60_000, 1001),
        ] {
            let period = EDITOR_TICKS_PER_SECOND * den;
            assert_eq!(
                period % num,
                0,
                "{num}/{den} fps period must be an integer tick count"
            );
        }
    }

    #[test]
    fn audio_sample_periods_are_not_exact() {
        assert_ne!(EDITOR_TICKS_PER_SECOND % 48_000, 0);
        assert_ne!(EDITOR_TICKS_PER_SECOND % 44_100, 0);
    }

    #[test]
    fn conversion_roundtrip_and_inexact() {
        assert_eq!(to_editor_ticks(to_legacy_ticks(1234)).unwrap(), 1234);
        assert_eq!(to_editor_ticks(705_600_000).unwrap(), 120_000);
        assert_eq!(to_editor_ticks(1).unwrap_err().code, "TIMEBASE_INEXACT");
    }
}
