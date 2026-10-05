//! Preferências de privacidade/atualização, persistidas no `AppDb` (namespace `support`).
//! **Crash report: desligado por padrão**; só um ato explícito do usuário liga.

use crate::error::SupportError;
use crate::util::now_ms;
use capia_store::AppDb;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const NAMESPACE: &str = "support";
const KEY: &str = "settings";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SupportSettings {
    /// Opt-in explícito. Padrão `false`.
    pub crash_reporting: bool,
    /// Momento (ms) em que o usuário decidiu; `None` = nunca decidiu.
    pub crash_reporting_decided_ms: Option<u64>,
    /// Canal de atualização: `stable` (padrão) ou `beta`.
    pub update_channel: String,
    /// Onboarding dispensado pelo usuário (nunca forçado).
    pub onboarding_dismissed: bool,
}

impl Default for SupportSettings {
    fn default() -> Self {
        Self {
            crash_reporting: false,
            crash_reporting_decided_ms: None,
            update_channel: "stable".to_owned(),
            onboarding_dismissed: false,
        }
    }
}

/// Acesso às preferências + o *flag* atômico que o gancho de pânico lê (sem tocar no banco durante um pânico).
#[derive(Debug, Clone)]
pub struct SettingsStore {
    db: Arc<AppDb>,
    opt_in: Arc<AtomicBool>,
}

impl SettingsStore {
    pub fn new(db: Arc<AppDb>) -> Result<Self, SupportError> {
        let s = Self {
            db,
            opt_in: Arc::new(AtomicBool::new(false)),
        };
        let cur = s.load()?;
        s.opt_in.store(cur.crash_reporting, Ordering::SeqCst);
        Ok(s)
    }

    /// Flag compartilhado com o gancho de pânico.
    pub fn opt_in_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.opt_in)
    }

    /// Ausente ou ilegível ⇒ **padrões seguros** (crash report desligado).
    pub fn load(&self) -> Result<SupportSettings, SupportError> {
        match self.db.get(NAMESPACE, KEY)? {
            Some(v) => Ok(serde_json::from_value(v).unwrap_or_default()),
            None => Ok(SupportSettings::default()),
        }
    }

    fn save(&self, s: &SupportSettings) -> Result<(), SupportError> {
        let v = serde_json::to_value(s).map_err(|e| SupportError::Invalid(e.to_string()))?;
        self.db.put(NAMESPACE, KEY, &v, now_ms())?;
        self.opt_in.store(s.crash_reporting, Ordering::SeqCst);
        Ok(())
    }

    pub fn set_crash_reporting(&self, enabled: bool) -> Result<SupportSettings, SupportError> {
        let mut s = self.load()?;
        s.crash_reporting = enabled;
        s.crash_reporting_decided_ms = Some(now_ms());
        self.save(&s)?;
        Ok(s)
    }

    pub fn set_update_channel(&self, channel: &str) -> Result<SupportSettings, SupportError> {
        if !matches!(channel, "stable" | "beta") {
            return Err(SupportError::Invalid(format!(
                "unknown update channel `{channel}`"
            )));
        }
        let mut s = self.load()?;
        channel.clone_into(&mut s.update_channel);
        self.save(&s)?;
        Ok(s)
    }

    pub fn dismiss_onboarding(&self) -> Result<SupportSettings, SupportError> {
        let mut s = self.load()?;
        s.onboarding_dismissed = true;
        self.save(&s)?;
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::util::testutil::Tmp;
    use std::time::Duration;

    fn open(t: &Tmp) -> SettingsStore {
        let db = AppDb::open(&t.0.join("app.db"), Duration::from_secs(2)).unwrap();
        SettingsStore::new(Arc::new(db)).unwrap()
    }

    #[test]
    fn crash_reporting_is_off_by_default_and_persists_choices() {
        let t = Tmp::new("set");
        let s = open(&t);
        let d = s.load().unwrap();
        assert!(!d.crash_reporting);
        assert_eq!(d.crash_reporting_decided_ms, None);
        assert_eq!(d.update_channel, "stable");
        assert!(!s.opt_in_flag().load(Ordering::SeqCst));

        s.set_crash_reporting(true).unwrap();
        assert!(s.opt_in_flag().load(Ordering::SeqCst));
        drop(s);
        // reabrir o app: a escolha persiste
        let s = open(&t);
        assert!(s.load().unwrap().crash_reporting);
        assert!(s.opt_in_flag().load(Ordering::SeqCst));
        // o usuário pode desligar
        let off = s.set_crash_reporting(false).unwrap();
        assert!(!off.crash_reporting && off.crash_reporting_decided_ms.is_some());
        assert!(!s.opt_in_flag().load(Ordering::SeqCst));
    }

    #[test]
    fn channel_is_validated_and_corrupt_value_falls_back_to_safe_defaults() {
        let t = Tmp::new("set2");
        let s = open(&t);
        assert!(s.set_update_channel("nightly").is_err());
        assert_eq!(s.set_update_channel("beta").unwrap().update_channel, "beta");
        s.db.put(NAMESPACE, KEY, &serde_json::json!("garbage"), 1)
            .unwrap();
        let d = s.load().unwrap();
        assert!(!d.crash_reporting);
        s.dismiss_onboarding().unwrap();
        assert!(s.load().unwrap().onboarding_dismissed);
    }
}
