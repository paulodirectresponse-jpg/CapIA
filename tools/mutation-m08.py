#!/usr/bin/env python3
"""Mutation testing manual da M08 (13 mutações). Aplica UMA alteração por vez num arquivo do
workspace, roda o(s) teste(s) que deveriam detectá-la e restaura o arquivo. Uma mutação é
DETECTADA quando o comando de teste falha. Uso: `python3 tools/mutation-m08.py [id ...]`.
Pré-requisitos: árvore limpa (git status) e FFmpeg no PATH. O arquivo original é sempre restaurado
(mesmo com Ctrl+C)."""
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
        name="impressão rápida aceita como identidade (relink sem SHA-256)",
        file="crates/capia-assets/src/scan.rs",
        old="Ok(d) if d.hash == t.hash => verified.push(f.path.clone()),",
        new="Ok(_) => verified.push(f.path.clone()),",
        cmd="cargo test -p capia-project --test pipeline batch_relink_rejects",
    ),
    dict(
        id=2,
        name="hash não detecta arquivo alterado durante a leitura",
        file="crates/capia-assets/src/hash.rs",
        old="if size != stamp.size || after_handle != stamp || after_path != stamp {",
        new="if false && (size != stamp.size || after_handle != stamp || after_path != stamp) {",
        cmd="cargo test -p capia-assets --test hashing growing_or_rewriting",
    ),
    dict(
        id=3,
        name="cache parcial publicado (sem validar antes do rename)",
        file="crates/capia-assets/src/cache.rs",
        old="        validate(&tmp).map_err(cleanup)?;\n",
        new="",
        cmd="cargo test -p capia-assets --test cache an_invalid_result_is_never_published",
    ),
    dict(
        id=4,
        name="cancelamento sem matar o FFmpeg",
        file="crates/capia-media/src/process.rs",
        old="        if cancel() {\n            kill(&mut child);\n            return Err(MediaError::new(\n                MediaErrorCode::MediaCancelled,",
        new="        if cancel() {\n            return Err(MediaError::new(\n                MediaErrorCode::MediaCancelled,",
        cmd="cargo test -p capia-media --test pipeline streaming_runner_kills_on_cancel",
    ),
    dict(
        id=5,
        name="busca de quadro ignora VFR (usa tempo × 25 fps)",
        file="crates/capia-media/src/index.rs",
        old="    pub fn frame_at_or_before(&self, t: Ticks) -> Option<usize> {\n",
        new="    pub fn frame_at_or_before(&self, t: Ticks) -> Option<usize> {\n        if true {\n            let n = (t.0.max(0) * 25 / TICKS_PER_SECOND) as usize;\n            return (n < self.entries.len()).then_some(n);\n        }\n",
        cmd="cargo test -p capia-media --test pipeline vfr_lookup_and_decode",
    ),
    dict(
        id=6,
        name="decode escolhe o quadro seguinte em vez do anterior",
        file="crates/capia-media/src/decode.rs",
        old="    let i = index.frame_at_or_before(at).ok_or_else(|| {",
        new="    let i = index.frame_at_or_after(at).ok_or_else(|| {",
        cmd="cargo test -p capia-media --test pipeline vfr_lookup_and_decode",
    ),
    dict(
        id=7,
        name="waveform gerado de dados incompletos (falha do decode ignorada)",
        file="crates/capia-media/src/decode.rs",
        old="        Some(s) if s.success() => Ok(total),\n        _ => Err(MediaError::new(",
        new="        Some(s) if s.success() => Ok(total),\n        _ if true => Ok(total),\n        _ => Err(MediaError::new(",
        cmd="cargo test -p capia-media --test pipeline waveform_from_a_real_file",
    ),
    dict(
        id=8,
        name="proxy vira fonte autoritativa (decode usa o proxy do cache)",
        file="crates/capia-project/src/pipeline.rs",
        old="        let (rec, file) = self.online_source(id)?;\n        let cache = self.cache_dir();\n        let env = Env {\n            toolchain,\n            cache: &cache,\n        };\n        Ok(derive::frame_source(&env, &rec, &file, cancel)?)",
        new="        let (rec, mut file) = self.online_source(id)?;\n        let cache = self.cache_dir();\n        let env = Env {\n            toolchain,\n            cache: &cache,\n        };\n        if let Ok(rd) = std::fs::read_dir(cache.root().join(\"proxy\")) {\n            for g in rd.flatten() {\n                if let Some(f) = std::fs::read_dir(g.path()).ok().and_then(|mut r| r.next()).and_then(Result::ok) {\n                    file = f.path();\n                }\n            }\n        }\n        Ok(derive::frame_source(&env, &rec, &file, cancel)?)",
        cmd="cargo test -p capia-project --test pipeline the_proxy_is_never_used",
    ),
    dict(
        id=9,
        name="force relink ignora clips inválidos (sem validação de dependentes)",
        file="crates/capia-commands/src/exec/structure.rs",
        old="    if !conflicts.is_empty() {\n        return Err(CommandError::new(\n            ErrorCode::Conflict,",
        new="    if false && !conflicts.is_empty() {\n        return Err(CommandError::new(\n            ErrorCode::Conflict,",
        cmd="cargo test -p capia-project --test force_relink",
    ),
    dict(
        id=10,
        name="relink em lote por tamanho único (match fraco)",
        file="crates/capia-assets/src/scan.rs",
        old="            match d {\n                Ok(d) if d.hash == t.hash => verified.push(f.path.clone()),",
        new="            match d {\n                Ok(_) if by_size.get(&t.size).map(Vec::len) == Some(1) => verified.push(f.path.clone()),\n                Ok(d) if d.hash == t.hash => verified.push(f.path.clone()),",
        cmd="cargo test -p capia-project --test pipeline batch_relink_matches_by_content",
    ),
    dict(
        id=11,
        name="job interrompido vira completed na recuperação",
        file="crates/capia-store/src/jobs.rs",
        old="\"UPDATE jobs SET state = 'interrupted', finished_ms = ?1, updated_ms = ?1 \\\n             WHERE state IN ('queued', 'running')\"",
        new="\"UPDATE jobs SET state = 'completed', finished_ms = ?1, updated_ms = ?1 \\\n             WHERE state IN ('queued', 'running')\"",
        cmd="cargo test -p capia-store --test jobs recovery_turns",
    ),
    dict(
        id=12,
        name="checksum do cache (CWFM) ignorado",
        file="crates/capia-media/src/waveform.rs",
        old="        if Sha256::digest(body).as_slice() != sum {",
        new="        if false && Sha256::digest(body).as_slice() != sum {",
        cmd="cargo test -p capia-media --lib waveform::tests::binary_round_trip",
    ),
    dict(
        id=13,
        name="lock por chave do cache removido",
        file="crates/capia-assets/src/cache.rs",
        old="            while held.contains(&final_path) {",
        new="            while false && held.contains(&final_path) {",
        cmd="cargo test -p capia-assets --test cache concurrent_producers",
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
            if m["old"] not in original:
                results.append((m["id"], m["name"], "PATTERN-NOT-FOUND", 0))
                print(f"[{m['id']:>2}] PATTERN NOT FOUND in {m['file']}")
                continue
            path.write_text(original.replace(m["old"], m["new"], 1))
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
    (ROOT / "target" / "mutation-m08.json").write_text(json.dumps(results, indent=1))
    return 0 if detected == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
