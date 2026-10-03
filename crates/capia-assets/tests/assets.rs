//! Import, identidade, dedup, disponibilidade, relink e cache — com probe sintético (sem ffprobe).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_assets::testing::StaticProbe;
use capia_assets::*;
use capia_media::{MediaErrorCode, MediaKind};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "capia-assets-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(&p, bytes).unwrap();
        p
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn import(p: &Path) -> Result<PreparedAsset, AssetError> {
    prepare_import(p, None, &StaticProbe, 1_000)
}

#[test]
fn imports_video_audio_and_image_with_classification() {
    let t = Tmp::new("kinds");
    let v = import(&t.file("a.mp4", b"video-bytes")).unwrap();
    let a = import(&t.file("a.wav", b"audio-bytes")).unwrap();
    let i = import(&t.file("a.png", b"image-bytes")).unwrap();
    assert_eq!(
        (v.record.kind, a.record.kind, i.record.kind),
        (AssetKind::Video, AssetKind::Audio, AssetKind::Image)
    );
    assert_eq!(v.record.media.kind, MediaKind::Video);
    assert!(v.record.asset_id.as_str().starts_with("ast_"));
    assert_eq!(v.record.asset_id.as_str().len(), 4 + 32);
    assert_eq!(v.record.status, Availability::Online);
    assert_eq!(v.record.size_bytes, 11);
    assert_eq!(v.record.display_name, "a.mp4");
}

#[test]
fn identity_is_content_not_path_or_name() {
    let t = Tmp::new("dedup");
    let a = import(&t.file("one/clip.mp4", b"SAME CONTENT")).unwrap();
    // arquivo igual / path igual
    let again = import(&t.file("one/clip.mp4", b"SAME CONTENT")).unwrap();
    assert_eq!(a.record.asset_id, again.record.asset_id);
    // arquivo igual / path e nome diferentes
    let moved = import(&t.file("two/renamed.mp4", b"SAME CONTENT")).unwrap();
    assert_eq!(a.record.asset_id, moved.record.asset_id);
    assert_eq!(a.record.content_hash, moved.record.content_hash);
    assert_ne!(a.record.location.path, moved.record.location.path);
    // arquivo diferente / nome igual
    let other = import(&t.file("three/clip.mp4", b"DIFFERENT")).unwrap();
    assert_ne!(a.record.asset_id, other.record.asset_id);
    assert_eq!(a.record.display_name, other.record.display_name);
    // arquivo alterado depois da importação ⇒ outro hash/id
    let edited = import(&t.file("one/clip.mp4", b"SAME CONTENT!")).unwrap();
    assert_ne!(a.record.asset_id, edited.record.asset_id);
}

#[test]
fn bad_inputs_are_structured_errors() {
    let t = Tmp::new("bad");
    let code = |r: Result<PreparedAsset, AssetError>| r.unwrap_err().code;
    assert_eq!(
        code(import(&t.0.join("missing.mp4"))),
        AssetErrorCode::AssetFileNotFound
    );
    assert_eq!(
        code(import(&t.file("empty.mp4", b""))),
        AssetErrorCode::AssetEmptyFile
    );
    assert_eq!(code(import(&t.0)), AssetErrorCode::AssetNotRegularFile);
    assert_eq!(
        code(import(&t.file("notmedia.bad", b"hello"))),
        AssetErrorCode::Media(MediaErrorCode::MediaProbeFailed)
    );
    assert_eq!(
        code(import(Path::new(""))),
        AssetErrorCode::AssetPathInvalid
    );
    let long = "x".repeat(5000);
    assert_eq!(
        code(import(Path::new(&long))),
        AssetErrorCode::AssetPathInvalid
    );
}

#[cfg(unix)]
#[test]
fn special_files_and_symlinks() {
    use std::os::unix::fs::symlink;
    let t = Tmp::new("special");
    // FIFO nunca bloqueia o import
    let fifo = t.0.join("pipe.mp4");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        import(&fifo).unwrap_err().code,
        AssetErrorCode::AssetNotRegularFile
    );
    // dispositivo
    assert_eq!(
        import(Path::new("/dev/null")).unwrap_err().code,
        AssetErrorCode::AssetNotRegularFile
    );
    // symlink para arquivo regular: aceito; o caminho guardado é o que o usuário passou
    let real = t.file("real.mp4", b"payload");
    let link = t.0.join("link.mp4");
    symlink(&real, &link).unwrap();
    let p = import(&link).unwrap();
    assert_eq!(p.record.location.to_path_buf(), link);
    assert_eq!(
        p.record.content_hash,
        import(&real).unwrap().record.content_hash
    );
    // symlink pendente e laço de symlinks: erro estruturado, sem travar
    let dangling = t.0.join("dangling.mp4");
    symlink(t.0.join("nowhere"), &dangling).unwrap();
    assert_eq!(
        import(&dangling).unwrap_err().code,
        AssetErrorCode::AssetFileNotFound
    );
    let (l1, l2) = (t.0.join("l1.mp4"), t.0.join("l2.mp4"));
    symlink(&l2, &l1).unwrap();
    symlink(&l1, &l2).unwrap();
    assert!(import(&l1).is_err());
}

