#!/usr/bin/env python3
"""Mutation testing manual da Fase 2 (16 mutações). Aplica UMA alteração por vez num arquivo do
workspace, roda o(s) teste(s) que deveriam detectá-la e restaura o arquivo. Uma mutação é
DETECTADA quando o comando de teste falha (e o build não quebrou). Uso:
`python3 tools/mutation-phase2.py [id ...]`. Pré-requisitos: árvore limpa e FFmpeg no PATH.
O arquivo original é sempre restaurado (mesmo com Ctrl+C)."""
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

M = [
    dict(
        id=1,
        name="VFR tratado como CFR (quadro por tempo × 25 fps)",
        file="crates/capia-project/src/render.rs",
        old="        let Some(i) = v.index.frame_at_or_before(source_t) else {",
        new="        let Some(i) = Some((source_t.0.max(0) * 25 / capia_time::TICKS_PER_SECOND) as usize).filter(|n| *n < v.index.len()) else {",
        cmd="cargo test -p capia-project --test av_drift vfr_source",
    ),
    dict(
        id=2,
        name="cache de quadros devolve o quadro errado (chave sem o PTS)",
        file="crates/capia-decode/src/service.rs",
        old="            pts,\n            pixel_format: PixelFormat::Rgba8,",
        new="            pts: 0,\n            pixel_format: PixelFormat::Rgba8,",
        cmd="cargo test -p capia-decode --test service sequential_playback",
    ),
    dict(
        id=3,
        name="orçamento em bytes do LRU ignorado",
        file="crates/capia-decode/src/cache.rs",
        old="        while self.used.saturating_add(bytes) > self.budget {",
        new="        while false && self.used.saturating_add(bytes) > self.budget {",
        cmd="cargo test -p capia-decode --lib cache::tests",
    ),
    dict(
        id=4,
        name="quadro obsoleto de scrub é renderizado/apresentado",
        file="crates/capia-preview/src/scheduler.rs",
        old="            let superseded =\n                sh.generation.load(Ordering::SeqCst) != gen_before || c.pending.is_some();",
        new="            let superseded = false;",
        cmd="cargo test -p capia-preview --test scheduler rapid_scrub",
    ),
    dict(
        id=5,
        name="seek de áudio erra por uma amostra",
        file="crates/capia-media/src/audio_index.rs",
        old="    let skip = start_sample.saturating_sub(raw_start_sample).min(have);",
        new="    let skip = (start_sample.saturating_sub(raw_start_sample) + 1).min(have);",
        cmd="cargo test -p capia-media --test audio_seek pcm_wav",
    ),
    dict(
        id=6,
        name="índice de áudio ignorado (decodifica sempre do início: O(início))",
        file="crates/capia-media/src/audio_index.rs",
        old="    if plan.start_frame > 0 {\n        let limit =",
        new="    if false && plan.start_frame > 0 {\n        let limit =",
        cmd="cargo test -p capia-media --test audio_seek seek_cost_does_not_scale",
    ),
    dict(
        id=7,
        name="z-order invertido (tracks de cima para baixo)",
        file="crates/capia-render/src/graph.rs",
        old="        for track in &gs.tracks {\n            if track.kind != TrackKind::Visual || track.hidden {",
        new="        for track in gs.tracks.iter().rev() {\n            if track.kind != TrackKind::Visual || track.hidden {",
        cmd="cargo test -p capia-render --test golden_video",
    ),
    dict(
        id=8,
        name="alpha blend invertido (destino sobre a fonte)",
        file="crates/capia-render/src/image.rs",
        old="pub fn blend_over(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {",
        new="pub fn blend_over(src: [u8; 4], dst: [u8; 4]) -> [u8; 4] {",
        cmd="cargo test -p capia-render --test golden_video",
    ),
    dict(
        id=9,
        name="opacidade ignorada no compositor",
        file="crates/capia-render/src/render.rs",
        old="canvas, img, tr.opacity, tr.scale, tr.pos_x, tr.pos_y, rotation,",
        new="canvas, img, 1.0, tr.scale, tr.pos_x, tr.pos_y, rotation,",
        cmd="cargo test -p capia-render --test golden_video",
    ),
    dict(
        id=10,
        name="mapeamento de tempo do nested errado (tempo da pai em vez do conteúdo)",
        file="crates/capia-render/src/graph.rs",
        old="layers: self.plan_video_at(sequence, content_t, depth + 1)?,",
        new="layers: self.plan_video_at(sequence, t, depth + 1)?,",
        cmd="cargo test -p capia-render --test golden_video",
    ),
    dict(
        id=11,
        name="mapeamento de retime errado (velocidade ignorada)",
        file="crates/capia-render/src/graph.rs",
        old="            let content_t = gc\n                .clip\n                .content_time(t)\n                .map_err(|e| RenderError::new(\"RENDER_TIME_OVERFLOW\", e.to_string()))?;",
        new="            let content_t = Ticks(gc.clip.source_in.0 + (t.0 - gc.clip.start.0));",
        cmd="cargo test -p capia-render --test golden_video",
    ),
    dict(
        id=12,
        name="export parcial publicado como final (erro publica o staging)",
        file="crates/capia-project/src/export.rs",
        old="            Err(e) => {\n                remove_dir_retry(&stage);\n                Err(e)\n            }\n        }\n    }\n\n    /// Detecta",
        new="            Err(e) => {\n                let _ = publish(&stage, out, true);\n                Err(e)\n            }\n        }\n    }\n\n    /// Detecta",
        cmd="cargo test -p capia-project --test export cancelled_export",
    ),
    dict(
        id=13,
        name="proxy vira fonte da verdade do render/export",
        file="crates/capia-project/src/render.rs",
        old="        let e = self.entry(id)?;\n        let file = self.file(e)?;\n        let v = e.rec.media.video().ok_or_else(|| {\n            SourceError::new(\"MEDIA_NO_VIDEO\", format!(\"asset {id} has no video stream\"))\n        })?;\n        let produced",
        new="        let e = self.entry(id)?;\n        let mut pf = self.file(e)?.clone();\n        if let Ok(rd) = std::fs::read_dir(self.cache.root().join(\"proxy\")) {\n            for g in rd.flatten() {\n                if let Some(f) = std::fs::read_dir(g.path()).ok().and_then(|mut r| r.next()).and_then(Result::ok) {\n                    pf = f.path();\n                }\n            }\n        }\n        let file = &pf;\n        let v = e.rec.media.video().ok_or_else(|| {\n            SourceError::new(\"MEDIA_NO_VIDEO\", format!(\"asset {id} has no video stream\"))\n        })?;\n        let produced",
        cmd="cargo test -p capia-project --test parity with_a_proxy_present",
    ),
    dict(
        id=14,
        name="ordem de composição não determinística",
        file="crates/capia-render/src/graph.rs",
        old="                kind,\n            });\n        }\n        Ok(out)",
        new="                kind,\n            });\n        }\n        if out.len() > 1 {\n            let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as usize) % out.len();\n            out.rotate_left(n);\n        }\n        Ok(out)",
        cmd="cargo test -p capia-project --test parity export_is_identical",
    ),
    dict(
        id=15,
        name="encoder GPL/não suportado aceito em silêncio",
        file="crates/capia-media/src/encoder.rs",
        old="            None | Some(EncoderPolicy::Prohibited) => {\n                return Err(MediaError::new(\n                    MediaErrorCode::MediaEncoderProhibited,",
        new="            Some(_) | None => {}\n            #[allow(unreachable_patterns)]\n            None | Some(EncoderPolicy::Prohibited) => {\n                return Err(MediaError::new(\n                    MediaErrorCode::MediaEncoderProhibited,",
        cmd="cargo test -p capia-media --lib encoder::tests",
    ),
    dict(
        id=16,
        name="drift A/V introduzido no export (áudio deslocado 4 quadros)",
        file="crates/capia-project/src/export.rs",
        old="            let (audio, aw) = self.render_audio_range(services, seq, p.audio_range, &s)?;\n            let wav = stage.join(\"audio.wav\");",
        new="            let shifted = TimeRange::new(Ticks(p.audio_range.start.0 + 4 * p.frame_rate.frame_duration().0), p.audio_range.duration);\n            let (audio, aw) = self.render_audio_range(services, seq, shifted, &s)?;\n            let wav = stage.join(\"audio.wav\");",
        cmd="cargo test -p capia-project --test av_drift exported_flash",
    ),
]


