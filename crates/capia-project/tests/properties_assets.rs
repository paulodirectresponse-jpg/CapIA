//! Propriedade com assets (ADR-046..048): sequências aleatórias de criar/apagar/mover arquivos,
//! importar, relink, verify, clips, delete_asset, undo/redo, comandos de composição e REABRIR o
//! projeto. Um modelo de referência independente (arquivos × conteúdos) prevê cada resultado do
//! catálogo; a cada reabertura documento e catálogo precisam ser idênticos.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_assets::testing::StaticProbe;
use capia_assets::{Availability, hash_reader};
use capia_commands::{Actor, Command, CommandEnvelope, NewClip, Transaction};
use capia_model::{AssetId, ClipContent, ErrorCode};
use capia_project::{ImportOutcome, Project, ProjectError, expected_asset_id};
use capia_store::{StoreOptions, Synchronous};
use capia_time::{FrameRate, Rational, Ticks};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FRAME: i64 = 23_520_000;

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

/// Seis conteúdos; os pares (0,1), (2,3), (4,5) têm o MESMO tamanho (só o hash os distingue).
fn content(i: usize) -> Vec<u8> {
    let len = 20 + 10 * (i / 2);
    let mut v = vec![b'a' + u8::try_from(i).unwrap(); len];
    v[0] = b'#';
    v
}

fn hash_of(i: usize) -> String {
    hash_reader(&content(i)[..]).unwrap().hash.to_string()
}

struct Cat {
    loc: PathBuf,
    known: Vec<PathBuf>,
}

fn alias(known: &mut Vec<PathBuf>, p: &Path) {
    if !known.iter().any(|k| k == p) {
        known.push(p.to_path_buf());
        if known.len() > 64 {
            known.remove(0);
        }
    }
}

struct Model {
    files: BTreeMap<PathBuf, usize>,
    /// índice de conteúdo → entrada do catálogo
    cat: BTreeMap<usize, Cat>,
}

impl Model {
    fn id(c: usize) -> AssetId {
        expected_asset_id(&capia_assets::ContentHash::parse(&hash_of(c)).unwrap())
    }

    /// Candidatos existentes, em ordem (principal + aliases), com o conteúdo que têm.
    fn existing(&self, c: usize) -> Vec<usize> {
        let e = &self.cat[&c];
        std::iter::once(&e.loc)
            .chain(e.known.iter())
            .filter_map(|p| self.files.get(p).copied())
            .collect()
    }

    /// Status que o projeto deve informar (checagem barata): algum candidato com o tamanho
    /// conhecido ⇒ online; senão modificado; nenhum ⇒ offline.
    fn quick(&self, c: usize) -> Availability {
        let ex = self.existing(c);
        if ex.iter().any(|&f| content(f).len() == content(c).len()) {
            Availability::Online
        } else if ex.is_empty() {
            Availability::Offline
        } else {
            Availability::Modified
        }
    }

    /// Status do verify completo: algum candidato com o conteúdo exato ⇒ online.
    fn full(&self, c: usize) -> Availability {
        let ex = self.existing(c);
        if ex.contains(&c) {
            Availability::Online
        } else if ex.is_empty() {
            Availability::Offline
        } else {
            Availability::Modified
        }
    }
}

fn opts() -> StoreOptions {
    StoreOptions {
        synchronous: Synchronous::Off,
        ..StoreOptions::default()
    }
}

fn tx(label: &str, op: &str, command: Command) -> Transaction {
    Transaction {
        transaction_id: None,
        label: label.into(),
        base_revision: None,
        commands: vec![CommandEnvelope {
            operation_id: op.into(),
            reference: None,
            command,
        }],
        max_ops: None,
    }
}

fn sem(p: &Project) -> String {
    let mut d = p.document().clone();
    d.revision = 0;
    capia_commands::document_digest(&d)
}

/// Visão do catálogo estável entre execuções (sem relógio).
fn catalog_json(p: &Project) -> Value {
    let mut v = serde_json::to_value(p.assets().unwrap()).unwrap();
    for a in v.as_array_mut().unwrap() {
        if let Some(c) = a.get_mut("catalog").and_then(Value::as_object_mut) {
            c.remove("status_checked_ms");
        }
    }
    v
}

type Tally = BTreeMap<&'static str, u64>;

fn bump(t: &mut Tally, k: &'static str) {
    *t.entry(k).or_default() += 1;
}

