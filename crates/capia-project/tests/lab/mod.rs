//! Laboratório de testes do render do projeto: projeto real em diretório temporário, mídia gerada
//! pelo FFmpeg (conteúdo conhecido), comandos pelo Command Engine real.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, unreachable_pub)]

use capia_commands::{Actor, Command, CommandEnvelope, NewClip, Transaction};
use capia_media::{FfprobeBackend, MediaConfig, MediaToolchain};
use capia_model::{AssetId, ClipContent, SequenceId, TrackKind};
use capia_project::{Project, RenderServices};
use capia_store::{StoreOptions, Synchronous};
use capia_time::{FrameRate, Rational, Ticks};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub static N: AtomicU64 = AtomicU64::new(0);

/// Um frame a 30 fps.
pub const F: i64 = 23_520_000;

pub fn toolchain() -> Option<MediaToolchain> {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) if t.ffmpeg.is_some() => Some(t),
        other => {
            assert!(
                std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
                "CAPIA_REQUIRE_FFMPEG is set but ffmpeg is unavailable: {other:?}"
            );
            eprintln!("SKIP (no ffmpeg)");
            None
        }
    }
}

#[macro_export]
macro_rules! need {
    () => {
        match $crate::lab::toolchain() {
            Some(t) => t,
            None => return,
        }
    };
}

pub fn opts() -> StoreOptions {
    StoreOptions {
        synchronous: Synchronous::Normal,
        ..StoreOptions::default()
    }
}

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

pub struct Lab {
    pub dir: PathBuf,
    pub tc: MediaToolchain,
    pub project: Option<Project>,
    pub services: Arc<RenderServices>,
    ops: u64,
}