def main() -> int:
    want = {int(a) for a in sys.argv[1:]}
    dirty = subprocess.run(["git", "status", "--porcelain"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    if dirty:
        print("working tree is not clean; commit or stash first:\n" + dirty, file=sys.stderr)
        return 2
    results = []
    for m in M:
        if want and m["id"] not in want:
            continue
        path = ROOT / m["file"]
        original = path.read_text()
        backup = path.with_suffix(path.suffix + ".mutbak")
        shutil.copy(path, backup)
        t0 = time.time()
        try:
            edits = m.get("edits") or [(m["old"], m["new"])]
            mutated = original
            missing = [o for o, _ in edits if o not in mutated]
            if missing:
                results.append((m["id"], m["name"], "PATTERN-NOT-FOUND", 0))
                print(f"[{m['id']:>2}] PATTERN NOT FOUND in {m['file']}: {missing[0][:60]!r}")
                continue
            for o, n in edits:
                mutated = mutated.replace(o, n, 1)
            path.write_text(mutated)
            r = subprocess.run(m["cmd"], shell=True, cwd=ROOT, capture_output=True, text=True)
            built = "could not compile" not in (r.stdout + r.stderr)
            detected = r.returncode != 0
            verdict = "DETECTED" if detected and built else ("BROKE-BUILD" if not built else "SURVIVED")
        finally:
            path.write_text(original)
            backup.unlink(missing_ok=True)
        results.append((m["id"], m["name"], verdict, round(time.time() - t0)))
        print(f"[{m['id']:>2}] {verdict:<10} {m['name']}  ({round(time.time() - t0)} s)", flush=True)
    detected = sum(1 for r in results if r[2] == "DETECTED")
    print(f"\n{detected}/{len(results)} detected")
    (ROOT / "target").mkdir(exist_ok=True)
    (ROOT / "target" / "mutation-phase2.json").write_text(json.dumps(results, indent=1))
    return 0 if detected == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
