//! Semver (com pré-release) e a política de aceitação de um update.

use crate::error::UpdateError;
use crate::manifest::{Channel, UpdateManifest};
pub use semver::Version;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub fn parse(s: &str) -> Result<Version, UpdateError> {
    Version::parse(s).map_err(|e| UpdateError::Invalid(format!("`{s}` is not semver: {e}")))
}

/// Precedência semver (ignora metadados de build): `0.6.0-rc.1 < 0.6.0-rc.2 < 0.6.0`.
pub fn compare(a: &Version, b: &Version) -> Ordering {
    a.cmp_precedence(b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateKind {
    Upgrade,
    /// Rollback deliberado (manifesto marcado + pedido explícito do usuário).
    Rollback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    UpToDate,
    Available(UpdateKind),
    /// A versão instalada é menor que `min_version`: instalar antes a versão intermediária.
    RequiresIntermediate(String),
    Rejected(String),
}

/// Decide se `manifest` deve ser aplicado sobre `current`.
///
/// * canal diferente do selecionado ⇒ recusa; canal `stable` nunca recebe pré-release;
/// * **sem downgrade silencioso**: versão menor só com `manifest.rollback == true` **e** `allow_rollback`
///   (ato explícito do usuário na UI/CLI);
/// * versões já revertidas por falha de saúde (`rejected_versions`) não voltam sozinhas.
pub fn evaluate(
    current: &Version,
    selected: Channel,
    manifest: &UpdateManifest,
    allow_rollback: bool,
    rejected_versions: &[String],
) -> Result<Decision, UpdateError> {
    let target = parse(&manifest.version)?;
    if manifest.channel != selected {
        return Ok(Decision::Rejected(format!(
            "manifest channel `{}` does not match selected channel `{}`",
            manifest.channel.as_str(),
            selected.as_str()
        )));
    }
    if selected == Channel::Stable && !target.pre.is_empty() {
        return Ok(Decision::Rejected(
            "pre-release versions are not offered on the stable channel".into(),
        ));
    }
    if rejected_versions.contains(&manifest.version) {
        return Ok(Decision::Rejected(format!(
            "version {} failed its health check earlier and was rolled back",
            manifest.version
        )));
    }
    Ok(match compare(&target, current) {
        Ordering::Equal => Decision::UpToDate,
        Ordering::Less => {
            if manifest.rollback && allow_rollback {
                Decision::Available(UpdateKind::Rollback)
            } else if manifest.rollback {
                Decision::Rejected("rollback manifest requires an explicit user request".into())
            } else {
                Decision::Rejected(format!(
                    "downgrade from {current} to {target} is not allowed without a rollback manifest"
                ))
            }
        }
        Ordering::Greater => {
            if manifest.rollback {
                Decision::Rejected("a rollback manifest cannot move to a newer version".into())
            } else if let Some(min) = &manifest.min_version
                && compare(current, &parse(min)?) == Ordering::Less
            {
                Decision::RequiresIntermediate(min.clone())
            } else {
                Decision::Available(UpdateKind::Upgrade)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::manifest::fixtures::sample;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn decide(cur: &str, m: &UpdateManifest, ch: Channel, rb: bool) -> Decision {
        evaluate(&parse(cur).unwrap(), ch, m, rb, &[]).unwrap()
    }

    #[test]
    fn semver_precedence_with_prerelease() {
        let v = |s: &str| parse(s).unwrap();
        assert_eq!(compare(&v("0.6.0-rc.1"), &v("0.6.0-rc.2")), Ordering::Less);
        assert_eq!(compare(&v("0.6.0-rc.2"), &v("0.6.0")), Ordering::Less);
        assert_eq!(
            compare(&v("0.6.0-rc.10"), &v("0.6.0-rc.9")),
            Ordering::Greater
        );
        assert_eq!(compare(&v("1.0.0+a"), &v("1.0.0+b")), Ordering::Equal);
        assert_eq!(compare(&v("0.10.0"), &v("0.9.0")), Ordering::Greater);
    }

    #[test]
    fn upgrade_same_and_downgrade() {
        let up = sample("0.7.0", SHA, 1);
        assert_eq!(
            decide("0.6.0-rc.1", &up, Channel::Stable, false),
            Decision::Available(UpdateKind::Upgrade)
        );
        assert_eq!(
            decide("0.7.0", &up, Channel::Stable, false),
            Decision::UpToDate
        );
        assert!(
            matches!(decide("0.8.0", &up, Channel::Stable, false), Decision::Rejected(m) if m.contains("downgrade"))
        );
        // pedir rollback explícito NÃO libera downgrade se o manifesto não é de rollback
        assert!(matches!(
            decide("0.8.0", &up, Channel::Stable, true),
            Decision::Rejected(_)
        ));
    }

    #[test]
    fn rollback_needs_both_manifest_flag_and_explicit_request() {
        let mut rb = sample("0.6.0", SHA, 1);
        rb.rollback = true;
        assert!(matches!(
            decide("0.7.0", &rb, Channel::Stable, false),
            Decision::Rejected(_)
        ));
        assert_eq!(
            decide("0.7.0", &rb, Channel::Stable, true),
            Decision::Available(UpdateKind::Rollback)
        );
        // rollback "para frente" é recusado
        assert!(matches!(
            decide("0.5.0", &rb, Channel::Stable, true),
            Decision::Rejected(_)
        ));
    }

    #[test]
    fn channels_and_prerelease_policy() {
        let mut beta = sample("0.7.0-beta.1", SHA, 1);
        beta.channel = Channel::Beta;
        assert!(matches!(
            decide("0.6.0", &beta, Channel::Stable, false),
            Decision::Rejected(_)
        ));
        assert_eq!(
            decide("0.6.0", &beta, Channel::Beta, false),
            Decision::Available(UpdateKind::Upgrade)
        );
        let pre_on_stable = sample("0.7.0-rc.1", SHA, 1);
        assert!(matches!(
            decide("0.6.0", &pre_on_stable, Channel::Stable, false),
            Decision::Rejected(_)
        ));
    }

    #[test]
    fn min_version_requires_intermediate_and_rejected_versions_do_not_return() {
        let mut m = sample("1.0.0", SHA, 1);
        m.min_version = Some("0.9.0".into());
        assert_eq!(
            decide("0.8.0", &m, Channel::Stable, false),
            Decision::RequiresIntermediate("0.9.0".into())
        );
        assert_eq!(
            decide("0.9.0", &m, Channel::Stable, false),
            Decision::Available(UpdateKind::Upgrade)
        );
        let d = evaluate(
            &parse("0.9.0").unwrap(),
            Channel::Stable,
            &m,
            false,
            &["1.0.0".to_owned()],
        )
        .unwrap();
        assert!(matches!(d, Decision::Rejected(_)));
    }
}