impl Drop for Lab {
    fn drop(&mut self) {
        self.project = None;
        if std::env::var_os("CAPIA_KEEP_LAB").is_none() {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

impl Lab {
    pub fn new(tag: &str, tc: &MediaToolchain) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "capia-lab-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let project = Project::create(&dir.join("p.capia"), &opts()).unwrap();
        Self {
            services: Arc::new(RenderServices::new(tc.clone())),
            dir,
            tc: tc.clone(),
            project: Some(project),
            ops: 0,
        }
    }

    pub fn project_path(&self) -> PathBuf {
        self.dir.join("p.capia")
    }

    pub fn p(&self) -> &Project {
        self.project.as_ref().unwrap()
    }

    pub fn pm(&mut self) -> &mut Project {
        self.project.as_mut().unwrap()
    }

    /// Fecha e reabre o arquivo (serviços novos: caches de memória frios).
    pub fn reopen(&mut self) {
        self.project = None;
        self.project = Some(Project::open(&self.project_path(), &opts()).unwrap());
        self.services = Arc::new(RenderServices::new(self.tc.clone()));
    }

    pub fn run(&mut self, cmd: Command) {
        self.ops += 1;
        let tx = Transaction {
            transaction_id: None,
            label: "lab".into(),
            base_revision: None,
            commands: vec![CommandEnvelope {
                operation_id: format!("lab{}", self.ops),
                reference: None,
                command: cmd,
            }],
            max_ops: None,
        };
        self.pm()
            .execute(&Actor::user("lab"), tx)
            .unwrap_or_else(|e| panic!("command failed: {e:?}"));
    }

    pub fn import(&mut self, path: &Path) -> AssetId {
        let probe = FfprobeBackend::new(self.tc.clone());
        self.pm()
            .import_asset(&Actor::user("lab"), path, &probe)
            .unwrap()
            .asset_id
    }

    pub fn seq(&mut self, id: &str, fr: FrameRate) {
        self.run(Command::CreateSequence {
            id: Some(id.into()),
            name: id.into(),
            frame_rate: fr,
            sample_rate: None,
        });
    }

    pub fn track(&mut self, seq: &str, id: &str, kind: TrackKind) {
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

    #[allow(clippy::too_many_arguments)]
    pub fn clip(
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
    pub fn media(
        &mut self,
        track: &str,
        id: &str,
        asset: &AssetId,
        start: i64,
        dur: i64,
        source_in: i64,
        v: bool,
        a: bool,
    ) {
        self.clip(
            track,
            id,
            ClipContent::Media {
                asset: asset.clone(),
                has_video: v,
                has_audio: a,
            },
            start,
            dur,
            source_in,
            Rational::ONE,
        );
    }

    pub fn nested(
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

    pub fn prop(&mut self, clip: &str, prop: &str, value: f64) {
        self.run(Command::SetProperty {
            clip: clip.into(),
            prop: prop.into(),
            value,
        });
    }

    pub fn ffmpeg(&self, args: &[&str]) {
        let st = std::process::Command::new(self.tc.ffmpeg.as_ref().unwrap())
            .args(["-v", "error", "-y"])
            .args(args)
            .status()
            .unwrap();
        assert!(st.success(), "ffmpeg {args:?} failed");
    }

    /// MP4 (mpeg4 + AAC) `w×h` a `fps` com `secs` s: vídeo = testsrc2 (cada quadro distinto), áudio
    /// = chirp (nenhuma amostra se repete). `rate` Hz estéreo.
    pub fn gen_av(&self, name: &str, fps: &str, secs: u32, rate: u32) -> PathBuf {
        let out = self.dir.join(name);
        self.ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=64x48:rate={fps}:duration={secs}"),
            "-f",
            "lavfi",
            "-i",
            &format!(
                "aevalsrc=sin(2*PI*t*(300+40*t))|sin(2*PI*t*(500+25*t)):s={rate}:d={secs}:c=stereo"
            ),
            "-c:v",
            "mpeg4",
            "-g",
            "12",
            "-qscale:v",
            "3",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-shortest",
            out.to_str().unwrap(),
        ]);
        out
    }

    /// Clipe com um "flash" (quadro branco + beep de 1 kHz) de 1,0 s a 1,2 s: o instante do flash
    /// no vídeo e o do beep no áudio medem o sincronismo A/V de ponta a ponta.
    pub fn gen_flash(&self, name: &str, fps: &str, secs: u32, rate: u32) -> PathBuf {
        let out = self.dir.join(name);
        self.ffmpeg(&[
            "-f", "lavfi", "-i",
            &format!("color=c=black:s=64x48:r={fps}:d={secs},drawbox=x=0:y=0:w=64:h=48:color=white:t=fill:enable='between(t,1,1.2)'"),
            "-f", "lavfi", "-i",
            &format!("aevalsrc=if(between(t\\,1\\,1.2)\\,0.8*sin(2*PI*1000*t)\\,0)|if(between(t\\,1\\,1.2)\\,0.8*sin(2*PI*1000*t)\\,0):s={rate}:d={secs}:c=stereo"),
            "-c:v", "mpeg4", "-g", "12", "-qscale:v", "2", "-c:a", "aac", "-b:a", "128k",
            "-shortest", out.to_str().unwrap(),
        ]);
        out
    }

    /// `.mov` com vídeo (testsrc2) e áudio PCM s16 (chirp): áudio exato por amostra, seek rápido.
    pub fn gen_mov_pcm(&self, name: &str, fps: &str, secs: &str, rate: u32) -> PathBuf {
        let out = self.dir.join(name);
        self.ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=64x48:rate={fps}:duration={secs}"),
            "-f",
            "lavfi",
            "-i",
            &format!(
                "aevalsrc=sin(2*PI*t*(300+40*t))|sin(2*PI*t*(500+25*t)):s={rate}:d={secs}:c=stereo"
            ),
            "-c:v",
            "mpeg4",
            "-g",
            "12",
            "-qscale:v",
            "3",
            "-c:a",
            "pcm_s16le",
            "-shortest",
            out.to_str().unwrap(),
        ]);
        out
    }
}
