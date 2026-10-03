use crate::ids::{AssetId, ClipId, SequenceId, TrackId};
use crate::property::PropertySet;
use capia_time::{Rational, Ticks, TimeError, TimeRange};
use serde::{Deserialize, Serialize};

/// Velocidade de um clip: de 0,01× a 100× (D-S7-1). Fora disso: `OUT_OF_RANGE`.
pub fn speed_in_range(speed: Rational) -> bool {
    speed >= Rational::new(1, 100).unwrap_or(Rational::ONE) && speed <= Rational::from_int(100)
}

/// O que o clip contém (docs/TIMELINE_ENGINE.md §3). Conteúdos ainda não implementados no motor
/// (`Caption`) entram em missões futuras.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClipContent {
    /// Mídia com componente de vídeo e/ou áudio. Em track `Audio` só pode ter áudio.
    Media {
        asset: AssetId,
        has_video: bool,
        has_audio: bool,
    },
    /// Imagem (duração livre).
    Image {
        asset: AssetId,
    },
    Text {
        text: String,
    },
    Solid {
        color: String,
    },
    /// Outra sequence do projeto (referência compartilhada; DAG com profundidade ≤ 16).
    Nested {
        sequence: SequenceId,
    },
}

impl ClipContent {
    /// O tempo de conteúdo vem de uma fonte externa (mídia ou sequence filha). Para texto, imagem e
    /// sólido o tempo é "local desde a criação" e `split` re-basa os keyframes da metade direita.
    pub fn has_source_time(&self) -> bool {
        matches!(self, Self::Media { .. } | Self::Nested { .. })
    }

    /// Aceita retime (`speed`) e reverso.
    pub fn supports_retime(&self) -> bool {
        self.has_source_time()
    }

    pub fn asset(&self) -> Option<&AssetId> {
        match self {
            Self::Media { asset, .. } | Self::Image { asset } => Some(asset),
            _ => None,
        }
    }
}

/// Clip numa track. `start`/`duration` estão no tempo da sequence; o mapeamento para o conteúdo é
/// `content_t = source_in + (t − start) × speed` (ou invertido se `reversed`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub track: TrackId,
    pub start: Ticks,
    pub duration: Ticks,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub content: ClipContent,
    /// Offset no conteúdo (tempo de origem, em ticks — **subframe permitido**, D-S7-8).
    #[serde(default)]
    pub source_in: Ticks,
    #[serde(default = "rational_one")]
    pub speed: Rational,
    #[serde(default)]
    pub reversed: bool,
    #[serde(default)]
    pub properties: PropertySet,
}

fn default_true() -> bool {
    true
}

fn rational_one() -> Rational {
    Rational::ONE
}

impl Clip {
    /// Fim exclusivo na timeline.
    pub fn end(&self) -> Ticks {
        Ticks(self.start.0.saturating_add(self.duration.0))
    }

    pub fn range(&self) -> TimeRange {
        TimeRange::new(self.start, self.duration)
    }

    /// Tempo de conteúdo correspondente ao instante `t` da timeline (arredondamento half-up só
    /// quando `speed` não divide exatamente o tick).
    pub fn content_time(&self, t: Ticks) -> Result<Ticks, TimeError> {
        let local = t.checked_sub(self.start)?;
        let offset = if self.reversed {
            self.duration.checked_sub(local)?
        } else {
            local
        };
        self.source_in
            .checked_add(offset.mul_div_round(self.speed.num(), self.speed.den())?)
    }

    /// O trecho de fonte consumido `[source_in, source_in + duração × speed]` cabe em
    /// `source_len`? Comparação exata (sem arredondar).
    pub fn fits_source(&self, source_len: Ticks) -> bool {
        let den = i128::from(self.speed.den());
        let need = i128::from(self.source_in.0) * den
            + i128::from(self.duration.0) * i128::from(self.speed.num());
        self.source_in.0 >= 0 && need <= i128::from(source_len.0) * den
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn clip(start: i64, dur: i64, src_in: i64, speed: Rational, reversed: bool) -> Clip {
        Clip {
            id: "c".into(),
            track: "t".into(),
            start: Ticks(start),
            duration: Ticks(dur),
            name: String::new(),
            enabled: true,
            content: ClipContent::Media {
                asset: "a".into(),
                has_video: true,
                has_audio: false,
            },
            source_in: Ticks(src_in),
            speed,
            reversed,
            properties: PropertySet::new(),
        }
    }

    #[test]
    fn content_time_follows_start_source_in_and_speed() {
        let c = clip(100, 60, 1000, Rational::from_int(2), false);
        assert_eq!(c.content_time(Ticks(100)).unwrap(), Ticks(1000));
        assert_eq!(c.content_time(Ticks(130)).unwrap(), Ticks(1060));
        let slow = clip(0, 60, 0, Rational::new(1, 2).unwrap(), false);
        assert_eq!(slow.content_time(Ticks(5)).unwrap(), Ticks(3)); // 2.5 -> 3 (half-up)
    }

    #[test]
    fn reversed_clips_play_the_segment_backwards() {
        let c = clip(10, 90, 0, Rational::ONE, true);
        assert_eq!(c.content_time(Ticks(10)).unwrap(), Ticks(90));
        assert_eq!(c.content_time(Ticks(100)).unwrap(), Ticks(0));
    }

    #[test]
    fn fits_source_is_exact() {
        let c = clip(0, 50, 0, Rational::new(3, 2).unwrap(), false); // consome 75
        assert!(c.fits_source(Ticks(75)));
        assert!(!c.fits_source(Ticks(74)));
        let c = clip(0, 1, 0, Rational::new(1, 3).unwrap(), false);
        assert!(c.fits_source(Ticks(1)));
        assert!(!clip(0, 10, -1, Rational::ONE, false).fits_source(Ticks(100)));
    }

    #[test]
    fn json_defaults_are_sane() {
        let c: Clip = serde_json::from_str(
            r##"{"id":"c","track":"t","start":0,"duration":5,
                "content":{"type":"solid","color":"#fff"}}"##,
        )
        .unwrap();
        assert!(c.enabled);
        assert_eq!(c.speed, Rational::ONE);
        assert_eq!(c.source_in, Ticks(0));
        assert!(!c.reversed);
    }
}
