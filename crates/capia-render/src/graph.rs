//! Render graph (ADR-063): a sequence compilada para uma forma imutável e extensível, e o plano de
//! layers de um instante. `Document → RenderGraph → plano(t) → decode → composição`.

use crate::error::{RenderError, RenderWarning};
use capia_model::{
    Animatable, AssetId, Clip, ClipContent, ClipId, Document, SequenceId, TextStyle, TrackId,
    TrackKind, TransitionKind, property_spec,
};
use capia_time::{FrameRate, TICKS_PER_SECOND, Ticks};
use std::collections::BTreeMap;

/// Mesma profundidade máxima do modelo (ADR-045).
pub const MAX_NEST_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub enum GraphClipKind {
    Media {
        asset: AssetId,
        has_video: bool,
        has_audio: bool,
    },
    Image {
        asset: AssetId,
    },
    Solid {
        color: String,
    },
    Text {
        text: String,
        style: TextStyle,
    },
    Nested {
        sequence: SequenceId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphClip {
    pub clip: Clip,
    pub kind: GraphClipKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphTrack {
    pub id: TrackId,
    pub kind: TrackKind,
    pub hidden: bool,
    pub muted: bool,
    pub solo: bool,
    /// Ordenados por `(start, id)`.
    pub clips: Vec<GraphClip>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphSequence {
    pub id: SequenceId,
    pub frame_rate: FrameRate,
    pub sample_rate: u32,
    /// Ordem da sequence: a primeira track é a de **baixo** (z-order crescente).
    pub tracks: Vec<GraphTrack>,
    pub duration: Ticks,
}

/// Valores avaliados de um layer num instante.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub opacity: f64,
    pub scale: f64,
    pub pos_x: f64,
    pub pos_y: f64,
    pub rotation: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayerKind {
    /// Tempo de fonte (ticks, relativo ao 1º quadro da mídia).
    Media {
        asset: AssetId,
        source_t: Ticks,
    },
    Image {
        asset: AssetId,
    },
    Solid {
        color: String,
    },
    Nested {
        sequence: SequenceId,
        layers: Vec<LayerPlan>,
    },
    /// Texto/legenda (renderizado por `text::render_text`, o mesmo caminho em preview e export).
    Text {
        text: String,
        style: TextStyle,
    },
    /// Conteúdo sem renderizador (reservado; hoje nada o produz).
    Unsupported {
        what: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayerPlan {
    pub clip: ClipId,
    pub track: TrackId,
    pub transform: Transform,
    /// Deslocamento horizontal de entrada (`SlideIn`) como fração da **largura do quadro**
    /// (1 = totalmente à direita, 0 = no lugar); somado ao `position_x` na composição.
    pub slide_x: f64,
    pub kind: LayerKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderGraph {
    pub root: SequenceId,
    /// `BTreeMap`: ordem determinística.
    pub sequences: BTreeMap<SequenceId, GraphSequence>,
}

impl RenderGraph {
    /// Compila `root` e todas as sequences alcançáveis por nested.
    pub fn compile(doc: &Document, root: &SequenceId) -> Result<Self, RenderError> {
        let mut sequences = BTreeMap::new();
        let mut stack = vec![root.clone()];
        while let Some(id) = stack.pop() {
            if sequences.contains_key(&id) {
                continue;
            }
            let seq = doc.sequence(&id).ok_or_else(|| {
                RenderError::new(
                    "RENDER_SEQUENCE_NOT_FOUND",
                    format!("sequence {id} does not exist"),
                )
            })?;
            let mut tracks = Vec::new();
            for t in seq.tracks() {
                let mut clips: Vec<GraphClip> = seq
                    .track_clips(&t.id)
                    .map(|c| {
                        let kind = match &c.content {
                            ClipContent::Media {
                                asset,
                                has_video,
                                has_audio,
                            } => GraphClipKind::Media {
                                asset: asset.clone(),
                                has_video: *has_video,
                                has_audio: *has_audio,
                            },
                            ClipContent::Image { asset } => GraphClipKind::Image {
                                asset: asset.clone(),
                            },
                            ClipContent::Solid { color } => GraphClipKind::Solid {
                                color: color.clone(),
                            },
                            ClipContent::Text { text, style } => GraphClipKind::Text {
                                text: text.clone(),
                                style: style.clone(),
                            },
                            ClipContent::Nested { sequence, .. } => GraphClipKind::Nested {
                                sequence: sequence.clone(),
                            },
                        };
                        GraphClip {
                            clip: c.clone(),
                            kind,
                        }
                    })
                    .collect();
                clips.sort_by(|a, b| (a.clip.start, &a.clip.id).cmp(&(b.clip.start, &b.clip.id)));
                for c in &clips {
                    if let GraphClipKind::Nested { sequence } = &c.kind {
                        stack.push(sequence.clone());
                    }
                }
                tracks.push(GraphTrack {
                    id: t.id.clone(),
                    kind: t.kind,
                    hidden: t.hidden,
                    muted: t.muted,
                    solo: t.solo,
                    clips,
                });
            }
            sequences.insert(
                id.clone(),
                GraphSequence {
                    id,
                    frame_rate: seq.frame_rate(),
                    sample_rate: seq.header.sample_rate,
                    tracks,
                    duration: seq.duration(),
                },
            );
        }
        Ok(Self {
            root: root.clone(),
            sequences,
        })
    }

    pub fn sequence(&self, id: &SequenceId) -> Result<&GraphSequence, RenderError> {
        self.sequences.get(id).ok_or_else(|| {
            RenderError::new(
                "RENDER_SEQUENCE_NOT_FOUND",
                format!("sequence {id} is not in the graph"),
            )
        })
    }

    /// Layers visíveis em `t` (tempo **da sequence** `seq`), de baixo para cima.
    pub fn plan_video(&self, seq: &SequenceId, t: Ticks) -> Result<Vec<LayerPlan>, RenderError> {
        self.plan_video_at(seq, t, 0)
    }

    fn plan_video_at(
        &self,
        seq: &SequenceId,
        t: Ticks,
        depth: usize,
    ) -> Result<Vec<LayerPlan>, RenderError> {
        if depth > MAX_NEST_DEPTH {
            return Err(RenderError::new(
                "RENDER_NEST_DEPTH",
                "nested sequences are too deep",
            ));
        }
        let gs = self.sequence(seq)?;
        let mut out = Vec::new();
        for track in &gs.tracks {
            if track.kind != TrackKind::Visual || track.hidden {
                continue;
            }
            let clips = &track.clips;
            let idx = clips.partition_point(|c| c.clip.start <= t);
            let cur = idx
                .checked_sub(1)
                .and_then(|i| clips.get(i))
                .filter(|c| t < c.clip.end());
            let prev_of_cur = idx
                .checked_sub(2)
                .and_then(|i| clips.get(i))
                .zip(cur)
                .filter(|(p, c)| p.clip.end() == c.clip.start)
                .map(|(p, _)| p);
            let next = clips.get(idx);
            let mut push = |gc: &GraphClip, mul: f64, slide: f64| -> Result<(), RenderError> {
                if !gc.clip.enabled {
                    return Ok(());
                }
                if let Some(l) = self.layer_for(gc, &track.id, t, mul, slide, depth)? {
                    out.push(l);
                }
                Ok(())
            };
            // 1) transição de entrada do clip atual (corte com o anterior adjacente)
            let mut cur_mul = 1.0;
            let mut cur_slide = 0.0;
            if let Some(gc) = cur
                && let Some(tr) = gc.clip.transition_in
            {
                let s0 = gc.clip.start;
                let half = tr.duration.0 / 2;
                match tr.kind {
                    TransitionKind::Dissolve if t.0 < s0.0 + half => {
                        if let Some(p) = prev_of_cur {
                            push(p, 1.0, 0.0)?;
                            cur_mul = ratio(t.0 - (s0.0 - half), tr.duration.0);
                        }
                    }
                    TransitionKind::Fade if t.0 < s0.0 + half && prev_of_cur.is_some() => {
                        cur_mul = ratio(t.0 - s0.0, half);
                    }
                    TransitionKind::SlideIn if t.0 < s0.0 + tr.duration.0 => {
                        cur_slide = 1.0 - ratio(t.0 - s0.0, tr.duration.0);
                    }
                    _ => {}
                }
            }
            // 2) o clip atual sai para o próximo (fade) ou o próximo já entra (dissolve)
            let mut ext_next: Option<(&GraphClip, f64)> = None;
            if let (Some(gc), Some(n)) = (cur, next)
                && gc.clip.end() == n.clip.start
                && let Some(tr) = n.clip.transition_in
            {
                let half = tr.duration.0 / 2;
                if t.0 >= n.clip.start.0 - half {
                    let u = ratio(t.0 - (n.clip.start.0 - half), tr.duration.0);
                    match tr.kind {
                        TransitionKind::Fade => {
                            cur_mul *= 1.0 - ratio(t.0 - (n.clip.start.0 - half), half);
                        }
                        TransitionKind::Dissolve => ext_next = Some((n, u)),
                        TransitionKind::SlideIn => {}
                    }
                }
            }
            if let Some(gc) = cur {
                push(gc, cur_mul, cur_slide)?;
            }
            if let Some((n, u)) = ext_next {
                push(n, u, 0.0)?;
            }
        }
        Ok(out)
    }

    /// Plano de um clip em `t` (que pode estar **fora** do trecho do clip: extensão de dissolve).
    /// `mul` multiplica a opacidade (transição); `slide` é a entrada deslizante.
    fn layer_for(
        &self,
        gc: &GraphClip,
        track: &TrackId,
        t: Ticks,
        mul: f64,
        slide: f64,
        depth: usize,
    ) -> Result<Option<LayerPlan>, RenderError> {
        let content_t = content_time_ext(&gc.clip, t)
            .map_err(|e| RenderError::new("RENDER_TIME_OVERFLOW", e))?;
        let mut transform = eval_transform(&gc.clip, content_t);
        transform.opacity *= mul.clamp(0.0, 1.0) * fade_factor(&gc.clip, t);
        let kind = match &gc.kind {
            GraphClipKind::Media {
                asset,
                has_video: true,
                ..
            } => LayerKind::Media {
                asset: asset.clone(),
                // ADR-120: antes do 1º quadro (sem handle na dissolução) congela o 1º quadro
                source_t: Ticks(content_t.0.max(0)),
            },
            GraphClipKind::Media { .. } => return Ok(None), // só áudio
            GraphClipKind::Image { asset } => LayerKind::Image {
                asset: asset.clone(),
            },
            GraphClipKind::Solid { color } => LayerKind::Solid {
                color: color.clone(),
            },
            GraphClipKind::Text { text, style } => LayerKind::Text {
                text: text.clone(),
                style: style.clone(),
            },
            GraphClipKind::Nested { sequence } => LayerKind::Nested {
                sequence: sequence.clone(),
                layers: self.plan_video_at(sequence, content_t, depth + 1)?,
            },
        };
        Ok(Some(LayerPlan {
            clip: gc.clip.id.clone(),
            track: track.clone(),
            transform,
            slide_x: slide,
            kind,
        }))
    }

    /// Avisos estáticos do grafo (reservado; texto e transições são renderizados), ordenados e
    /// únicos.
    pub fn warnings(&self) -> Vec<RenderWarning> {
        let mut w: Vec<RenderWarning> = Vec::new();
        for gs in self.sequences.values() {
            for t in &gs.tracks {
                for c in &t.clips {
                    if let GraphClipKind::Text { style, .. } = &c.kind
                        && !crate::text::KNOWN_FAMILIES.contains(&style.font_family.as_str())
                    {
                        w.push(RenderWarning::new(
                            "FONT_FALLBACK",
                            format!(
                                "clip {} uses font `{}`, which is not available; using `sans`",
                                c.clip.id, style.font_family
                            ),
                        ));
                    }
                }
            }
        }
        w.sort();
        w.dedup();
        w
    }
}

/// Avalia as propriedades visuais (valor padrão se ausentes) no tempo de conteúdo `content_t`.
pub(crate) fn eval_transform(clip: &Clip, content_t: Ticks) -> Transform {
    let get = |name: &str| -> f64 {
        let Some(spec) = property_spec(name) else {
            return 0.0;
        };
        match clip.properties.get(name) {
            Some(a) => a.eval(content_t, spec),
            None => Animatable::Static(spec.default).eval(content_t, spec),
        }
    };
    Transform {
        opacity: get("opacity"),
        scale: get("scale"),
        pos_x: get("position_x"),
        pos_y: get("position_y"),
        rotation: get("rotation"),
    }
}

/// `num / den` limitado a `[0, 1]` (progresso de transição/fade).
pub(crate) fn ratio(num: i64, den: i64) -> f64 {
    if den <= 0 {
        return 1.0;
    }
    (num as f64 / den as f64).clamp(0.0, 1.0)
}

/// Tempo de conteúdo em `t`, também **fora** do trecho do clip (extensão para dissolve): mesma
/// fórmula linear `source_in + (t − start)·speed`, em inteiros de 128 bits, half-up. Dentro do
/// clip delega a `Clip::content_time` (comportamento exato e reverso inalterados).
pub(crate) fn content_time_ext(clip: &Clip, t: Ticks) -> Result<Ticks, String> {
    if t >= clip.start && t < clip.end() {
        return clip.content_time(t).map_err(|e| e.to_string());
    }
    let local = i128::from(t.0) - i128::from(clip.start.0);
    let (n, d) = (i128::from(clip.speed.num()), i128::from(clip.speed.den()));
    let scaled = (2 * local * n + d).div_euclid(2 * d);
    i64::try_from(i128::from(clip.source_in.0) + scaled)
        .map(Ticks)
        .map_err(|_| "content time overflows".to_owned())
}

/// Fator de fade-in/fade-out (propriedades `fade_in`/`fade_out`, em segundos) em `t`.
pub(crate) fn fade_factor(clip: &Clip, t: Ticks) -> f64 {
    let secs = |name: &str| -> f64 {
        let Some(spec) = property_spec(name) else {
            return 0.0;
        };
        clip.properties
            .get(name)
            .map_or(spec.default, |a| a.eval(clip.source_in, spec))
    };
    let (fi, fo) = (secs("fade_in"), secs("fade_out"));
    if fi <= 0.0 && fo <= 0.0 {
        return 1.0;
    }
    let tps = TICKS_PER_SECOND as f64;
    let local = (t.0 - clip.start.0) as f64 / tps;
    let remain = (clip.end().0 - t.0) as f64 / tps;
    let mut f = 1.0f64;
    if fi > 0.0 {
        f = f.min(local / fi);
    }
    if fo > 0.0 {
        f = f.min(remain / fo);
    }
    f.clamp(0.0, 1.0)
}
