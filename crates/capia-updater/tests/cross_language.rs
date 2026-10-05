//! Paridade entre linguagens: um manifesto assinado por `tools/release/make-update-manifest.mjs` (Node,
//! chave de TESTE descartável; só a pública está versionada) precisa verificar no Rust — prova que o JSON
//! canônico e o Ed25519 são idênticos nos dois lados.
#![allow(clippy::unwrap_used)]

use capia_updater::{Channel, Decision, Ed25519Verifier, TrustedKey, UpdateManifest, check_manifest};
use serde_json::Value;

const MANIFEST: &str = include_str!("fixtures/node_signed_manifest.json");
const PUBKEY: &str = include_str!("fixtures/node_signed_manifest.json.test-pubkey.json");

fn verifier() -> Ed25519Verifier {
    let p: Value = serde_json::from_str(PUBKEY).unwrap();
    Ed25519Verifier::new(vec![
        TrustedKey::from_hex(
            p["key_id"].as_str().unwrap(),
            p["public_hex"].as_str().unwrap(),
        )
        .unwrap(),
    ])
}

#[test]
fn a_manifest_signed_by_the_node_tool_verifies_in_rust() {
    let m = UpdateManifest::parse_and_verify(MANIFEST.as_bytes(), &verifier()).unwrap();
    assert_eq!(m.version, "0.7.0");
    assert!(m.notes.contains("acentuação"), "non-ASCII must survive canonicalization");
    assert_eq!(m.min_version.as_deref(), Some("0.6.0-rc.1"));
}

#[test]
fn the_fixture_is_rejected_when_changed_by_even_one_byte() {
    let tampered = MANIFEST.replacen("0.7.0", "0.7.1", 1);
    assert!(UpdateManifest::parse_and_verify(tampered.as_bytes(), &verifier()).is_err());
}

#[test]
fn policy_applies_on_top_of_the_verified_fixture() {
    let cur = |v: &str| semver_parse(v);
    let ok = check_manifest(
        MANIFEST.as_bytes(),
        &verifier(),
        &cur("0.6.0-rc.1"),
        Channel::Stable,
        false,
        &[],
    )
    .unwrap();
    assert!(matches!(ok.1, Decision::Available(_)));
    let beta = check_manifest(
        MANIFEST.as_bytes(),
        &verifier(),
        &cur("0.6.0-rc.1"),
        Channel::Beta,
        false,
        &[],
    )
    .unwrap();
    assert!(matches!(beta.1, Decision::Rejected(_)), "channel mismatch");
    let down = check_manifest(
        MANIFEST.as_bytes(),
        &verifier(),
        &cur("0.9.0"),
        Channel::Stable,
        true,
        &[],
    )
    .unwrap();
    assert!(matches!(down.1, Decision::Rejected(_)), "no silent downgrade");
}

fn semver_parse(v: &str) -> capia_updater::Version {
    capia_updater::Version::parse(v).unwrap()
}
