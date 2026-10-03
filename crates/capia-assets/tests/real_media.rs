//! Import com o ffprobe REAL sobre as fixtures versionadas (CAPIA_REQUIRE_FFMPEG=1 no CI).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_assets::*;
use capia_media::{FfprobeBackend, MediaConfig, MediaErrorCode, MediaKind, MediaToolchain};
use capia_time::{TICKS_PER_SECOND, Ticks};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

fn toolchain() -> Option<MediaToolchain> {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) => Some(t),
        Err(e) => {
            assert!(
                std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
                "ffprobe required: {e}"
            );
            eprintln!("SKIP (no ffprobe): {e}");
            None
        }
    }
}

#[test]
fn real_files_import_with_normalized_metadata() {
    let Some(t) = toolchain() else { return };
    let probe = FfprobeBackend::new(t);
    let v = prepare_import(&fixture("video_audio.mp4"), None, &probe, 1)
        .unwrap()
        .record;
    assert_eq!(v.kind, AssetKind::Video);
    assert_eq!(v.media.duration, Some(Ticks(TICKS_PER_SECOND)));
    assert!(v.media.has_audio());
    let a = prepare_import(&fixture("audio.wav"), None, &probe, 1)
        .unwrap()
        .record;
    assert_eq!((a.kind, a.media.kind), (AssetKind::Audio, MediaKind::Audio));
    let i = prepare_import(&fixture("image_alpha.png"), None, &probe, 1)
        .unwrap()
        .record;
    assert_eq!(i.kind, AssetKind::Image);
    assert_eq!(i.media.duration, None);
    let e = prepare_import(&fixture("invalid.mp4"), None, &probe, 1).unwrap_err();
    assert_eq!(
        e.code,
        AssetErrorCode::Media(MediaErrorCode::MediaProbeFailed)
    );
    // determinismo: dois imports do mesmo arquivo dão registros semanticamente idênticos
    let v2 = prepare_import(&fixture("video_audio.mp4"), None, &probe, 99)
        .unwrap()
        .record;
    assert_eq!(
        (v.asset_id.clone(), v.content_hash.clone(), v.media.clone()),
        (v2.asset_id, v2.content_hash, v2.media)
    );
}

#[test]
fn thumbnail_goes_to_the_cache_and_is_reused_and_survives_cache_deletion() {
    let Some(t) = toolchain() else { return };
    if t.ffmpeg.is_none() {
        assert!(
            std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
            "ffmpeg required"
        );
        return;
    }
    let probe = FfprobeBackend::new(t.clone());
    let f = fixture("video_audio.mp4");
    let rec = prepare_import(&f, None, &probe, 1).unwrap().record;
    let dir = std::env::temp_dir().join(format!("capia-thumb-cache-{}", std::process::id()));
    let cache = CacheDir::new(&dir);
    let at = Ticks(TICKS_PER_SECOND / 2);
    let p1 = ensure_thumbnail(&t, &rec, &f, at, 32, &cache).unwrap();
    assert!(std::fs::metadata(&p1).unwrap().len() > 0);
    let p2 = ensure_thumbnail(&t, &rec, &f, at, 32, &cache).unwrap();
    assert_eq!(p1, p2);
    // outro instante/tamanho ⇒ outra entrada
    let p3 = ensure_thumbnail(&t, &rec, &f, at, 48, &cache).unwrap();
    assert_ne!(p1, p3);
    // apagar o cache não quebra nada: regenera
    cache.clear().unwrap();
    let p4 = ensure_thumbnail(&t, &rec, &f, at, 32, &cache).unwrap();
    assert_eq!(p1, p4);
    // áudio não tem quadro
    let wav = fixture("audio.wav");
    let arec = prepare_import(&wav, None, &probe, 1).unwrap().record;
    assert!(ensure_thumbnail(&t, &arec, &wav, at, 32, &cache).is_err());
    // arquivo trocado por outro conteúdo ⇒ recusa (e cache miss: outra chave não é gerada)
    let other = fixture("video_only.mp4");
    cache.clear().unwrap();
    let e = ensure_thumbnail(&t, &rec, &other, at, 32, &cache).unwrap_err();
    assert_eq!(e.code, AssetErrorCode::AssetHashMismatch);
    let _ = std::fs::remove_dir_all(dir);
}
