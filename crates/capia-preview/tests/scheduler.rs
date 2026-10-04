//! Scheduler de preview: seek, scrub rápido (supersession), reprodução por relógio, descarte de
//! quadros atrasados, fim da sequence e paridade com o render direto.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_commands::{Actor, Command, CommandEnvelope, Engine, NewClip, Transaction};
use capia_model::{AssetId, ClipContent, Document, SequenceId, TrackKind};
use capia_preview::{
    DropReason, HeadlessSink, ManualClock, PlayState, PreviewScheduler, SystemClock,
};
use capia_render::{
    AudioBuffer, AudioRequest, Image, MediaSource, RenderGraph, RenderSettings, SourceError,
    frame_digest, render_frame,
};
use capia_time::{FrameRate, Rational, Ticks};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const F: i64 = 23_520_000;
const T: Duration = Duration::from_secs(20);

/// Vídeo sintético: o quadro `k` (k = t / F) é uma cor única; `delay` simula um decode lento.
struct Slow {
    delay: Duration,
    calls: AtomicU64,
    cancels: AtomicU64,
}

impl Slow {
    fn new(delay_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            delay: Duration::from_millis(delay_ms),
            calls: AtomicU64::new(0),
            cancels: AtomicU64::new(0),
        })
    }
}

impl MediaSource for Slow {
    fn video_frame(&self, _a: &AssetId, t: Ticks) -> Result<Option<Arc<Image>>, SourceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        let k = (t.0 / F) as u32;
        let c = [
            (k * 7 % 251) as u8,
            (k * 13 % 241) as u8,
            (k * 29 % 239) as u8,
            255,
        ];
        Ok(Some(Arc::new(Image::filled(32, 24, c).unwrap())))
    }
    fn still_image(&self, _a: &AssetId) -> Result<Arc<Image>, SourceError> {
        Err(SourceError::new("NO", "no stills"))
    }
    fn audio(&self, _a: &AssetId, r: AudioRequest) -> Result<AudioBuffer, SourceError> {
        Ok(AudioBuffer::silence(r.sample_rate, r.channels, r.frames))
    }
    fn cancel_pending(&self) {
        self.cancels.fetch_add(1, Ordering::SeqCst);
    }
}

fn doc(frames: i64) -> (Arc<RenderGraph>, SequenceId) {
    let mut e = Engine::new(Document::new(), [3; 32]);
    let mut n = 0;
    let mut run = |c: Command| {
        n += 1;
        let tx = Transaction {
            transaction_id: None,
            label: "t".into(),
            base_revision: None,
            commands: vec![CommandEnvelope {
                operation_id: format!("o{n}"),
                reference: None,
                command: c,
            }],
            max_ops: None,
        };
        e.execute(&Actor::user("t"), tx, 0).unwrap();
    };
    run(Command::CreateSequence {
        id: Some("S".into()),
        name: "S".into(),
        frame_rate: FrameRate::FPS_30,
        sample_rate: None,
        width: None,
        height: None,
        folder: None,
    });
    run(Command::AddTrack {
        sequence: "S".into(),
        id: Some("V".into()),
        kind: TrackKind::Visual,
        name: None,
        role: None,
        magnetic: false,
        index: None,
    });
    run(Command::RegisterAsset {
        asset: capia_model::Asset {
            id: "a".into(),
            name: "a".into(),
            duration: Some(Ticks(100 * capia_time::TICKS_PER_SECOND)),
            has_video: true,
            has_audio: false,
            offline: false,
        },
    });
    run(Command::InsertClip {
        track: "V".into(),
        start: Ticks(0),
        clip: NewClip {
            id: Some("c".into()),
            name: "c".into(),
            duration: Ticks(frames * F),
            content: ClipContent::Media {
                asset: "a".into(),
                has_video: true,
                has_audio: false,
            },
            source_in: Ticks(0),
            speed: Rational::ONE,
            reversed: false,
            properties: Default::default(),
        },
        split_at_insert: false,
        split_new_id: None,
    });
    let seq: SequenceId = "S".into();
    (
        Arc::new(RenderGraph::compile(e.document(), &seq).unwrap()),
        seq,
    )
}

fn sched(
    frames: i64,
    src: &Arc<Slow>,
    clock: Arc<dyn capia_preview::Clock>,
) -> (PreviewScheduler, HeadlessSink, Arc<RenderGraph>, SequenceId) {
    let (g, seq) = doc(frames);
    let sink = HeadlessSink::new();
    let s = PreviewScheduler::new(
        Arc::clone(&g),
        seq.clone(),
        Arc::clone(src) as Arc<dyn MediaSource>,
        RenderSettings::new(32, 24),
        Box::new(sink.clone()),
        clock,
    )
    .unwrap();
    (s, sink, g, seq)
}

fn direct(g: &RenderGraph, seq: &SequenceId, t: Ticks, src: &Arc<Slow>) -> String {
    frame_digest(
        &render_frame(g, seq, t, &RenderSettings::new(32, 24), &**src)
            .unwrap()
            .image,
    )
}

#[test]
fn a_seek_presents_the_same_frame_as_the_direct_render() {
    let src = Slow::new(0);
    let (s, sink, g, seq) = sched(60, &src, Arc::new(SystemClock::new()));
    let id = s.request(Ticks(10 * F));
    assert!(sink.wait_presented(1, T));
    let p = &sink.presented()[0];
    assert_eq!((p.request_id, p.time), (id, Ticks(10 * F)));
    assert_eq!(p.digest, direct(&g, &seq, Ticks(10 * F), &src));
    assert_eq!(s.playhead(), Ticks(10 * F));
    assert_eq!(s.state(), PlayState::Stopped);
}

