//! DSL de testes: monta documentos pelo Command Engine real e fornece uma `MediaSource` sintética
//! (cores/senoides conhecidas), de modo que os goldens independem de FFmpeg.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use capia_commands::{Actor, Command, CommandEnvelope, Engine, NewClip, Transaction};
use capia_model::{AssetId, ClipContent, Document, Interp, SequenceId, TrackKind};
use capia_render::{AudioBuffer, AudioRequest, Image, MediaSource, SourceError};
use capia_time::{FrameRate, Rational, Ticks};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Um frame a 30 fps.
pub(crate) const F: i64 = 23_520_000;

pub(crate) struct Dsl {
    pub(crate) e: Engine,
    n: u64,
}

impl Dsl {
    pub(crate) fn new() -> Self {
        Self {
            e: Engine::new(Document::new(), [3; 32]),
            n: 0,
        }
    }

    pub(crate) fn run(&mut self, cmd: Command) {
        self.n += 1;
        let tx = Transaction {
            transaction_id: None,
            label: "t".into(),
            base_revision: None,
            commands: vec![CommandEnvelope {
                operation_id: format!("op{}", self.n),
                reference: None,
                command: cmd,
            }],
            max_ops: None,
        };
        self.e
            .execute(&Actor::user("t"), tx, 0)
            .unwrap_or_else(|e| panic!("command failed: {e:?}"));
    }

    pub(crate) fn seq(&mut self, id: &str, fr: FrameRate) {
        self.run(Command::CreateSequence {
            id: Some(id.into()),
            name: id.into(),
            frame_rate: fr,
            sample_rate: None,
            width: None,
            height: None,
            folder: None,
        });
    }

    pub(crate) fn track(&mut self, seq: &str, id: &str, kind: TrackKind) {
        self.run(Command::AddTrack {
            sequence: SequenceId::from(seq),
            id: Some(id.into()),
            kind,
            name: None,
            role: None,
            magnetic: false,
            index: None,
        });
    }