#[test]
fn quick_status_and_verify_distinguish_online_offline_modified() {
    let t = Tmp::new("status");
    let f = t.file("v.mp4", b"original bytes");
    let rec = import(&f).unwrap().record;
    assert_eq!(quick_status(&rec, None).0, Availability::Online);
    assert_eq!(
        verify_content(&rec, None).unwrap().status,
        Availability::Online
    );

    // mesmo tamanho, bytes diferentes: o status barato NÃO enxerga; o verify sim (mtime não decide)
    std::fs::write(&f, b"ORIGINAL BYTES").unwrap();
    assert_eq!(quick_status(&rec, None).0, Availability::Online);
    let r = verify_content(&rec, None).unwrap();
    assert_eq!(r.status, Availability::Modified);
    assert_ne!(r.actual.unwrap().hash, rec.content_hash);

    // tamanho diferente: já aparece como modified sem hash
    std::fs::write(&f, b"much longer content now").unwrap();
    assert_eq!(quick_status(&rec, None).0, Availability::Modified);

    // restaurado ⇒ volta a online (o catálogo guarda o hash original)
    std::fs::write(&f, b"original bytes").unwrap();
    assert_eq!(
        verify_content(&rec, None).unwrap().status,
        Availability::Online
    );

    // removido ⇒ offline; as informações conhecidas continuam no registro
    std::fs::remove_file(&f).unwrap();
    assert_eq!(quick_status(&rec, None).0, Availability::Offline);
    let r = verify_content(&rec, None).unwrap();
    assert_eq!((r.status, r.actual), (Availability::Offline, None));
    assert!(rec.content_hash.as_str().starts_with("sha256:"));
}

#[test]
fn project_relative_resolution_survives_moving_the_whole_folder() {
    let t = Tmp::new("rel");
    let proj = t.0.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let f = t.file("proj/media/v.mp4", b"abc");
    let rec = prepare_import(&f, Some(&proj), &StaticProbe, 1)
        .unwrap()
        .record;
    assert_eq!(rec.location.relative.as_deref(), Some("media/v.mp4"));
    // move a pasta inteira do projeto: o absoluto morre, o relativo acha
    let moved = t.0.join("moved");
    std::fs::rename(&proj, &moved).unwrap();
    assert_eq!(quick_status(&rec, None).0, Availability::Offline);
    let (st, path) = quick_status(&rec, Some(&moved));
    assert_eq!(st, Availability::Online);
    assert_eq!(path.unwrap(), moved.join("media/v.mp4"));
}

#[test]
fn relink_requires_the_same_content_and_explains_the_mismatch() {
    let t = Tmp::new("relink");
    let rec = import(&t.file("a/v.mp4", b"the real content"))
        .unwrap()
        .record;
    // outro caminho, mesmo conteúdo ⇒ aceito
    let ok = check_relink(&rec, &t.file("b/copy.mp4", b"the real content")).unwrap();
    assert_eq!(ok.digest.hash, rec.content_hash);
    // conteúdo diferente ⇒ rejeição estruturada com os dois hashes
    let e = check_relink(&rec, &t.file("c/other.mp4", b"something else")).unwrap_err();
    assert_eq!(e.code, AssetErrorCode::AssetHashMismatch);
    let d = e.details.unwrap();
    assert_eq!(d["expected_hash"], Value::from(rec.content_hash.as_str()));
    assert_ne!(d["found_hash"], d["expected_hash"]);
    assert_eq!(d["asset_id"], Value::from(rec.asset_id.as_str()));
    // candidato inexistente / diretório
    assert_eq!(
        check_relink(&rec, &t.0.join("nope.mp4")).unwrap_err().code,
        AssetErrorCode::AssetFileNotFound
    );
    assert_eq!(
        check_relink(&rec, &t.0).unwrap_err().code,
        AssetErrorCode::AssetNotRegularFile
    );
}