#[test]
fn rapid_scrub_drops_every_obsolete_request_and_presents_only_the_last() {
    let src = Slow::new(200);
    let (s, sink, g, seq) = sched(60, &src, Arc::new(SystemClock::new()));
    let t1 = s.request(Ticks(10 * F));
    std::thread::sleep(Duration::from_millis(60)); // t1 já está renderizando
    let _t2 = s.request(Ticks(20 * F));
    let _t3 = s.request(Ticks(30 * F));
    let t4 = s.request(Ticks(40 * F));
    assert!(sink.wait_events(2, T));
    std::thread::sleep(Duration::from_millis(500));
    let presented = sink.presented();
    let dropped = sink.dropped();
    assert_eq!(presented.len(), 1, "{presented:?} {dropped:?}");
    assert_eq!(presented[0].request_id, t4);
    assert_eq!(presented[0].digest, direct(&g, &seq, Ticks(40 * F), &src));
    // o t1 (em andamento) foi descartado ao terminar; t2/t3 nem foram renderizados
    assert_eq!(dropped.len(), 1);
    assert_eq!(
        (dropped[0].request_id, dropped[0].reason),
        (t1, DropReason::Superseded)
    );
    assert!(src.cancels.load(Ordering::SeqCst) >= 3);
    let st = s.stats();
    assert_eq!((st.renders, st.presented, st.dropped_superseded), (2, 1, 1));
}

#[test]
fn playback_follows_the_clock_one_frame_at_a_time() {
    let src = Slow::new(0);
    let clock = Arc::new(ManualClock::new());
    let (s, sink, g, seq) = sched(30, &src, clock.clone());
    s.play(Ticks(0), Rational::ONE);
    assert!(sink.wait_presented(1, T));
    for k in 1..=9 {
        clock.advance(Ticks(F));
        s.poke();
        assert!(sink.wait_presented(k + 1, T), "frame {k}");
    }
    let p = sink.presented();
    assert_eq!(p.len(), 10);
    for (k, f) in p.iter().enumerate() {
        assert_eq!(f.frame_index, Some(k as i64));
        assert_eq!(f.time, Ticks(k as i64 * F));
        assert_eq!(f.digest, direct(&g, &seq, f.time, &src));
    }
    assert!(sink.dropped().is_empty());
    assert_eq!(s.state(), PlayState::Playing);
    assert_eq!(s.playhead(), Ticks(9 * F));
    s.pause();
    assert_eq!(s.state(), PlayState::Stopped);
    assert_eq!(s.playhead(), Ticks(9 * F));
}

#[test]
fn a_slow_decode_makes_the_stale_frame_drop_instead_of_lagging_behind() {
    let src = Slow::new(150);
    let clock = Arc::new(ManualClock::new());
    let (s, sink, _g, _seq) = sched(60, &src, clock.clone());
    s.play(Ticks(0), Rational::ONE);
    std::thread::sleep(Duration::from_millis(40)); // o quadro 0 está renderizando
    clock.advance(Ticks(5 * F)); // …e o quadro 5 já venceu
    s.poke();
    assert!(sink.wait_events(2, T));
    assert!(sink.wait_presented(1, T));
    let d = sink.dropped();
    assert_eq!(d[0].reason, DropReason::Late);
    assert_eq!(d[0].time, Ticks(0));
    let p = sink.presented();
    assert_eq!(p[0].frame_index, Some(5), "{p:?}");
    // nunca volta atrás
    clock.advance(Ticks(F));
    s.poke();
    assert!(sink.wait_presented(2, T));
    let idx: Vec<i64> = sink
        .presented()
        .iter()
        .filter_map(|f| f.frame_index)
        .collect();
    assert!(idx.windows(2).all(|w| w[0] < w[1]), "{idx:?}");
    assert!(s.stats().dropped_late >= 1);
}

#[test]
fn playback_stops_at_the_end_of_the_sequence() {
    let src = Slow::new(0);
    let clock = Arc::new(ManualClock::new());
    let (s, sink, _g, _seq) = sched(5, &src, clock.clone());
    s.play(Ticks(0), Rational::ONE);
    assert!(sink.wait_presented(1, T));
    clock.advance(Ticks(10 * F));
    s.poke();
    let t = std::time::Instant::now();
    while s.state() != PlayState::Ended {
        assert!(t.elapsed() < T);
        std::thread::sleep(Duration::from_millis(5));
    }
    // nada além do fim foi apresentado
    assert!(sink.presented().iter().all(|f| f.time.0 < 5 * F));
}

#[test]
fn a_seek_during_playback_stops_it_and_shows_the_target() {
    let src = Slow::new(0);
    let clock = Arc::new(ManualClock::new());
    let (s, sink, g, seq) = sched(60, &src, clock.clone());
    s.play(Ticks(0), Rational::ONE);
    assert!(sink.wait_presented(1, T));
    let id = s.request(Ticks(33 * F));
    assert!(sink.wait_presented(2, T));
    assert_eq!(s.state(), PlayState::Stopped);
    let last = sink.presented().pop().unwrap();
    assert_eq!((last.request_id, last.time), (id, Ticks(33 * F)));
    assert_eq!(last.digest, direct(&g, &seq, Ticks(33 * F), &src));
}