    pub(crate) fn register(&mut self, asset: &str, secs: i64, video: bool, audio: bool) {
        self.run(Command::RegisterAsset {
            asset: capia_model::Asset {
                id: AssetId::from(asset),
                name: asset.into(),
                duration: Some(Ticks(secs * capia_time::TICKS_PER_SECOND)),
                has_video: video,
                has_audio: audio,
                offline: false,
            },
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn clip(
        &mut self,
        track: &str,
        id: &str,
        content: ClipContent,
        start: i64,
        dur: i64,
        source_in: i64,
        speed: Rational,
    ) {
        self.run(Command::InsertClip {
            track: track.into(),
            start: Ticks(start),
            clip: NewClip {
                id: Some(id.into()),
                name: id.into(),
                duration: Ticks(dur),
                content,
                source_in: Ticks(source_in),
                speed,
                reversed: false,
                properties: Default::default(),
            },
            split_at_insert: false,
            split_new_id: None,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn media(
        &mut self,
        track: &str,
        id: &str,
        asset: &str,
        start: i64,
        dur: i64,
        v: bool,
        a: bool,
    ) {
        self.clip(
            track,
            id,
            ClipContent::Media {
                asset: asset.into(),
                has_video: v,
                has_audio: a,
            },
            start,
            dur,
            0,
            Rational::ONE,
        );
    }

    pub(crate) fn solid(&mut self, track: &str, id: &str, color: &str, start: i64, dur: i64) {
        self.clip(
            track,
            id,
            ClipContent::Solid {
                color: color.into(),
            },
            start,
            dur,
            0,
            Rational::ONE,
        );
    }

    pub(crate) fn nested(
        &mut self,
        track: &str,
        id: &str,
        child: &str,
        start: i64,
        dur: i64,
        source_in: i64,
    ) {
        self.run(Command::InsertNested {
            track: track.into(),
            start: Ticks(start),
            sequence: SequenceId::from(child),
            id: Some(id.into()),
            name: id.into(),
            duration: Some(Ticks(dur)),
            source_in: Ticks(source_in),
            follow_length: false,
            split_at_insert: false,
            split_new_id: None,
        });
    }

    pub(crate) fn prop(&mut self, clip: &str, prop: &str, value: f64) {
        self.run(Command::SetProperty {
            clip: clip.into(),
            prop: prop.into(),
            value,
        });
    }

    pub(crate) fn keyframe(&mut self, clip: &str, prop: &str, at: i64, value: f64) {
        self.run(Command::AddKeyframe {
            clip: clip.into(),
            prop: prop.into(),
            at: Ticks(at),
            value,
            interp: Some(Interp::Linear),
        });
    }

    pub(crate) fn doc(&self) -> &Document {
        self.e.document()
    }
}

pub(crate) struct VideoSpec {
    pub(crate) w: u32,
    pub(crate) h: u32,
    /// (tempo relativo ao 1º quadro, cor).
    pub(crate) frames: Vec<(Ticks, [u8; 4])>,
}

/// Fonte sintética: vídeo = quadros de cor sólida; áudio = senoide (amplitude 0,5, estéreo igual).
#[derive(Default)]
pub(crate) struct Synth {
    pub(crate) videos: BTreeMap<String, VideoSpec>,
    pub(crate) stills: BTreeMap<String, Arc<Image>>,
    /// asset → (frequência Hz, duração em segundos)
    pub(crate) tones: BTreeMap<String, (f64, f64)>,
}

impl Synth {
    pub(crate) fn solid_video(mut self, asset: &str, w: u32, h: u32, color: [u8; 4]) -> Self {
        self.videos.insert(
            asset.into(),
            VideoSpec {
                w,
                h,
                frames: vec![(Ticks(0), color)],
            },
        );
        self
    }

    pub(crate) fn frames_video(
        mut self,
        asset: &str,
        w: u32,
        h: u32,
        frames: Vec<(Ticks, [u8; 4])>,
    ) -> Self {
        self.videos.insert(asset.into(), VideoSpec { w, h, frames });
        self
    }

    pub(crate) fn still(mut self, asset: &str, img: Image) -> Self {
        self.stills.insert(asset.into(), Arc::new(img));
        self
    }

    pub(crate) fn tone(mut self, asset: &str, freq: f64, secs: f64) -> Self {
        self.tones.insert(asset.into(), (freq, secs));
        self
    }
}

impl MediaSource for Synth {
    fn video_frame(&self, asset: &AssetId, t: Ticks) -> Result<Option<Arc<Image>>, SourceError> {
        let v = self
            .videos
            .get(asset.as_str())
            .ok_or_else(|| SourceError::new("MISSING", format!("no video {asset}")))?;
        let idx = v.frames.partition_point(|(ft, _)| *ft <= t);
        if idx == 0 {
            return Ok(None);
        }
        Ok(Some(Arc::new(
            Image::filled(v.w, v.h, v.frames[idx - 1].1).unwrap(),
        )))
    }

    fn still_image(&self, asset: &AssetId) -> Result<Arc<Image>, SourceError> {
        self.stills
            .get(asset.as_str())
            .cloned()
            .ok_or_else(|| SourceError::new("MISSING", format!("no still {asset}")))
    }

    fn audio(&self, asset: &AssetId, req: AudioRequest) -> Result<AudioBuffer, SourceError> {
        let (freq, secs) = *self
            .tones
            .get(asset.as_str())
            .ok_or_else(|| SourceError::new("MISSING", format!("no tone {asset}")))?;
        let limit = (secs * f64::from(req.sample_rate)).round() as u64;
        let ch = req.channels as usize;
        let mut samples = Vec::with_capacity(req.frames as usize * ch);
        for k in 0..req.frames {
            let n = req.start_sample + k;
            let v = if n < limit {
                (0.5 * (2.0 * std::f64::consts::PI * freq * n as f64 / f64::from(req.sample_rate))
                    .sin()) as f32
            } else {
                0.0
            };
            for _ in 0..ch {
                samples.push(v);
            }
        }
        Ok(AudioBuffer {
            sample_rate: req.sample_rate,
            channels: req.channels,
            samples,
        })
    }
}

pub(crate) fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
    img.pixel(x, y)
}
