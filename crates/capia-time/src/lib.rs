//! Base de tempo do CapIA (ADR-007). **Scaffold:** só a unidade canônica e o tipo `Ticks`.
//!
//! Regras (docs/TIMELINE_ENGINE.md §1): tempo é inteiro; `Rational`, `FrameRate`, conversões
//! frame/amostra e `TimeRange` chegam na Fase 2. Este crate não faz IO e compila para WASM.

/// Unidade canônica: 1 tick = 1/705.600.000 s. Divisível exatamente pela duração de frame de
/// todas as taxas usuais de vídeo e pelas taxas de amostragem de áudio (ver testes).
pub const TICKS_PER_SECOND: i64 = 705_600_000;

/// Teto de qualquer posição/duração de timeline: 24 h. Mantém todo valor como inteiro seguro em
/// JavaScript (`Number.MAX_SAFE_INTEGER`) — ver docs/spikes/S4-wasm-core.md.
pub const MAX_TIMELINE_TICKS: i64 = 24 * 3600 * TICKS_PER_SECOND;

/// Maior inteiro exatamente representável em JavaScript (`Number.MAX_SAFE_INTEGER`).
const JS_MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;
// Invariante verificada em tempo de compilação: o teto de 24 h cabe num `Number` do JS.
const _: () = assert!(MAX_TIMELINE_TICKS < JS_MAX_SAFE_INTEGER);

/// Instante ou duração em ticks. Nunca use ponto flutuante no modelo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ticks(pub i64);

#[cfg(test)]
mod tests {
    use super::*;

    /// (numerador, denominador) de taxas de quadros: ticks por frame = TPS * den / num.
    const FRAME_RATES: [(i64, i64); 10] = [
        (24_000, 1001),
        (24, 1),
        (25, 1),
        (30_000, 1001),
        (30, 1),
        (48, 1),
        (50, 1),
        (60_000, 1001),
        (60, 1),
        (120, 1),
    ];

    #[test]
    fn every_standard_frame_rate_has_an_exact_tick_duration() {
        for (num, den) in FRAME_RATES {
            assert_eq!(
                (TICKS_PER_SECOND * den) % num,
                0,
                "{num}/{den} fps is not exact"
            );
        }
    }

    #[test]
    fn known_frame_durations_match_the_documented_table() {
        assert_eq!(TICKS_PER_SECOND * 1001 / 24_000, 29_429_400);
        assert_eq!(TICKS_PER_SECOND * 1001 / 30_000, 23_543_520);
        assert_eq!(TICKS_PER_SECOND * 1001 / 60_000, 11_771_760);
        assert_eq!(TICKS_PER_SECOND / 24, 29_400_000);
    }

    #[test]
    fn standard_audio_sample_rates_divide_the_timebase() {
        for rate in [44_100, 48_000, 96_000] {
            assert_eq!(TICKS_PER_SECOND % rate, 0, "{rate} Hz is not exact");
        }
    }

    #[test]
    fn timeline_cap_is_a_javascript_safe_integer() {
        assert_eq!(MAX_TIMELINE_TICKS, 60_963_840_000_000);
    }

    #[test]
    fn ticks_are_ordered_integers() {
        assert!(Ticks(1) < Ticks(2));
        assert_eq!(Ticks::default(), Ticks(0));
    }
}
