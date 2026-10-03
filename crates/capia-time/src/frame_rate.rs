use crate::rational::Rational;
use crate::ticks::{TICKS_PER_SECOND, Ticks, TimeError};
use serde::{Deserialize, Serialize};

/// Taxa de quadros de uma sequence, p. ex. `30000/1001`. Só são aceitas taxas cuja duração de frame
/// é um número **inteiro** de ticks (cobre todas as taxas usuais; docs/TIMELINE_ENGINE.md §1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Rational", into = "Rational")]
pub struct FrameRate {
    rate: Rational,
    frame_ticks: Ticks,
}

impl TryFrom<Rational> for FrameRate {
    type Error = TimeError;

    fn try_from(rate: Rational) -> Result<Self, TimeError> {
        Self::new(rate)
    }
}

impl From<FrameRate> for Rational {
    fn from(value: FrameRate) -> Self {
        value.rate
    }
}

impl FrameRate {
    pub const FPS_24: Self = Self::const_int(24);
    pub const FPS_25: Self = Self::const_int(25);
    pub const FPS_30: Self = Self::const_int(30);
    pub const FPS_60: Self = Self::const_int(60);

    const fn const_int(n: i64) -> Self {
        Self {
            rate: Rational::from_int(n),
            frame_ticks: Ticks(TICKS_PER_SECOND / n),
        }
    }

    pub fn new(rate: Rational) -> Result<Self, TimeError> {
        if !rate.is_positive() {
            return Err(TimeError::InvalidRatio);
        }
        let total = i128::from(TICKS_PER_SECOND) * i128::from(rate.den());
        let num = i128::from(rate.num());
        if total % num != 0 {
            return Err(TimeError::InvalidRatio);
        }
        let frame_ticks = i64::try_from(total / num).map_err(|_| TimeError::Overflow)?;
        Ok(Self {
            rate,
            frame_ticks: Ticks(frame_ticks),
        })
    }

    pub fn from_fraction(num: i64, den: i64) -> Result<Self, TimeError> {
        Self::new(Rational::new(num, den)?)
    }

    pub const fn rate(&self) -> Rational {
        self.rate
    }

    /// Duração exata de um frame, em ticks.
    pub const fn frame_duration(&self) -> Ticks {
        self.frame_ticks
    }

    /// `n` frames em ticks (exato).
    pub fn frames_to_ticks(&self, frames: i64) -> Result<Ticks, TimeError> {
        self.frame_ticks.checked_mul(frames)
    }

    /// `frames` (racional, p. ex. `21/2`) em ticks. Exato quando a fração resulta em tick inteiro;
    /// caso contrário arredonda half-up (o chamador decide se aceita ticks não alinhados).
    pub fn rational_frames_to_ticks(&self, frames: Rational) -> Result<Ticks, TimeError> {
        self.frame_ticks.mul_div_round(frames.num(), frames.den())
    }

    /// Índice do frame que **contém** `t` (floor).
    pub fn ticks_to_frames_floor(&self, t: Ticks) -> i64 {
        t.0.div_euclid(self.frame_ticks.0)
    }

    /// `t` é múltiplo exato da duração de frame.
    pub fn is_aligned(&self, t: Ticks) -> bool {
        t.0.rem_euclid(self.frame_ticks.0) == 0
    }

    /// Alinha a borda de vídeo ao frame mais próximo, **half-up** (D-S7-8).
    pub fn align_half_up(&self, t: Ticks) -> Result<Ticks, TimeError> {
        let f = self.frame_ticks.0;
        let frames = (2 * i128::from(t.0) + i128::from(f)).div_euclid(2 * i128::from(f));
        i64::try_from(frames * i128::from(f))
            .map(Ticks)
            .map_err(|_| TimeError::Overflow)
    }

    /// Alinha para baixo (frame que contém `t`).
    pub fn align_floor(&self, t: Ticks) -> Ticks {
        Ticks(t.0.div_euclid(self.frame_ticks.0) * self.frame_ticks.0)
    }

