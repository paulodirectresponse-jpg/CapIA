#!/usr/bin/env python3
"""Mutation testing manual da Fase 5 (autonomia). Uma alteração por vez; roda o teste que deve
detectá-la e restaura o arquivo (sempre). DETECTED = o teste falhou e o build não quebrou.
Uso: `python3 tools/mutation-phase5.py [id ...]` (árvore limpa; FFmpeg no PATH)."""
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
I = "crates/capia-intelligence"
T = "CARGO_INCREMENTAL=0 cargo test -q -p capia-intelligence --test"

M = [
    dict(id=1, name="memória User/Client nasce ATIVA (promoção automática)",
         file=f"{I}/src/autonomy/memory.rs",
         old="            status: if activate {\n                MemoryStatus::Active\n            } else {\n                MemoryStatus::Proposed\n            },",
         new="            status: MemoryStatus::Active,",
         cmd=f"{T} autonomy_memory"),
    dict(id=2, name="política do projeto ativa memória de qualquer escopo",
         file=f"{I}/src/autonomy/memory.rs",
         old="let activate = d.scope == MemoryScope::Project && project_auto_activate;",
         new="let activate = project_auto_activate;",
         cmd=f"{T} autonomy_memory"),
    dict(id=3, name="EDIT alcançável sem VALIDATE_PLAN (escrita antes do plano validado)",
         file=f"{I}/src/autonomy/machine.rs",
         old="        (S::Plan, O::Success) => go(S::ValidatePlan),",
         new="        (S::Plan, O::Success) => go(S::Edit),",
         cmd=f"{T} autonomy_properties"),
    dict(id=4, name="EFEITO colidido executa de novo (INSERT OR REPLACE no claim)",
         file="crates/capia-store/src/autonomy.rs",
         old='"INSERT OR IGNORE INTO ai_side_effects(',
         new='"INSERT OR REPLACE INTO ai_side_effects(',
         cmd="CARGO_INCREMENTAL=0 cargo test -q -p capia-store --test autonomy_store"),
    dict(id=5, name="orçamento: reserva ignora o teto",
         file="crates/capia-store/src/autonomy.rs",
         old="            && committed.saturating_add(micros) > l\n",
         new="            && false && committed.saturating_add(micros) > l\n",
         cmd="CARGO_INCREMENTAL=0 cargo test -q -p capia-store --test autonomy_store"),
    dict(id=6, name="drift manual ignorado: aplica sem comparar o diff validado",
         file=f"{I}/src/autonomy/stages.rs",
         old="                if validated.is_none_or(|v| v.diff_digest != digest) {",
         new="                if false && validated.is_none_or(|v| v.diff_digest != digest) {",
         cmd=f"{T} autonomy_scenarios -- drift"),
    dict(id=7, name="licença desconhecida entra sem aprovação",
         file=f"{I}/src/autonomy/gateway.rs",
         old="            if p.unknown_license_requires_approval {\n                LicenseVerdict::NeedsApproval",
         new="            if false && p.unknown_license_requires_approval {\n                LicenseVerdict::NeedsApproval",
         cmd=f"{T} autonomy_scenarios -- missing_broll"),
    dict(id=8, name="asset adquirido some (fica só no cache descartável)",
         file=f"{I}/src/autonomy/stages.rs",
         old="        if to.exists() {\n            let _ = std::fs::remove_file(from);\n            return to;\n        }",
         new="        if true {\n            let _ = std::fs::remove_file(from);\n            return from.to_path_buf();\n        }",
         cmd=f"{T} autonomy_scenarios -- missing_broll"),
    dict(id=9, name="undo seletivo apaga edição manual dependente (sem conflito de dependência)",
         file="crates/capia-commands/src/engine.rs",
         old="        if after.apply_ops(&ops).is_err() {",
         new="        if false && after.apply_ops(&ops).is_err() {",
         cmd=f"{T} autonomy_variants"),
    dict(id=10, name="recover() deixa a Run em running (auto-retomada implícita)",
         file=f"{I}/src/autonomy/orchestrator.rs",
         old="                        r.status = RunStatus::Paused;\n                        r.resume_stage = Some(r.stage);\n                    }\n                    Ok((\n                        (),\n                        vec![(\n                            \"run_interrupted\".into(),",
         new="                        r.resume_stage = Some(r.stage);\n                    }\n                    Ok((\n                        (),\n                        vec![(\n                            \"run_interrupted\".into(),",
         cmd=f"{T} autonomy_crash"),
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
            verdict = "DETECTED" if r.returncode != 0 and built else ("BROKE-BUILD" if not built else "SURVIVED")
        finally:
            path.write_text(original)
            backup.unlink(missing_ok=True)
        results.append((m["id"], m["name"], verdict, round(time.time() - t0)))
        print(f"[{m['id']:>2}] {verdict:<10} {m['name']}  ({round(time.time() - t0)} s)", flush=True)
    detected = sum(1 for r in results if r[2] == "DETECTED")
    print(f"\n{detected}/{len(results)} detected")
    (ROOT / "target").mkdir(exist_ok=True)
    (ROOT / "target" / "mutation-phase5.json").write_text(json.dumps(results, indent=1))
    return 0 if detected == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
