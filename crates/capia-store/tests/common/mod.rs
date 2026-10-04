//! Utilitários compartilhados pelos testes do store.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use capia_commands::{Actor, Command, CommandEnvelope, Engine, EngineConfig, NewClip, Transaction};
use capia_model::{ClipContent, SequenceId, TrackKind};
use capia_store::{ProjectStore, StoreOptions, Synchronous};
use capia_time::{FrameRate, Rational, Ticks};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub(crate) const FRAME: i64 = 23_520_000;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Diretório temporário removido ao sair (sem dependências externas).
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p =
            std::env::temp_dir().join(format!("capia-{tag}-{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    pub(crate) fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Opções rápidas para testes de volume (a atomicidade vem do WAL, não do fsync).
pub(crate) fn fast() -> StoreOptions {
    StoreOptions {
        synchronous: Synchronous::Normal,
        busy_timeout: Duration::from_millis(3_000),
        snapshot_every: 256,
        ..StoreOptions::default()
    }
}

pub(crate) fn key() -> [u8; 32] {
    [11; 32]
}

pub(crate) fn create(path: &Path, opts: &StoreOptions) -> Engine {
    ProjectStore::create_engine(path, opts, key(), EngineConfig::default()).unwrap()
}

pub(crate) fn open(path: &Path, opts: &StoreOptions) -> Engine {
    ProjectStore::open_engine(path, opts, key(), EngineConfig::default()).unwrap()
}

pub(crate) fn env(op: &str, command: Command) -> CommandEnvelope {
    CommandEnvelope {
        operation_id: op.into(),
        reference: None,
        command,
    }
}

pub(crate) fn tx(label: &str, commands: Vec<CommandEnvelope>) -> Transaction {
    Transaction {
        transaction_id: None,
        label: label.into(),
        base_revision: None,
        commands,
        max_ops: None,
    }
}

pub(crate) fn user() -> Actor {
    Actor::user("test")
}

/// Sequence `S` com tracks `V1` (livre) e `V2` (livre), em uma transação.
pub(crate) fn setup_tx(prefix: &str) -> Transaction {
    let mut cmds = vec![env(
        &format!("{prefix}-seq"),
        Command::CreateSequence {
            id: Some("S".into()),
            name: "S".into(),
            frame_rate: FrameRate::FPS_30,
            sample_rate: None,
            width: None,
            height: None,
            folder: None,
        },
    )];
    for id in ["V1", "V2"] {
        cmds.push(env(
            &format!("{prefix}-{id}"),
            Command::AddTrack {
                sequence: SequenceId::from("S"),
                id: Some(id.into()),
                kind: TrackKind::Visual,
                name: None,
                role: None,
                magnetic: false,
                index: None,
            },
        ));
    }
    tx("setup", cmds)
}

pub(crate) fn insert(op: &str, track: &str, start: i64, id: &str, dur: i64) -> CommandEnvelope {
    env(
        op,
        Command::InsertClip {
            track: track.into(),
            start: Ticks(start * FRAME),
            clip: NewClip {
                id: Some(id.into()),
                name: id.into(),
                duration: Ticks(dur * FRAME),
                content: ClipContent::Solid {
                    color: "#000".into(),
                },
                source_in: Ticks::ZERO,
                speed: Rational::ONE,
                reversed: false,
                properties: Default::default(),
            },
            split_at_insert: false,
            split_new_id: None,
        },
    )
}

pub(crate) fn clip_ids(e: &Engine, track: &str) -> Vec<String> {
    e.document()
        .sequence(&SequenceId::from("S"))
        .map(|s| {
            s.track_clips(&track.into())
                .map(|c| c.id.0.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Digest do estado ignorando o contador de revisão.
pub(crate) fn sem(doc: &capia_model::Document) -> String {
    let mut d = doc.clone();
    d.revision = 0;
    capia_commands::document_digest(&d)
}