    /// Duração de vídeo alinhada com **half-up** e mínimo de **1 frame** (D-S7-8).
    pub fn align_duration_half_up(&self, d: Ticks) -> Result<Ticks, TimeError> {
        Ok(self.align_half_up(d)?.max(self.frame_ticks))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn standard_rates_have_exact_frame_durations() {
        assert_eq!(FrameRate::FPS_30.frame_duration(), Ticks(23_520_000));
        assert_eq!(FrameRate::FPS_24.frame_duration(), Ticks(29_400_000));
        assert_eq!(
            FrameRate::from_fraction(30_000, 1001)
                .unwrap()
                .frame_duration(),
            Ticks(23_543_520)
        );
        assert_eq!(
            FrameRate::from_fraction(24_000, 1001)
                .unwrap()
                .frame_duration(),
            Ticks(29_429_400)
        );
        assert_eq!(
            FrameRate::from_fraction(60_000, 1001)
                .unwrap()
                .frame_duration(),
            Ticks(11_771_760)
        );
    }

    #[test]
    fn frame_30_at_ntsc_matches_the_documented_tick() {
        let fr = FrameRate::from_fraction(30_000, 1001).unwrap();
        assert_eq!(fr.frames_to_ticks(30).unwrap(), Ticks(706_305_600));
        assert_eq!(fr.frames_to_ticks(60).unwrap(), Ticks(1_412_611_200));
    }

    #[test]
    fn rates_that_are_not_tick_exact_are_rejected() {
        // 705.600.000 = 2^9 * 3^2 * 5^5 * 7^2: 11 e 13 não dividem o timebase.
        assert_eq!(
            FrameRate::from_fraction(11, 1),
            Err(TimeError::InvalidRatio)
        );
        assert_eq!(
            FrameRate::from_fraction(13, 1),
            Err(TimeError::InvalidRatio)
        );
        assert_eq!(FrameRate::from_fraction(0, 1), Err(TimeError::InvalidRatio));
        assert_eq!(
            FrameRate::from_fraction(-30, 1),
            Err(TimeError::InvalidRatio)
        );
        assert!(FrameRate::from_fraction(7, 1).is_ok());
    }

    #[test]
    fn alignment_rounds_half_up_with_one_frame_minimum() {
        let fr = FrameRate::FPS_30;
        let f = fr.frame_duration().0;
        assert_eq!(fr.align_half_up(Ticks(f / 2)).unwrap(), Ticks(f)); // 0.5 frame -> 1
        assert_eq!(fr.align_half_up(Ticks(f / 2 - 1)).unwrap(), Ticks(0));
        assert_eq!(
            fr.align_half_up(Ticks(3 * f + f / 2)).unwrap(),
            Ticks(4 * f)
        );
        assert_eq!(
            fr.align_duration_half_up(Ticks(1)).unwrap(),
            Ticks(f),
            "min 1 frame"
        );
        assert_eq!(fr.align_duration_half_up(Ticks(0)).unwrap(), Ticks(f));
        assert!(fr.is_aligned(Ticks(5 * f)));
        assert!(!fr.is_aligned(Ticks(5 * f + 1)));
        assert!(!fr.is_aligned(Ticks(-1)));
        assert_eq!(fr.ticks_to_frames_floor(Ticks(5 * f + 7)), 5);
        assert_eq!(fr.ticks_to_frames_floor(Ticks(-1)), -1);
        assert_eq!(fr.align_floor(Ticks(5 * f + 7)), Ticks(5 * f));
    }

    #[test]
    fn rational_frames_convert_to_ticks() {
        let fr = FrameRate::FPS_30;
        // 21/2 frames = 10.5 frames = 247.0 M ticks (exato porque o frame tem tick par)
        assert_eq!(
            fr.rational_frames_to_ticks(Rational::new(21, 2).unwrap())
                .unwrap(),
            Ticks(246_960_000)
        );
        assert!(!fr.is_aligned(Ticks(246_960_000)));
    }

    #[test]
    fn serde_round_trip_and_validation() {
        let fr = FrameRate::from_fraction(30_000, 1001).unwrap();
        let json = serde_json::to_string(&fr).unwrap();
        assert_eq!(json, "\"30000/1001\"");
        assert_eq!(serde_json::from_str::<FrameRate>(&json).unwrap(), fr);
        assert!(serde_json::from_str::<FrameRate>("\"0\"").is_err());
        assert!(serde_json::from_str::<FrameRate>("30").is_ok());
    }
}
