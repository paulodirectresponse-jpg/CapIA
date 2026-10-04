use crate::ids::{AssetId, ClipId, SequenceId, TrackId};
use crate::property::PropertySet;
use capia_time::{Rational, Ticks, TimeError, TimeRange};
use serde::{Deserialize, Serialize};

/// Velocidade de um clip: de 0,01× a 100× (D-S7-1). Fora disso: `OUT_OF_RANGE`.
pub fn speed_in_range(speed: Rational) -> bool {
    speed >= Rational::new(1, 100).unwrap_or(Rational::ONE) && speed <= Rational::from_int(100)
}

/// Alinhamento horizontal do texto.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// Estilo de texto determinístico (um único renderizador serve preview e export). Tudo inteiro/
/// string: o tamanho é relativo à **altura do quadro** (`size_permille` = 1/1000 da altura), então o
/// mesmo estilo rende igual em qualquer resolução.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextStyle {
    /// `sans` (padrão, fonte embutida) — famílias desconhecidas caem em `sans` com aviso.
    #[serde(default = "default_font_family")]
    pub font_family: String,
    /// 10..=500 (‰ da altura do quadro).
    #[serde(default = "default_size_permille")]
    pub size_permille: u32,
    /// 100..=900 (`>= 600` usa a variante negrito).
    #[serde(default = "default_weight")]
    pub weight: u16,
    #[serde(default)]
    pub align: TextAlign,
    /// `#RRGGBB` ou `#RRGGBBAA`.
    #[serde(default = "default_text_color")]
    pub color: String,
    /// Caixa de fundo opcional (`#RRGGBB[AA]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Contorno opcional (`#RRGGBB[AA]`) com `stroke_permille` (‰ da altura; 1..=50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub stroke_permille: u32,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn default_font_family() -> String {
    "sans".to_owned()
}

fn default_size_permille() -> u32 {
    60
}

fn default_weight() -> u16 {
    400
}

fn default_text_color() -> String {
    "#FFFFFF".to_owned()
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_family: default_font_family(),
            size_permille: default_size_permille(),
            weight: default_weight(),
            align: TextAlign::Center,
            color: default_text_color(),
            background: None,
            stroke: None,
            stroke_permille: 0,
        }
    }
}

fn valid_hex_color(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() == 7 || b.len() == 9) && b[0] == b'#' && b[1..].iter().all(u8::is_ascii_hexdigit)
}

impl TextStyle {
    /// Estilo padrão (não é serializado: mantém o digest de projetos anteriores).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// `Err` com o motivo se algum campo estiver fora da faixa.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(10..=500).contains(&self.size_permille) {
            return Err("size_permille must be within 10..=500");
        }
        if !(100..=900).contains(&self.weight) {
            return Err("weight must be within 100..=900");
        }
        if self.font_family.is_empty() || self.font_family.len() > 64 {
            return Err("font_family must have 1..=64 characters");
        }
        for c in [
            Some(&self.color),
            self.background.as_ref(),
            self.stroke.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if !valid_hex_color(c) {
                return Err("colors must be #RRGGBB or #RRGGBBAA");
            }
        }
        if self.stroke_permille > 50 || (self.stroke.is_some() && self.stroke_permille == 0) {
            return Err("stroke needs stroke_permille within 1..=50");
        }
        Ok(())
    }
}

/// Transição **na entrada** de um clip (no corte com o clip anterior da mesma track).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    /// Cross dissolve centrado no corte (precisa de *handles* de mídia nos dois lados: metade da
    /// duração além do fim do anterior e antes do início deste).
    Dissolve,
    /// Mergulho: o anterior some e este surge, sem handles (cada metade fica do seu lado do corte).
    Fade,
    /// Este clip entra deslizando da direita sobre o que está abaixo, durante `duration`.
    SlideIn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    /// Duração total (> 0), alinhada ao tick; limitada pela validação do comando.
    pub duration: Ticks,
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
        #[serde(default, skip_serializing_if = "TextStyle::is_default")]
        style: TextStyle,
    },
    Solid {
        color: String,
    },
    /// Outra sequence do projeto (referência compartilhada; DAG com profundidade ≤ 16). Com
    /// `follow_length`, a duração do clip acompanha a da sequence filha (ADR-045).
    Nested {
        sequence: SequenceId,
        #[serde(default, skip_serializing_if = "core::ops::Not::not")]
        follow_length: bool,
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
    /// Grupo de clips (movem juntos; rótulo livre único por sequence).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Transição na entrada deste clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_in: Option<Transition>,
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
            group: None,
            transition_in: None,
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