#[test]
fn hashing_a_large_file_streams_it() {
    // 48 MiB esparsos: se o hash carregasse o arquivo inteiro, o teste de bloco do módulo `hash`
    // já falharia; aqui conferimos o resultado e o tamanho num arquivo grande de verdade
    let t = Tmp::new("big");
    let p = t.0.join("big.bin");
    let f = std::fs::File::create(&p).unwrap();
    f.set_len(48 * 1024 * 1024).unwrap();
    drop(f);
    let d = hash_file(&p).unwrap();
    assert_eq!(d.size, 48 * 1024 * 1024);
    // tamanho exato lido em streaming
    assert_eq!(d.hash.hex().len(), 64);
    let again = hash_file(&p).unwrap();
    assert_eq!(d, again);
}

#[test]
fn cache_keys_are_deterministic_content_addressed_and_the_cache_is_disposable() {
    let t = Tmp::new("cache");
    let h = hash_file(&t.file("a.bin", b"x")).unwrap().hash;
    let k = |op: &str, p: &[(&str, &str)], prod: &str| CacheKey::new(&h, op, p, prod).unwrap();
    let a = k("thumbnail", &[("at", "0"), ("max", "64")], "p/1");
    // ordem dos parâmetros não importa; qualquer mudança relevante muda a chave
    assert_eq!(a, k("thumbnail", &[("max", "64"), ("at", "0")], "p/1"));
    assert_ne!(a, k("thumbnail", &[("at", "1"), ("max", "64")], "p/1"));
    assert_ne!(a, k("thumbnail", &[("at", "0"), ("max", "64")], "p/2"));
    assert_ne!(a, k("waveform", &[("at", "0"), ("max", "64")], "p/1"));
    // não há ambiguidade entre campos adjacentes
    assert_ne!(k("op", &[("a", "bc")], "p"), k("op", &[("ab", "c")], "p"));
    // nomes que virariam caminhos são rejeitados
    for bad in ["", "../x", "a/b", "A", "x y"] {
        assert!(CacheKey::new(&h, bad, &[], "p").is_err(), "{bad}");
    }
    let c = CacheDir::new(t.0.join("cache"));
    assert!(c.get(&a, "png").is_none());
    let p = c.put(&a, "png", b"PNGDATA").unwrap();
    assert_eq!(c.get(&a, "png").as_deref(), Some(p.as_path()));
    assert_eq!(std::fs::read(&p).unwrap(), b"PNGDATA");
    assert!(c.path_for(&a, "../x").is_err());
    c.clear().unwrap();
    assert!(c.get(&a, "png").is_none());
    c.clear().unwrap(); // idempotente
}

#[test]
fn a_good_alias_beats_an_overwritten_primary_path() {
    let t = Tmp::new("alias");
    let primary = t.file("a/v.mp4", b"original content");
    let mut rec = import(&primary).unwrap().record;
    let alias = t.file("b/copy.mp4", b"original content");
    rec.known_paths.push(alias.display().to_string());
    // o caminho principal é sobrescrito por OUTRO conteúdo de tamanho diferente
    std::fs::write(&primary, b"something else entirely, longer").unwrap();
    let (st, found) = quick_status(&rec, None);
    assert_eq!(
        (st, found.as_deref()),
        (Availability::Online, Some(alias.as_path()))
    );
    let r = verify_content(&rec, None).unwrap();
    assert_eq!(
        (r.status, r.path.as_deref()),
        (Availability::Online, Some(alias.as_path()))
    );
    // e com MESMO tamanho no principal: só o hash distingue; o alias bom continua vencendo
    std::fs::write(&primary, b"ORIGINAL CONTENT").unwrap();
    let r = verify_content(&rec, None).unwrap();
    assert_eq!(
        (r.status, r.path.as_deref()),
        (Availability::Online, Some(alias.as_path()))
    );
    // nenhum candidato com o conteúdo certo ⇒ modified, reportando o principal
    std::fs::write(&alias, b"ALSO CHANGED....").unwrap();
    let r = verify_content(&rec, None).unwrap();
    assert_eq!(
        (r.status, r.path.as_deref()),
        (Availability::Modified, Some(primary.as_path()))
    );
    assert_ne!(r.actual.unwrap().hash, rec.content_hash);
}
