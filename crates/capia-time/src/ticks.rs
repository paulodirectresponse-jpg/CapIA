use core::fmt;
use serde::{Deserialize, Serialize};

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

/// Erros de aritmética/conversão de tempo. Nunca há pânico por overflow: tudo é `checked`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeError {
    /// Resultado fora do intervalo de `i64`.
    Overflow,
    /// Denominador zero (ou taxa/razão inválida).
    DivideByZero,
    /// Razão/taxa inválida (numerador ≤ 0 onde só positivos são aceitos).
    InvalidRatio,
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Overflow => "time arithmetic overflow",
            Self::DivideByZero => "division by zero",
            Self::InvalidRatio => "invalid ratio",
        })
    }
}

impl core::error::Error for TimeError {}

/// Instante ou duração em ticks. Nunca use ponto flutuante no modelo.
///
/// Serializa como inteiro JSON (seguro em JS enquanto ≤ 24 h, ver [`MAX_TIMELINE_TICKS`]).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Ticks(pub i64);

impl Ticks {
    pub const ZERO: Self = Self(0);

    /// Ticks correspondentes a `seconds` segundos inteiros.
    pub fn from_seconds(seconds: i64) -> Result<Self, TimeError> {
        seconds
            .checked_mul(TICKS_PER_SECOND)
            .map(Self)
            .ok_or(TimeError::Overflow)
    }

    pub fn checked_add(self, other: Self) -> Result<Self, TimeError> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(TimeError::Overflow)
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, TimeError> {
        self.0
            .checked_sub(other.0)
            .map(Self)
            .ok_or(TimeError::Overflow)
    }

    /// `self * k`, sem overflow silencioso.
    pub fn checked_mul(self, k: i64) -> Result<Self, TimeError> {
        self.0.checked_mul(k).map(Self).ok_or(TimeError::Overflow)
    }

    /// `self * num / den` com **half-up** (arredonda .5 em direção a +∞), via `i128`.
    pub fn mul_div_round(self, num: i64, den: i64) -> Result<Self, TimeError> {
        if den == 0 {
            return Err(TimeError::DivideByZero);
        }
        let (mut n, mut d) = (i128::from(self.0) * i128::from(num), i128::from(den));
        if d < 0 {
            n = -n;
            d = -d;
        }
        // floor((2n + d) / (2d)) == round-half-up(n / d)
        let q = (2 * n + d).div_euclid(2 * d);
        i64::try_from(q).map(Self).map_err(|_| TimeError::Overflow)
    }

    /// `self * num / den` com `floor` (em direção a −∞).
    pub fn mul_div_floor(self, num: i64, den: i64) -> Result<Self, TimeError> {
        if den == 0 {
            return Err(TimeError::DivideByZero);
        }
        let (mut n, mut d) = (i128::from(self.0) * i128::from(num), i128::from(den));
        if d < 0 {
            n = -n;
            d = -d;
        }
        i64::try_from(n.div_euclid(d))
            .map(Self)
            .map_err(|_| TimeError::Overflow)
    }

    /// Esta posição/duração cabe no intervalo permitido da timeline `[0, 24 h]`.
    pub fn within_timeline(self) -> bool {
        (0..=MAX_TIMELINE_TICKS).contains(&self.0)
    }
}

impl fmt::Display for Ticks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}t", self.0)
    }
}

/// Intervalo semiaberto `[start, start + duration)` em ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Ticks,
    pub duration: Ticks,
}

impl TimeRange {
    pub fn new(start: Ticks, duration: Ticks) -> Self {
        Self { start, duration }
    }

    /// Fim exclusivo. `None` se estourar `i64`.
    pub fn end(&self) -> Option<Ticks> {
        self.start.checked_add(self.duration).ok()
    }

    /// Intervalos semiabertos: encostar (`a.end == b.start`) **não** é sobrepor.
    pub fn overlaps(&self, other: &Self) -> bool {
        match (self.end(), other.end()) {
            (Some(a_end), Some(b_end)) => self.start < b_end && other.start < a_end,
            _ => true,
        }
    }

    /// `t ∈ [start, end)`.
    pub fn contains(&self, t: Ticks) -> bool {
        self.end().is_some_and(|end| self.start <= t && t < end)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
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

    #[test]
    fn round_half_up_goes_toward_positive_infinity_on_ties() {
        assert_eq!(Ticks(5).mul_div_round(1, 2).unwrap(), Ticks(3)); // 2.5 -> 3
        assert_eq!(Ticks(-5).mul_div_round(1, 2).unwrap(), Ticks(-2)); // -2.5 -> -2
        assert_eq!(Ticks(10).mul_div_round(1, 3).unwrap(), Ticks(3)); // 3.33 -> 3
        assert_eq!(Ticks(20).mul_div_round(1, 3).unwrap(), Ticks(7)); // 6.67 -> 7
        assert_eq!(Ticks(7).mul_div_round(-1, -2).unwrap(), Ticks(4)); // sign-normalised
    }

    #[test]
    fn floor_division_handles_negatives() {
        assert_eq!(Ticks(7).mul_div_floor(1, 2).unwrap(), Ticks(3));
        assert_eq!(Ticks(-7).mul_div_floor(1, 2).unwrap(), Ticks(-4));
    }

    #[test]
    fn arithmetic_never_wraps() {
        assert_eq!(
            Ticks(i64::MAX).checked_add(Ticks(1)),
            Err(TimeError::Overflow)
        );
        assert_eq!(
            Ticks(i64::MIN).checked_sub(Ticks(1)),
            Err(TimeError::Overflow)
        );
        assert_eq!(Ticks(i64::MAX).checked_mul(2), Err(TimeError::Overflow));
        assert_eq!(Ticks(1).mul_div_round(1, 0), Err(TimeError::DivideByZero));
        assert_eq!(
            Ticks(i64::MAX).mul_div_round(2, 1),
            Err(TimeError::Overflow)
        );
        // i128 intermediate: no spurious overflow when the result fits.
        assert_eq!(
            Ticks(i64::MAX).mul_div_round(2, 2).unwrap(),
            Ticks(i64::MAX)
        );
    }

    #[test]
    fn ranges_are_half_open() {
        let a = TimeRange::new(Ticks(0), Ticks(30));
        let b = TimeRange::new(Ticks(30), Ticks(30));
        let c = TimeRange::new(Ticks(29), Ticks(10));
        assert!(!a.overlaps(&b), "touching ranges do not overlap");
        assert!(a.overlaps(&c));
        assert!(a.contains(Ticks(0)));
        assert!(!a.contains(Ticks(30)));
    }

    #[test]
    fn ticks_serialize_as_plain_integers() {
        assert_eq!(
            serde_json::to_string(&Ticks(706_305_600)).unwrap(),
            "706305600"
        );
        assert_eq!(serde_json::from_str::<Ticks>("42").unwrap(), Ticks(42));
    }

    #[test]
    fn timeline_bounds() {
        assert!(Ticks(0).within_timeline());
        assert!(Ticks(MAX_TIMELINE_TICKS).within_timeline());
        assert!(!Ticks(MAX_TIMELINE_TICKS + 1).within_timeline());
        assert!(!Ticks(-1).within_timeline());
    }
}