fn run_case(seed: u64, steps: usize, tally: &mut Tally) {
    let mut rng = Rng::new(seed ^ 0x51ED);
    let dir = std::env::temp_dir().join(format!("capia-propassets-{}-{seed}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("media/sub")).unwrap();
    let proj_path = dir.join("p.capia");
    let user = Actor::user("prop");
    let mut p = Project::create(&proj_path, &opts()).unwrap();
    let mut m = Model {
        files: BTreeMap::new(),
        cat: BTreeMap::new(),
    };
    p.execute(
        &user,
        Transaction {
            transaction_id: None,
            label: "setup".into(),
            base_revision: None,
            commands: vec![
                CommandEnvelope {
                    operation_id: "seq".into(),
                    reference: None,
                    command: Command::CreateSequence {
                        id: Some("S".into()),
                        name: "S".into(),
                        frame_rate: FrameRate::FPS_30,
                        sample_rate: None,
                    },
                },
                CommandEnvelope {
                    operation_id: "trk".into(),
                    reference: None,
                    command: Command::AddTrack {
                        sequence: "S".into(),
                        id: Some("V1".into()),
                        kind: capia_model::TrackKind::Visual,
                        name: None,
                        role: None,
                        magnetic: false,
                        index: None,
                    },
                },
            ],
            max_ops: None,
        },
    )
    .unwrap();
    let paths: Vec<PathBuf> = (0..6)
        .map(|i| {
            if i % 2 == 0 {
                dir.join(format!("media/f{i}.mp4"))
            } else {
                dir.join(format!("media/sub/f{i}.mp4"))
            }
        })
        .collect();
    let mut n = 0u32;
    for step in 0..steps {
        n += 1;
        let ctx = format!("seed {seed} step {step}");
        match rng.below(100) {
            // escreve/sobrescreve um arquivo de mídia
            0..=17 => {
                let (pi, c) = (rng.below(6) as usize, rng.below(6) as usize);
                std::fs::write(&paths[pi], content(c)).unwrap();
                m.files.insert(paths[pi].clone(), c);
            }
            // remove ou move um arquivo
            18..=27 => {
                let pi = rng.below(6) as usize;
                if m.files.contains_key(&paths[pi]) {
                    if rng.chance(50) {
                        std::fs::remove_file(&paths[pi]).unwrap();
                        m.files.remove(&paths[pi]);
                    } else {
                        let to = &paths[(pi + 1) % 6];
                        if !m.files.contains_key(to) {
                            std::fs::rename(&paths[pi], to).unwrap();
                            let c = m.files.remove(&paths[pi]).unwrap();
                            m.files.insert(to.clone(), c);
                        }
                    }
                }
            }
            // import
            28..=51 => {
                let pi = rng.below(6) as usize;
                let path = &paths[pi];
                let result = p.import_asset(&user, path, &StaticProbe);
                let Some(&c) = m.files.get(path) else {
                    assert!(result.is_err(), "{ctx}: importing a missing file must fail");
                    continue;
                };
                let r = result.unwrap_or_else(|e| panic!("{ctx}: import failed: {e}"));
                assert_eq!(
                    r.asset_id,
                    Model::id(c),
                    "{ctx}: asset id is the content identity"
                );
                let in_doc_before = r.commit.is_none();
                let expected = match m.cat.get(&c) {
                    None => ImportOutcome::Created,
                    Some(_)
                        if !p
                            .document()
                            .asset(&r.asset_id)
                            .is_some_and(|_| in_doc_before)
                            && r.commit.is_some() =>
                    {
                        ImportOutcome::Reregistered
                    }
                    Some(_) => ImportOutcome::Existing, // refinado abaixo
                };
                match (m.cat.get_mut(&c), r.outcome) {
                    (None, o) => {
                        bump(tally, "import:created");
                        assert_eq!(o, ImportOutcome::Created, "{ctx}");
                        m.cat.insert(
                            c,
                            Cat {
                                loc: path.clone(),
                                known: Vec::new(),
                            },
                        );
                    }
                    (Some(e), ImportOutcome::Reregistered) => {
                        bump(tally, "import:reregistered");
                        assert_eq!(expected, ImportOutcome::Reregistered, "{ctx}");
                        let old = e.loc.clone();
                        alias(&mut e.known, &old);
                        e.loc = path.clone();
                    }
                    (Some(e), ImportOutcome::Existing) => {
                        bump(tally, "import:existing");
                        assert!(
                            e.loc == *path || e.known.contains(path),
                            "{ctx}: Existing only for known paths"
                        );
                    }
                    (Some(e), ImportOutcome::Aliased) => {
                        bump(tally, "import:aliased");
                        assert!(e.loc != *path && !e.known.contains(path), "{ctx}");
                        alias(&mut e.known, path);
                    }
                    (Some(e), ImportOutcome::Relinked) => {
                        bump(tally, "import:auto_relinked");
                        let old = e.loc.clone();
                        alias(&mut e.known, &old);
                        e.loc = path.clone();
                    }
                    (Some(_), other) => panic!("{ctx}: unexpected outcome {other:?}"),
                }
            }
            // relink
            52..=61 => {
                let known: Vec<usize> = m.cat.keys().copied().collect();
                if known.is_empty() {
                    continue;
                }
                let c = known[rng.below(known.len() as u64) as usize];
                let pi = rng.below(6) as usize;
                let id = Model::id(c);
                let r = p.relink_asset(&id, &paths[pi]);
                match m.files.get(&paths[pi]) {
                    None => assert!(r.is_err(), "{ctx}: relink to a missing file must fail"),
                    Some(&found) if found != c => {
                        let e = r.unwrap_err();
                        bump(tally, "relink:rejected");
                        assert_eq!(e.code(), "ASSET_HASH_MISMATCH", "{ctx}");
                    }
                    Some(_) => {
                        bump(tally, "relink:ok");
                        r.unwrap_or_else(|e| panic!("{ctx}: relink failed: {e}"));
                        let e = m.cat.get_mut(&c).unwrap();
                        let old = e.loc.clone();
                        alias(&mut e.known, &old);
                        e.loc = paths[pi].clone();
                    }
                }
            }
            // verify
            62..=69 => {
                let known: Vec<usize> = m.cat.keys().copied().collect();
                if known.is_empty() {
                    continue;
                }
                let c = known[rng.below(known.len() as u64) as usize];
                let r = p.verify_asset(&Model::id(c)).unwrap();
                assert_eq!(r.status, m.full(c), "{ctx}: verify status");
                bump(
                    tally,
                    match r.status {
                        Availability::Online => "verify:online",
                        Availability::Offline => "verify:offline",
                        Availability::Modified => "verify:modified",
                    },
                );
            }
            // clip sobre um asset do documento
            70..=79 => {
                let ids: Vec<AssetId> = p.document().assets().map(|a| a.id.clone()).collect();
                // o undo pode ter desfeito o setup: só coloca clip se a track ainda existe
                let has_track = p.document().find_track(&"V1".into()).is_some();
                if ids.is_empty() || !has_track {
                    continue;
                }
                let asset = ids[rng.below(ids.len() as u64) as usize].clone();
                let r = p.execute(
                    &user,
                    tx(
                        "clip",
                        &format!("clip-{n}"),
                        Command::InsertClip {
                            track: "V1".into(),
                            start: Ticks(i64::from(n) * 40 * FRAME),
                            clip: NewClip {
                                id: Some(format!("c{n}").as_str().into()),
                                name: String::new(),
                                duration: Ticks(10 * FRAME),
                                content: ClipContent::Media {
                                    asset,
                                    has_video: true,
                                    has_audio: false,
                                },
                                source_in: Ticks::ZERO,
                                speed: Rational::ONE,
                                reversed: false,
                                properties: Default::default(),
                            },
                            split_at_insert: false,
                            split_new_id: None,
                        },
                    ),
                );
                assert!(
                    r.is_ok(),
                    "{ctx}: placing a clip on a registered asset: {r:?}"
                );
                bump(tally, "clip:placed");
            }
            // delete_asset: IN_USE exatamente quando algum clip o referencia
            80..=84 => {
                let ids: Vec<AssetId> = p.document().assets().map(|a| a.id.clone()).collect();
                if ids.is_empty() {
                    continue;
                }
                let asset = ids[rng.below(ids.len() as u64) as usize].clone();
                let used = p
                    .document()
                    .sequences()
                    .any(|(_, s)| s.clips().any(|c| c.content.asset() == Some(&asset)));
                let r = p.execute(
                    &user,
                    tx(
                        "rm asset",
                        &format!("rma-{n}"),
                        Command::DeleteAsset { asset },
                    ),
                );
                match (used, r) {
                    (true, Err(e)) => {
                        bump(tally, "delete_asset:in_use");
                        assert_eq!(e.code, ErrorCode::InUse, "{ctx}");
                    }
                    (false, Ok(_)) => bump(tally, "delete_asset:ok"),
                    (u, r) => panic!("{ctx}: delete_asset used={u} → {r:?}"),
                }
            }
            // delete de clip (libera assets)
            85..=87 => {
                let clips: Vec<String> = p
                    .document()
                    .sequences()
                    .flat_map(|(_, s)| s.clips().map(|c| c.id.0.clone()).collect::<Vec<_>>())
                    .collect();
                if let Some(id) = clips.get(rng.below(clips.len().max(1) as u64) as usize) {
                    let _ = p.execute(
                        &user,
                        tx(
                            "rm clip",
                            &format!("rmc-{n}"),
                            Command::DeleteClip {
                                clip: id.as_str().into(),
                                ripple: None,
                                scope: Default::default(),
                            },
                        ),
                    );
                }
            }
            // composição: duplicar S / agrupar um clip / achatar um nested
            88..=92 => {
                let clips: Vec<String> = p
                    .document()
                    .sequence(&"S".into())
                    .map(|s| s.clips().map(|c| c.id.0.clone()).collect())
                    .unwrap_or_default();
                let cmd = match rng.below(3) {
                    0 => Command::DuplicateSequence {
                        source: "S".into(),
                        new_sequence: Some(format!("d{n}").as_str().into()),
                        name: None,
                        deep: rng.chance(50),
                    },
                    1 if !clips.is_empty() => Command::CreateNestedFromSelection {
                        clips: vec![
                            clips[rng.below(clips.len() as u64) as usize]
                                .as_str()
                                .into(),
                        ],
                        new_sequence: Some(format!("g{n}").as_str().into()),
                        name: None,
                        clip_id: Some(format!("gn{n}").as_str().into()),
                        track: None,
                        follow_length: false,
                    },
                    _ => {
                        let nested: Vec<String> = p
                            .document()
                            .sequence(&"S".into())
                            .map(|s| s.nested_refs().map(|(c, _)| c.0.clone()).collect())
                            .unwrap_or_default();
                        match nested.first() {
                            Some(c) => Command::FlattenNested {
                                clip: c.as_str().into(),
                                prefix: Some(format!("f{n}")),
                            },
                            None => continue,
                        }
                    }
                };
                let r = p.execute(&user, tx("compose", &format!("cmp-{n}"), cmd));
                match &r {
                    Ok(_) => bump(tally, "compose:ok"),
                    // falhas legítimas são estruturadas; nunca erro de persistência
                    Err(e) => assert_ne!(e.code, ErrorCode::PersistenceFailed, "{ctx}: {e}"),
                }
            }
            // undo / redo
            93..=96 => {
                let r = if rng.chance(60) {
                    p.undo(&user)
                } else {
                    p.redo(&user)
                };
                if r.is_ok() {
                    bump(tally, "undo_redo:ok");
                }
            }
            // reabrir: documento e catálogo idênticos
            _ => {
                let (digest, cat) = (sem(&p), catalog_json(&p));
                drop(p);
                bump(tally, "reopen");
                p = Project::open(&proj_path, &opts()).unwrap();
                assert_eq!(sem(&p), digest, "{ctx}: document differs after reopen");
                assert_eq!(catalog_json(&p), cat, "{ctx}: catalog differs after reopen");
            }
        }
        // invariantes: o catálogo bate com o modelo; documento válido
        let views = p.assets().unwrap();
        let from_catalog = views.iter().filter(|v| v.catalog.is_some()).count();
        assert_eq!(from_catalog, m.cat.len(), "{ctx}: catalog size");
        for c in m.cat.keys() {
            let v = p.asset(&Model::id(*c)).unwrap();
            assert_eq!(
                v.catalog.unwrap().status,
                m.quick(*c),
                "{ctx}: status of content {c}"
            );
        }
        assert!(
            capia_model::validate_document(p.document()).is_empty(),
            "{ctx}"
        );
    }
    // o arquivo termina íntegro e o cache (inexistente) não faz falta
    drop(p);
    let report = Project::validate(&proj_path);
    assert!(report.ok, "seed {seed}: {:?}", report.issues);
    let _ = std::fs::remove_dir_all(&dir);
}

fn cases() -> u64 {
    std::env::var("CAPIA_IO_PROP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200)
}

#[test]
fn assets_survive_random_imports_moves_relinks_edits_undo_and_reopens() {
    let n = cases();
    let mut tally = Tally::new();
    for seed in 0..n {
        run_case(seed, 40, &mut tally);
    }
    eprintln!("assets property: {n} cases × 40 steps: {tally:?}");
    if n >= 100 {
        // o gerador precisa mesmo exercitar cada ramo (senão a propriedade seria vazia)
        for k in [
            "import:created",
            "import:existing",
            "import:aliased",
            "import:auto_relinked",
            "import:reregistered",
            "relink:ok",
            "relink:rejected",
            "verify:online",
            "verify:offline",
            "verify:modified",
            "clip:placed",
            "delete_asset:in_use",
            "delete_asset:ok",
            "compose:ok",
            "undo_redo:ok",
            "reopen",
        ] {
            assert!(
                tally.get(k).copied().unwrap_or(0) > 0,
                "branch `{k}` never exercised: {tally:?}"
            );
        }
    }
}

#[test]
fn the_error_type_is_stable_json() {
    // contrato usado pela CLI: todo erro tem `code` e `message`
    let e = ProjectError::Invalid {
        code: "X",
        message: "m".into(),
        details: None,
    };
    let j = e.to_json();
    assert_eq!(
        (j["code"].as_str(), j["message"].as_str()),
        (Some("X"), Some("m"))
    );
}
