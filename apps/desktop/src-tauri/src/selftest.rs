//! `capia-desktop --self-test`: verificação headless do produto **instalado** (sem WebView, sem rede, sem IA).
//!
//! Cria um projeto num diretório temporário, importa uma mídia gerada aqui (WAV senoidal minúsculo, passa pelo
//! ffprobe empacotado), aplica uma edição pelo Command Engine (inclui undo/redo), exporta e confere o suporte
//! (preferências de privacidade padrão, diagnóstico sem segredo). Devolve um relatório JSON; saída 0 = tudo ok.
//! Usado pelo smoke test do instalador (`tools/release/installer-smoke.ps1`).

use capia_editor_api::{Reply, Session, SessionConfig};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const FRAME: i64 = 23_520_000; // 1/30 s em ticks (705.600.000/s)

#[derive(Clone, Debug, Default)]
pub struct SelfTestOptions {
    /// Falha se o FFmpeg/ffprobe empacotado não for encontrado (o instalador exige).
    pub require_media: bool,
}

struct Runner {
    steps: Vec<Value>,
    ok: bool,
}

impl Runner {
    fn step(&mut self, name: &str, f: impl FnOnce() -> Result<Value, String>) -> Option<Value> {
        let t = Instant::now();
        let r = f();
        let ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
        match r {
            Ok(detail) => {
                self.steps
                    .push(json!({"name": name, "ok": true, "ms": ms, "detail": detail}));
                Some(detail)
            }
            Err(e) => {
                self.ok = false;
                self.steps.push(
                    json!({"name": name, "ok": false, "ms": ms, "error": capia_secrets::redact_global(&e)}),
                );
                None
            }
        }
    }
}

fn json_of(r: Reply) -> Result<Value, String> {
    match r {
        Reply::Json(v) => Ok(v),
        Reply::Binary { .. } => Err("unexpected binary reply".into()),
    }
}

fn call(s: &mut Session, m: &str, p: Value) -> Result<Value, String> {
    s.call(m, p)
        .map_err(|e| format!("{m}: {e}"))
        .and_then(json_of)
}

fn poll_until(
    s: &mut Session,
    what: &str,
    mut pred: impl FnMut(&Value) -> bool,
) -> Result<Vec<Value>, String> {
    let start = Instant::now();
    let mut all = Vec::new();
    while start.elapsed() < Duration::from_secs(60) {
        let ev = call(s, "events.poll", json!({}))?;
        if let Some(list) = ev["events"].as_array() {
            all.extend(list.iter().cloned());
        }
        if all.iter().any(&mut pred) {
            return Ok(all);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!("timeout waiting for {what}"))
}

/// WAV PCM16 mono, 8 kHz, ~0,5 s de seno de 440 Hz (cabeçalho canônico de 44 bytes).
pub fn tiny_wav() -> Vec<u8> {
    let rate: u32 = 8000;
    let samples: u32 = rate / 2;
    let mut data = Vec::with_capacity(samples as usize * 2);
    for n in 0..samples {
        let t = f64::from(n) / f64::from(rate);
        let v = (t * 440.0 * std::f64::consts::TAU).sin() * 8000.0;
        #[allow(clippy::cast_possible_truncation)]
        data.extend_from_slice(&(v as i16).to_le_bytes());
    }
    let len = u32::try_from(data.len()).unwrap_or(0);
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1_u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&data);
    out
}

