//! Render graph (ADR-063): a sequence compilada para uma forma imutável e extensível, e o plano de
//! layers de um instante. `Document → RenderGraph → plano(t) → decode → composição`.

use crate::error::{RenderError, RenderWarning};
use capia_model::{
    Animatable, AssetId, Clip, ClipContent, ClipId, Document, SequenceId, TrackId, TrackKind,
    property_spec,
};
use capia_time::{FrameRate, Ticks};
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
    Text,
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
    /// Conteúdo que a Fase 2 não renderiza (texto); gera aviso.
    Unsupported {
        what: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayerPlan {
    pub clip: ClipId,
    pub track: TrackId,
    pub transform: Transform,
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
                            ClipContent::Text { .. } => GraphClipKind::Text,
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
            let Some(gc) = active_clip(&track.clips, t) else {
                continue;
            };
            if !gc.clip.enabled {
                continue;
            }
            let content_t = gc
                .clip
                .content_time(t)
                .map_err(|e| RenderError::new("RENDER_TIME_OVERFLOW", e.to_string()))?;
            let transform = eval_transform(&gc.clip, content_t);
            let kind = match &gc.kind {
                GraphClipKind::Media {
                    asset,
                    has_video: true,
                    ..
                } => LayerKind::Media {
                    asset: asset.clone(),
                    source_t: content_t,
                },
                GraphClipKind::Media { .. } => continue, // só áudio
                GraphClipKind::Image { asset } => LayerKind::Image {
                    asset: asset.clone(),
                },
                GraphClipKind::Solid { color } => LayerKind::Solid {
                    color: color.clone(),
                },
                GraphClipKind::Text => LayerKind::Unsupported { what: "text" },
                GraphClipKind::Nested { sequence } => LayerKind::Nested {
                    sequence: sequence.clone(),
                    layers: self.plan_video_at(sequence, content_t, depth + 1)?,
                },
            };
            out.push(LayerPlan {
                clip: gc.clip.id.clone(),
                track: track.id.clone(),
                transform,
                kind,
            });
        }
        Ok(out)
    }

    /// Avisos estáticos do grafo (conteúdo que a Fase 2 não renderiza), ordenados e únicos.
    pub fn warnings(&self) -> Vec<RenderWarning> {
        let mut w = Vec::new();
        for gs in self.sequences.values() {
            for t in &gs.tracks {
                for c in &t.clips {
                    if matches!(c.kind, GraphClipKind::Text) {
                        w.push(RenderWarning::new(
                            "TEXT_NOT_RENDERED",
                            format!(
                                "clip {} is text; text rendering is not part of Phase 2",
                                c.clip.id
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

/// Clip ativo em `t` (`start ≤ t < start+duração`) numa lista ordenada por início.
pub(crate) fn active_clip(clips: &[GraphClip], t: Ticks) -> Option<&GraphClip> {
    let idx = clips.partition_point(|c| c.clip.start <= t);
    let c = clips.get(idx.checked_sub(1)?)?;
    (t < c.clip.end()).then_some(c)
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
