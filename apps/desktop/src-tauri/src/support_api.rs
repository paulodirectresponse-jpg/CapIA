//! Métodos `support.*` / `update.*` (Fase 6, Track C) roteados pelo mesmo IPC `editor_call` — a UI os usa só
//! pelo `store/supportController.ts`. Nada aqui toca o documento: são preferências de privacidade, diagnóstico
//! e verificação de atualização. Sem rede: o manifesto vem de um arquivo local (`CAPIA_UPDATE_MANIFEST_FILE`);
//! sem chaves de confiança embutidas na build o resultado é `not_configured` (pendência externa, sem fingir).

use capia_store::AppDb;
use capia_support::{
    BuildInfo, CrashReporter, CrashStore, DiagnosticBuilder, SettingsStore, SupportDirs, now_ms,
};
use capia_updater::{
    Channel, Decision, Ed25519Verifier, FileStateStore, StateStore, Version, check_manifest,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const KEEP_DIAGNOSTICS: usize = 5;

#[derive(Debug)]
pub struct SupportService {
    settings: SettingsStore,
    dirs: SupportDirs,
    build: BuildInfo,
    reporter: Arc<CrashReporter>,
    diagnostics_dir: PathBuf,
    update_dir: PathBuf,
    verifier: Ed25519Verifier,
}

fn err(code: &str, msg: impl std::fmt::Display) -> Value {
    json!({"code": code, "message": capia_secrets::redact_global(&msg.to_string())})
}

impl SupportService {
    pub fn handles(method: &str) -> bool {
        method.starts_with("support.") || method.starts_with("update.")
    }

    /// `app_data`: diretório de dados do usuário (o mesmo do `capia-app.db`). Cria só pastas do app.
    pub fn new(app_data: &Path) -> Result<Self, String> {
        let dirs = SupportDirs::under(app_data);
        dirs.ensure().map_err(|e| e.to_string())?;
        let db = AppDb::open(&app_data.join("capia-app.db"), Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        let settings = SettingsStore::new(Arc::new(db)).map_err(|e| e.to_string())?;
        let build = BuildInfo::current();
        let store = CrashStore::new(&dirs.crash, capia_support::DEFAULT_MAX_RECORDS)
            .map_err(|e| e.to_string())?;
        // sem destino de envio configurado (pendência externa): o opt-in só mantém os registros locais
        let reporter = Arc::new(CrashReporter::new(
            store,
            build.clone(),
            settings.opt_in_flag(),
            None,
        ));
        Ok(Self {
            diagnostics_dir: app_data.join("support").join("diagnostics"),
            update_dir: app_data.join("update"),
            verifier: Ed25519Verifier::embedded(),
            settings,
            dirs,
            build,
            reporter,
        })
    }

    pub fn reporter(&self) -> &Arc<CrashReporter> {
        &self.reporter
    }

    pub fn dirs(&self) -> &SupportDirs {
        &self.dirs
    }

    fn status(&self) -> Result<Value, Value> {
        let s = self.settings.load().map_err(|e| err("SUPPORT", e))?;
        Ok(json!({
            "version": self.build.version,
            "build": {
                "display": self.build.display(),
                "os": self.build.os,
                "arch": self.build.arch,
                "dev_build": self.build.dev_build,
                "build_id": self.build.build_id,
            },
            "crash": {
                "opt_in": s.crash_reporting,
                "decided": s.crash_reporting_decided_ms.is_some(),
                "status": serde_json::to_value(self.reporter.status()).unwrap_or(Value::Null),
            },
            "update_channel": s.update_channel,
            "onboarding_dismissed": s.onboarding_dismissed,
        }))
    }

    fn prune_diagnostics(&self) {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.diagnostics_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "zip"))
            .collect();
        files.sort();
        let excess = files.len().saturating_sub(KEEP_DIAGNOSTICS);
        for f in files.into_iter().take(excess) {
            let _ = std::fs::remove_file(f);
        }
    }

    fn builder(&self) -> Result<DiagnosticBuilder, Value> {
        let s = self.settings.load().map_err(|e| err("SUPPORT", e))?;
        Ok(DiagnosticBuilder::new(
            self.build.clone(),
            self.dirs.clone(),
            s,
        ))
    }

    fn update_check(&self) -> Result<Value, Value> {
        let s = self.settings.load().map_err(|e| err("SUPPORT", e))?;
        let channel = Channel::parse(&s.update_channel).unwrap_or(Channel::Stable);
        let base = json!({"channel": s.update_channel, "current": self.build.version});
        let merge = |extra: Value| -> Value {
            let mut v = base.clone();
            if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            v
        };
        if !self.verifier.is_configured() {
            return Ok(merge(
                json!({"state": "not_configured", "reason": "no_trusted_keys"}),
            ));
        }
        let Some(file) = std::env::var_os("CAPIA_UPDATE_MANIFEST_FILE") else {
            return Ok(merge(
                json!({"state": "not_configured", "reason": "no_endpoint"}),
            ));
        };
        let bytes = std::fs::read(&file).map_err(|e| err("UPDATE_IO", e))?;
        let current = Version::parse(&self.build.version).map_err(|e| err("VERSION", e))?;
        let rejected = FileStateStore::new(&self.update_dir)
            .load()
            .ok()
            .flatten()
            .map(|r| r.rejected_versions)
            .unwrap_or_default();
        Ok(
            match check_manifest(&bytes, &self.verifier, &current, channel, false, &rejected) {
                Err(e) => merge(json!({"state": "invalid", "reason": e.to_string()})),
                Ok((m, Decision::Available(_))) => merge(
                    json!({"state": "available", "version": m.version, "notes": m.notes, "signature": "verified"}),
                ),
                Ok((_, Decision::UpToDate)) => merge(json!({"state": "up_to_date"})),
                Ok((m, Decision::RequiresIntermediate(v))) => merge(
                    json!({"state": "requires_intermediate", "version": m.version, "min_version": v}),
                ),
                Ok((_, Decision::Rejected(r))) => merge(json!({"state": "rejected", "reason": r})),
            },
        )
    }

    pub fn call(&self, method: &str, params: &Value) -> Result<Value, Value> {
        match method {
            "support.status" => self.status(),
            "support.crash.set" => {
                let enabled = params["enabled"]
                    .as_bool()
                    .ok_or_else(|| err("INVALID", "`enabled` must be a boolean"))?;
                self.settings
                    .set_crash_reporting(enabled)
                    .map_err(|e| err("SUPPORT", e))?;
                self.status()
            }
            "support.channel.set" => {
                let ch = params["channel"].as_str().unwrap_or_default();
                self.settings
                    .set_update_channel(ch)
                    .map_err(|e| err("INVALID", e))?;
                self.status()
            }
            "support.onboarding.dismiss" => {
                self.settings
                    .dismiss_onboarding()
                    .map_err(|e| err("SUPPORT", e))?;
                self.status()
            }
            "support.diagnostic.preview" => {
                Ok(serde_json::to_value(self.builder()?.preview()).unwrap_or(Value::Null))
            }
            "support.diagnostic.create" => {
                std::fs::create_dir_all(&self.diagnostics_dir).map_err(|e| err("SUPPORT_IO", e))?;
                let path = self
                    .diagnostics_dir
                    .join(format!("capia-diagnostic-{}.zip", now_ms()));
                let written = self
                    .builder()?
                    .write_zip(&path)
                    .map_err(|e| err("SUPPORT_IO", e))?;
                self.prune_diagnostics();
                Ok(json!({
                    "path": capia_support::scrub_paths(&path.display().to_string()),
                    "bytes": std::fs::metadata(&path).map_or(0, |m| m.len()),
                    "entries": written.entries,
                }))
            }
            "update.status" => {
                let rec = FileStateStore::new(&self.update_dir).load().ok().flatten();
                Ok(json!({
                    "configured": self.verifier.is_configured(),
                    "state": rec.as_ref().map_or_else(|| "Idle".to_owned(), |r| format!("{:?}", r.state)),
                    "last_outcome": rec.and_then(|r| r.last_outcome).map(|o| format!("{o:?}")),
                }))
            }
            "update.check" => self.update_check(),
            other => Err(err("UNKNOWN_METHOD", format!("unknown method `{other}`"))),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn svc(tag: &str) -> (SupportService, PathBuf) {
        let d = std::env::temp_dir().join(format!("capia-supsvc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        (SupportService::new(&d).unwrap(), d)
    }

    #[test]
    fn privacy_defaults_and_toggles() {
        let (s, d) = svc("a");
        let st = s.call("support.status", &json!({})).unwrap();
        assert_eq!(st["crash"]["opt_in"], false);
        assert_eq!(st["update_channel"], "stable");
        assert_eq!(st["version"], env!("CARGO_PKG_VERSION"));
        let on = s
            .call("support.crash.set", &json!({"enabled": true}))
            .unwrap();
        assert_eq!(on["crash"]["opt_in"], true);
        assert!(
            s.call("support.crash.set", &json!({"enabled": "yes"}))
                .is_err()
        );
        assert!(
            s.call("support.channel.set", &json!({"channel": "nightly"}))
                .is_err()
        );
        let b = s
            .call("support.channel.set", &json!({"channel": "beta"}))
            .unwrap();
        assert_eq!(b["update_channel"], "beta");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn diagnostics_preview_then_create_match() {
        let (s, d) = svc("b");
        let pv = s.call("support.diagnostic.preview", &json!({})).unwrap();
        let made = s.call("support.diagnostic.create", &json!({})).unwrap();
        assert!(pv["entries"].as_array().unwrap().len() >= 4);
        assert_eq!(
            made["entries"].as_array().unwrap().len(),
            pv["entries"].as_array().unwrap().len()
        );
        assert!(made["bytes"].as_u64().unwrap() > 0);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn update_check_is_not_configured_without_keys_and_unknown_methods_fail() {
        let (s, d) = svc("c");
        let r = s.call("update.check", &json!({})).unwrap();
        assert_eq!(r["state"], "not_configured");
        assert_eq!(
            s.call("update.status", &json!({})).unwrap()["state"],
            "Idle"
        );
        assert_eq!(
            s.call("support.shell", &json!({})).unwrap_err()["code"],
            "UNKNOWN_METHOD"
        );
        assert!(SupportService::handles("update.check") && !SupportService::handles("ai.status"));
        let _ = std::fs::remove_dir_all(d);
    }
}