fn temp_dir() -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let d = std::env::temp_dir().join(format!("capia-selftest-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

fn support_checks(dir: &Path) -> Result<Value, String> {
    use capia_support::{BuildInfo, DiagnosticBuilder, SettingsStore, SupportDirs};
    use std::sync::Arc;
    let db = capia_store::AppDb::open(&dir.join("app.db"), Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    let settings = SettingsStore::new(Arc::new(db)).map_err(|e| e.to_string())?;
    let s = settings.load().map_err(|e| e.to_string())?;
    if s.crash_reporting {
        return Err("crash reporting must be OFF by default".into());
    }
    const CANARY: &str = "CNRY-selftest-6c1f0d9a2b7e4835";
    capia_secrets::register_global(CANARY);
    let dirs = SupportDirs::under(dir);
    dirs.ensure().map_err(|e| e.to_string())?;
    std::fs::write(dirs.logs.join("raw.log"), format!("secret {CANARY}\n"))
        .map_err(|e| e.to_string())?;
    let b = DiagnosticBuilder::new(BuildInfo::current(), dirs, s);
    let out = dir.join("diag");
    let pv = b.write_dir(&out).map_err(|e| e.to_string())?;
    let mut stack = vec![out];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if String::from_utf8_lossy(&std::fs::read(&p).map_err(|e| e.to_string())?)
                .contains(CANARY)
            {
                return Err("diagnostic bundle leaked a registered secret".into());
            }
        }
    }
    Ok(json!({"crash_reporting_default": false, "diagnostic_files": pv.entries.len()}))
}

/// Executa o self-test. Devolve `(ok, relatório)`.
pub fn run(opts: &SelfTestOptions) -> (bool, Value) {
    let started = Instant::now();
    let mut r = Runner {
        steps: Vec::new(),
        ok: true,
    };
    let build = capia_support::BuildInfo::current();
    let dir = match temp_dir() {
        Ok(d) => d,
        Err(e) => {
            return (
                false,
                json!({"ok": false, "version": build.version, "error": e}),
            );
        }
    };
    let mut s = Session::new(SessionConfig::default());

    let info = r.step("engine.info", || {
        let v = call(&mut s, "engine.info", json!({}))?;
        if v["version"] != build.version.as_str() {
            return Err(format!(
                "engine version {} != app version {}",
                v["version"], build.version
            ));
        }
        Ok(json!({"engine": v["version"], "media_available": v["media_available"], "media_version": v["media_version"]}))
    });
    let media = info
        .as_ref()
        .and_then(|i| i["media_available"].as_bool())
        .unwrap_or(false);
    if opts.require_media {
        r.step("media.bundled", || {
            if media {
                Ok(json!({"available": true}))
            } else {
                let why = info
                    .as_ref()
                    .and_then(|i| i["media_error"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "unknown".into());
                Err(format!(
                    "FFmpeg/ffprobe unusable next to the executable (<install>/ffmpeg): {why}"
                ))
            }
        });
    }

    let project = dir.join("selftest.capia");
    r.step("project.create", || {
        call(
            &mut s,
            "project.create",
            json!({"path": project.display().to_string()}),
        )
        .map(|_| json!({}))
    });

    r.step("edit.command_engine", || {
        call(
            &mut s,
            "command.execute",
            json!({"label": "self-test", "commands": [
                {"operation_id":"st1","type":"create_sequence","id":"s","name":"SelfTest","frame_rate":"30","width":320,"height":180},
                {"operation_id":"st2","type":"add_track","sequence":"s","id":"v","kind":"visual"},
                {"operation_id":"st3","type":"insert_clip","track":"v","start":0,
                 "clip":{"id":"c1","duration":FRAME*3,"content":{"type":"solid","color":"#336699"}}}
            ]}),
        )?;
        let snap = call(&mut s, "project.snapshot", json!({}))?;
        if snap["sequences"][0]["clip_count"] != 1 {
            return Err("clip was not inserted".into());
        }
        call(&mut s, "command.undo", json!({}))?;
        let after_undo = call(&mut s, "project.snapshot", json!({}))?;
        call(&mut s, "command.redo", json!({}))?;
        Ok(json!({"clip_count_after_undo": after_undo["sequences"][0]["clip_count"]}))
    });

    if media {
        r.step("assets.import", || {
            let wav = dir.join("tone.wav");
            std::fs::write(&wav, tiny_wav()).map_err(|e| e.to_string())?;
            let imp = call(
                &mut s,
                "assets.import",
                json!({"paths": [wav.display().to_string()]}),
            )?;
            if imp["tickets"].as_array().is_none_or(Vec::is_empty) {
                return Err(format!("import was not accepted: {imp}"));
            }
            let evs = poll_until(&mut s, "import", |e| e["kind"] == "import_finalized")?;
            let fin = evs
                .iter()
                .find(|e| e["kind"] == "import_finalized")
                .ok_or("no import_finalized")?;
            Ok(json!({"asset_id": fin["result"]["asset_id"]}))
        });

        let out = dir.join("out.rgba-dir");
        r.step("export.intermediate", || {
            call(
                &mut s,
                "export.start",
                json!({"items":[
                    {"id":"i1","sequence":"s","preset":"intermediate","path":out.display().to_string()}
                ]}),
            )?;
            let evs = poll_until(&mut s, "export", |e| e["kind"] == "export_batch_finished")?;
            let fin = evs
                .iter()
                .find(|e| e["kind"] == "export_item_finished")
                .ok_or("no export_item_finished")?;
            if fin["ok"] != true {
                return Err(format!("export failed: {fin}"));
            }
            if !out.exists() {
                return Err("export output was not published".into());
            }
            Ok(json!({"published": true}))
        });
    }

    r.step("support.privacy_and_diagnostics", || {
        support_checks(&dir.join("support-check"))
    });

    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
    let total = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let report = json!({
        "ok": r.ok,
        "version": build.version,
        "os": build.os,
        "arch": build.arch,
        "dev_build": build.dev_build,
        "media_available": media,
        "require_media": opts.require_media,
        "steps": r.steps,
        "total_ms": total,
    });
    (r.ok, report)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn wav_has_a_valid_canonical_header() {
        let w = tiny_wav();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        let data_len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(w.len(), 44 + data_len);
        assert_eq!(
            u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize,
            36 + data_len
        );
    }

    #[test]
    fn self_test_runs_without_media_and_reports_steps() {
        let (ok, report) = run(&SelfTestOptions {
            require_media: false,
        });
        assert!(report["steps"].as_array().unwrap().len() >= 4, "{report}");
        // sem FFmpeg (ambiente sem mídia) o relatório continua válido; com FFmpeg tudo precisa passar
        if report["media_available"] == true {
            assert!(ok, "{report}");
        } else {
            let failed: Vec<_> = report["steps"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| s["ok"] == false)
                .collect();
            assert!(failed.is_empty(), "{failed:?}");
        }
    }

    #[test]
    fn require_media_fails_when_the_toolchain_is_missing() {
        let (_, report) = run(&SelfTestOptions {
            require_media: true,
        });
        if report["media_available"] == false {
            assert_eq!(report["ok"], false);
        }
    }
}
