//! Critic (PHASE5_PLANNER_EDITOR_CRITIC §20..§31): checagens **determinísticas** locais (duração,
//! placeholders, assets offline, CTA, conteúdo proibido, safe area, buracos, beats ausentes,
//! formato) + achados semânticos (via LLM, em `roles.rs`) com **evidência**; e a compilação de
//! `CorrectionPlan` a partir de um vocabulário **fechado** de correções (nada de comando livre).
//!
//! O Critic **não escreve**: devolve `Review` + `CorrectionPlan` (JSON de comandos) que o
//! orquestrador passa por `preview → apply_plan`.

use super::plan::{EditPlan, Inventory, Layer, TICKS_PER_MS};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const RUBRIC_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Minor,
    Major,
    Blocker,
}

impl Severity {
    fn weight(self) -> f64 {
        match self {
            Self::Info => 0.0,
            Self::Minor => 0.05,
            Self::Major => 0.2,
            Self::Blocker => 0.4,
        }
    }

    pub fn blocks(self) -> bool {
        self >= Self::Major
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Timing,
    Pacing,
    Sync,
    Captions,
    Framing,
    Continuity,
    Brand,
    Brief,
    Reference,
    Audio,
    Transition,
    AssetQuality,
    Cta,
    Technical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSource {
    Deterministic,
    Semantic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Open,
    Resolved,
    Ignored,
    Locked,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// `timeline_range` | `beat` | `constraint` | `frame` | `transcript` | `metric` | `clip`.
    pub kind: String,
    pub detail: Value,
}

/// Vocabulário **fechado** de correções automáticas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum FixAction {
    /// Recorta a borda de saída do clip para o instante `new_end_ticks` (tempo da sequence).
    TrimToDuration {
        clip: String,
        new_end_ticks: i64,
    },
    AddCtaText {
        sequence: String,
        text: String,
        start_ticks: i64,
        duration_ticks: i64,
    },
    DeleteClip {
        clip: String,
    },
    SetProperty {
        clip: String,
        prop: String,
        value: f64,
    },
    SetClipEnabled {
        clip: String,
        enabled: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    /// Identidade estável do problema (travar/ignorar/oscilação).
    pub key: String,
    pub severity: Severity,
    pub category: Category,
    pub source: FindingSource,
    #[serde(default)]
    pub at: Option<RangeTicks>,
    pub evidence: Vec<Evidence>,
    pub expected: String,
    pub observed: String,
    #[serde(default)]
    pub suggested_fix: Option<FixAction>,
    pub confidence: f64,
    pub status: FindingStatus,
    /// O achado só se resolve replanejando.
    #[serde(default)]
    pub needs_replan: bool,
    #[serde(default)]
    pub deliverable: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeTicks {
    pub start: i64,
    pub end: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub id: String,
    pub run_id: String,
    pub plan_id: String,
    pub revision: u64,
    pub cycle: u32,
    pub score: f64,
    pub pass: bool,
    pub findings: Vec<Finding>,
    pub summary: String,
    pub rubric_version: u32,
    /// Proveniência (modelo/frames/DemandSpec) do lado semântico.
    #[serde(default)]
    pub provenance: Value,
}

impl Review {
    pub fn open_blocking(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.status == FindingStatus::Open && f.severity.blocks())
    }

    pub fn open_actionable(&self) -> impl Iterator<Item = &Finding> {
        self.open_blocking()
            .filter(|f| f.suggested_fix.is_some() && !f.needs_replan)
    }

    pub fn needs_replan(&self) -> bool {
        self.open_blocking().any(|f| f.needs_replan)
    }
}

/// Score da rubrica v1 (findings têm prioridade; o score só resume).
pub fn score(findings: &[Finding]) -> f64 {
    let penalty: f64 = findings
        .iter()
        .filter(|f| matches!(f.status, FindingStatus::Open))
        .map(|f| f.severity.weight() * f.confidence.clamp(0.0, 1.0))
        .sum();
    (1.0 - penalty).clamp(0.0, 1.0)
}

/// Aplica as decisões do usuário (ignorar/travar) — o Critic não insiste no próximo ciclo.
pub fn apply_decisions(
    findings: &mut [Finding],
    ignored: &BTreeSet<String>,
    locked: &BTreeSet<String>,
) {
    for f in findings {
        if locked.contains(&f.key) {
            f.status = FindingStatus::Locked;
        } else if ignored.contains(&f.key) {
            f.status = FindingStatus::Ignored;
        }
    }
}

pub fn make_review(
    run_id: &str,
    plan_id: &str,
    revision: u64,
    cycle: u32,
    mut findings: Vec<Finding>,
    provenance: Value,
) -> Review {
    findings.sort_by(|a, b| (b.severity, &a.key).cmp(&(a.severity, &b.key)));
    let sc = score(&findings);
    let open_block = findings
        .iter()
        .filter(|f| f.status == FindingStatus::Open && f.severity.blocks())
        .count();
    Review {
        id: format!("rv_{run_id}_{cycle}"),
        run_id: run_id.to_owned(),
        plan_id: plan_id.to_owned(),
        revision,
        cycle,
        score: sc,
        pass: open_block == 0,
        summary: if open_block == 0 {
            format!("{} finding(s), none blocking", findings.len())
        } else {
            format!("{open_block} blocking finding(s) of {}", findings.len())
        },
        findings,
        rubric_version: RUBRIC_VERSION,
        provenance,
    }
}

// ---- checagens determinísticas ------------------------------------------------------------------

/// O que o Critic sabe da demanda (extraído do `DemandSpec`; só texto verificável).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BriefFacts {
    pub cta: Option<String>,
    pub max_duration_ticks: Option<i64>,
    pub must_include: Vec<String>,
    pub must_avoid: Vec<String>,
}

#[derive(Debug)]
pub struct CheckInput<'a> {
    /// `(deliverable key, sequence JSON de `sequence.get`)`.
    pub sequences: &'a [(String, Value)],
    pub edit_plans: &'a [EditPlan],
    pub brief: &'a BriefFacts,
    pub inventory: &'a Inventory,
    pub cycle: u32,
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

fn contains_words(hay: &str, needle: &str) -> bool {
    let (h, n) = (norm(hay), norm(needle));
    !n.trim().is_empty() && h.contains(n.trim())
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Object(o) => ["static", "value", "v"]
            .iter()
            .find_map(|k| o.get(*k).and_then(num)),
        _ => None,
    }
}

fn static_prop(clip: &Value, name: &str) -> Option<f64> {
    clip["properties"].get(name).and_then(num)
}

fn clip_end(c: &Value) -> i64 {
    c["start"].as_i64().unwrap_or(0) + c["duration"].as_i64().unwrap_or(0)
}

fn seq_duration(seq: &Value) -> i64 {
    seq["clips"]
        .as_object()
        .map_or(0, |m| m.values().map(clip_end).max().unwrap_or(0))
}

fn finding(
    key: String,
    severity: Severity,
    category: Category,
    expected: String,
    observed: String,
    ev: Vec<Evidence>,
    fix: Option<FixAction>,
) -> Finding {
    Finding {
        id: key.clone(),
        key,
        severity,
        category,
        source: FindingSource::Deterministic,
        at: None,
        evidence: ev,
        expected,
        observed,
        suggested_fix: fix,
        confidence: 1.0,
        status: FindingStatus::Open,
        needs_replan: false,
        deliverable: None,
    }
}

/// Checagens locais e baratas, antes de qualquer LLM.
pub fn deterministic_checks(inp: &CheckInput<'_>) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    for (dk, seq) in inp.sequences {
        let tag = |f: &mut Finding| f.deliverable = Some(dk.clone());
        let clips: BTreeMap<String, Value> = seq["clips"]
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        if clips.is_empty() {
            let mut f = finding(
                format!("empty:{dk}"),
                Severity::Blocker,
                Category::Technical,
                "the sequence has content".into(),
                "the sequence is empty".into(),
                vec![],
                None,
            );
            f.needs_replan = true;
            tag(&mut f);
            out.push(f);
            continue;
        }
        let dur = seq_duration(seq);
        // 1. duração máxima
        if let Some(max) = inp.brief.max_duration_ticks
            && dur > max
        {
            // corta o último clip principal que ultrapassa o limite, se couber
            let over = dur - max;
            let last = clips
                .iter()
                .filter(|(_, c)| clip_end(c) == dur)
                .max_by_key(|(id, _)| (*id).clone());
            let fix = last.and_then(|(id, c)| {
                let d = c["duration"].as_i64().unwrap_or(0);
                (d > over).then(|| FixAction::TrimToDuration {
                    clip: id.clone(),
                    new_end_ticks: c["start"].as_i64().unwrap_or(0) + d - over,
                })
            });
            let mut f = finding(
                format!("duration:{dk}"),
                Severity::Major,
                Category::Timing,
                format!("duration <= {} ms", max / TICKS_PER_MS),
                format!("duration is {} ms", dur / TICKS_PER_MS),
                vec![Evidence {
                    kind: "constraint".into(),
                    detail: json!({"max_duration_ticks": max, "observed_ticks": dur}),
                }],
                fix.clone(),
            );
            f.needs_replan = fix.is_none();
            f.at = Some(RangeTicks {
                start: max,
                end: dur,
            });
            tag(&mut f);
            out.push(f);
        }
        // 2/3. placeholders e assets offline
        for (id, c) in &clips {
            let name = c["name"].as_str().unwrap_or("");
            if name.starts_with("PLACEHOLDER:") {
                let mut f = finding(
                    format!("placeholder:{dk}:{id}"),
                    Severity::Major,
                    Category::AssetQuality,
                    "every clip uses real media".into(),
                    format!("clip `{id}` is still `{name}`"),
                    vec![Evidence {
                        kind: "clip".into(),
                        detail: json!({"clip": id}),
                    }],
                    None,
                );
                f.needs_replan = true;
                f.at = Some(RangeTicks {
                    start: c["start"].as_i64().unwrap_or(0),
                    end: clip_end(c),
                });
                tag(&mut f);
                out.push(f);
            }
            if c["content"]["type"] == "media"
                && let Some(a) = c["content"]["asset"].as_str()
                && inp.inventory.get(a).is_some_and(|i| !i.online)
            {
                let mut f = finding(
                    format!("offline:{dk}:{id}"),
                    Severity::Blocker,
                    Category::Technical,
                    "every media asset is online".into(),
                    format!("clip `{id}` uses offline asset `{a}`"),
                    vec![Evidence {
                        kind: "clip".into(),
                        detail: json!({"clip": id, "asset": a}),
                    }],
                    None,
                );
                f.needs_replan = true;
                tag(&mut f);
                out.push(f);
            }
        }
        // 4. texto na timeline (CTA / must_avoid / must_include)
        let texts: Vec<(String, String)> = clips
            .iter()
            .filter(|(_, c)| c["content"]["type"] == "text")
            .map(|(id, c)| {
                (
                    id.clone(),
                    c["content"]["text"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect();
        let plan_text: String = inp
            .edit_plans
            .iter()
            .filter(|p| &p.deliverable_key == dk)
            .flat_map(|p| p.beats.iter())
            .filter_map(|b| b.script_segment.clone())
            .collect::<Vec<_>>()
            .join(" ");
        let all_text = format!(
            "{} {}",
            texts
                .iter()
                .map(|t| t.1.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            plan_text
        );
        if let Some(cta) = &inp.brief.cta
            && !contains_words(&all_text, cta)
        {
            let start = (dur - 2 * 705_600_000).max(0);
            let mut f = finding(
                format!("cta:{dk}"),
                Severity::Major,
                Category::Cta,
                format!("the CTA \"{cta}\" is present"),
                "no text or script segment contains the CTA".into(),
                vec![Evidence {
                    kind: "constraint".into(),
                    detail: json!({"cta": cta}),
                }],
                Some(FixAction::AddCtaText {
                    sequence: seq["header"]["id"].as_str().unwrap_or(dk).to_owned(),
                    text: cta.clone(),
                    start_ticks: start,
                    duration_ticks: (dur - start).max(705_600_000 / 30),
                }),
            );
            f.at = Some(RangeTicks { start, end: dur });
            tag(&mut f);
            out.push(f);
        }
        for bad in &inp.brief.must_avoid {
            for (id, t) in &texts {
                if contains_words(t, bad) {
                    let mut f = finding(
                        format!("avoid:{dk}:{id}"),
                        Severity::Blocker,
                        Category::Brief,
                        format!("\"{bad}\" must not appear"),
                        format!("text clip `{id}` contains it"),
                        vec![Evidence {
                            kind: "clip".into(),
                            detail: json!({"clip": id}),
                        }],
                        Some(FixAction::DeleteClip { clip: id.clone() }),
                    );
                    tag(&mut f);
                    out.push(f);
                }
            }
        }
        for need in &inp.brief.must_include {
            if !contains_words(&all_text, need) {
                let mut f = finding(
                    format!("include:{dk}:{}", norm(need).replace(' ', "_")),
                    Severity::Major,
                    Category::Brief,
                    format!("\"{need}\" must be covered"),
                    "not found in on-screen text or script".into(),
                    vec![Evidence {
                        kind: "constraint".into(),
                        detail: json!({"must_include": need}),
                    }],
                    None,
                );
                f.needs_replan = true;
                tag(&mut f);
                out.push(f);
            }
        }
        // 5. safe area das legendas
        let height = seq["header"]["height"].as_f64().unwrap_or(1080.0);
        for (id, c) in &clips {
            let is_caption = seq["tracks"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|t| t["id"] == c["track"] && t["role"] == "captions");
            if is_caption
                && let Some(y) = static_prop(c, "position_y")
                && y.abs() > 0.47 * height
            {
                let mut f = finding(
                    format!("safearea:{dk}:{id}"),
                    Severity::Minor,
                    Category::Captions,
                    "captions stay inside the safe area".into(),
                    format!("caption `{id}` is at y={y}"),
                    vec![Evidence {
                        kind: "clip".into(),
                        detail: json!({"clip": id, "position_y": y}),
                    }],
                    Some(FixAction::SetProperty {
                        clip: id.clone(),
                        prop: "position_y".into(),
                        value: 0.315 * height,
                    }),
                );
                tag(&mut f);
                out.push(f);
            }
        }
        // 6. buracos na trilha principal
        let main_tracks: Vec<&str> = seq["tracks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["role"] == "main" && t["kind"] == "visual")
            .filter_map(|t| t["id"].as_str())
            .collect();
        for tr in main_tracks {
            let mut on: Vec<(&String, &Value)> =
                clips.iter().filter(|(_, c)| c["track"] == tr).collect();
            on.sort_by_key(|(_, c)| c["start"].as_i64().unwrap_or(0));
            for w in on.windows(2) {
                let end = clip_end(w[0].1);
                let next = w[1].1["start"].as_i64().unwrap_or(0);
                if next - end > 705_600_000 / 30 {
                    let mut f = finding(
                        format!("gap:{dk}:{}", w[0].0),
                        Severity::Minor,
                        Category::Continuity,
                        "no empty gap in the main track".into(),
                        format!(
                            "gap of {} ms after `{}`",
                            (next - end) / TICKS_PER_MS,
                            w[0].0
                        ),
                        vec![Evidence {
                            kind: "timeline_range".into(),
                            detail: json!({"start": end, "end": next}),
                        }],
                        None,
                    );
                    f.at = Some(RangeTicks {
                        start: end,
                        end: next,
                    });
                    tag(&mut f);
                    out.push(f);
                }
            }
        }
        // 7. beats do plano ausentes
        for ep in inp.edit_plans.iter().filter(|p| &p.deliverable_key == dk) {
            for b in ep.beats.iter().filter(|b| b.layer == Layer::Main) {
                let suffix = format!("_{}", b.id);
                if !clips.keys().any(|k| k.ends_with(&suffix)) {
                    let mut f = finding(
                        format!("beat:{dk}:{}", b.id),
                        Severity::Major,
                        Category::Brief,
                        format!("beat `{}` ({}) is on the timeline", b.id, b.role),
                        "no clip of this beat was found".into(),
                        vec![Evidence {
                            kind: "beat".into(),
                            detail: json!({"beat": b.id}),
                        }],
                        None,
                    );
                    f.needs_replan = true;
                    tag(&mut f);
                    out.push(f);
                }
            }
            // 8. formato
            let (w, h) = (
                seq["header"]["width"].as_u64().unwrap_or(0),
                seq["header"]["height"].as_u64().unwrap_or(0),
            );
            if (w, h) != (u64::from(ep.format.width), u64::from(ep.format.height)) && w != 0 {
                let mut f = finding(
                    format!("format:{dk}"),
                    Severity::Major,
                    Category::Technical,
                    format!("{}x{}", ep.format.width, ep.format.height),
                    format!("{w}x{h}"),
                    vec![],
                    None,
                );
                f.needs_replan = true;
                tag(&mut f);
                out.push(f);
            }
        }
    }
    out
}

// ---- CorrectionPlan -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CorrectionPlan {
    pub id: String,
    pub cycle: u32,
    pub finding_refs: Vec<String>,
    pub commands: Vec<Value>,
    pub affected_clips: Vec<String>,
    pub expected_improvement: String,
    pub skipped: Vec<String>,
}

/// Compila as correções **mínimas** dos achados acionáveis. `operation_id` determinístico por
/// (run, ciclo, achado) — reaplicar a mesma correção nunca duplica.
pub fn compile_correction(run_id: &str, cycle: u32, findings: &[&Finding]) -> CorrectionPlan {
    let mut commands = Vec::new();
    let mut refs = Vec::new();
    let mut affected = Vec::new();
    let mut skipped = Vec::new();
    for (i, f) in findings.iter().enumerate() {
        let Some(fix) = &f.suggested_fix else {
            skipped.push(f.key.clone());
            continue;
        };
        let op = |k: &str| format!("{run_id}:c{cycle}:{i}:{k}");
        match fix {
            FixAction::TrimToDuration {
                clip,
                new_end_ticks,
            } => {
                commands.push(
                    json!({"operation_id": op("trim"), "type": "trim_clip", "clip": clip,
                    "edge": "out", "to": new_end_ticks}),
                );
                affected.push(clip.clone());
            }
            FixAction::AddCtaText {
                sequence,
                text,
                start_ticks,
                duration_ticks,
            } => {
                let track = format!("{sequence}_cta_c{cycle}");
                let clip_id = format!("{sequence}_cta_{cycle}_{i}");
                commands.push(json!({"operation_id": op("ctatrack"), "type": "add_track", "sequence": sequence,
                    "id": track, "kind": "visual", "role": "text", "name": "CTA"}));
                commands.push(json!({"operation_id": op("cta"), "type": "insert_clip", "track": track, "start": start_ticks,
                    "clip": {"id": clip_id, "name": "CTA", "duration": duration_ticks,
                             "content": {"type": "text", "text": text,
                                         "style": {"size_permille": 70, "weight": 800, "color": "#FFFFFF", "background": "#000000B3"}},
                             "source_in": 0, "speed": "1", "reversed": false, "properties": {}}}));
                affected.push(clip_id);
            }
            FixAction::DeleteClip { clip } => {
                commands
                    .push(json!({"operation_id": op("del"), "type": "delete_clip", "clip": clip}));
                affected.push(clip.clone());
            }
            FixAction::SetProperty { clip, prop, value } => {
                commands.push(json!({"operation_id": op("prop"), "type": "set_property", "clip": clip, "prop": prop, "value": value}));
                affected.push(clip.clone());
            }
            FixAction::SetClipEnabled { clip, enabled } => {
                commands.push(json!({"operation_id": op("en"), "type": "set_clip_enabled", "clip": clip, "enabled": enabled}));
                affected.push(clip.clone());
            }
        }
        refs.push(f.key.clone());
    }
    CorrectionPlan {
        id: format!("cp_{run_id}_{cycle}"),
        cycle,
        expected_improvement: format!("resolve {} finding(s)", refs.len()),
        finding_refs: refs,
        commands,
        affected_clips: affected,
        skipped,
    }
}

/// Detecta oscilação: o mesmo achado voltou depois de uma correção que o visava, ou o score não
/// melhorou entre ciclos consecutivos.
pub fn oscillating(history: &[Review]) -> bool {
    if history.len() < 2 {
        return false;
    }
    let (prev, cur) = (&history[history.len() - 2], &history[history.len() - 1]);
    let prev_blocking: BTreeSet<&String> = prev.open_blocking().map(|f| &f.key).collect();
    let cur_blocking: BTreeSet<&String> = cur.open_blocking().map(|f| &f.key).collect();
    let no_progress = cur.score <= prev.score + 1e-9 && !cur_blocking.is_empty();
    let same_back = !cur_blocking.is_empty()
        && cur_blocking.is_subset(&prev_blocking)
        && cur_blocking.len() >= prev_blocking.len();
    no_progress || same_back
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autonomy::plan::{AssetInfo, edit_plan_from};

    fn seq() -> Value {
        json!({
            "header": {"id": "S", "name": "S", "width": 1080, "height": 1920},
            "tracks": [
                {"id": "S_V1", "kind": "visual", "role": "main"},
                {"id": "S_CP", "kind": "visual", "role": "captions"}
            ],
            "clips": {
                "S_c_hook": {"track": "S_V1", "start": 0, "duration": 3000 * TICKS_PER_MS, "name": "hook",
                              "content": {"type": "media", "asset": "raw"}},
                "S_c_body": {"track": "S_V1", "start": 3000 * TICKS_PER_MS, "duration": 5000 * TICKS_PER_MS, "name": "PLACEHOLDER:n1",
                              "content": {"type": "solid", "color": "#2B2B2B"}},
                "cap1": {"track": "S_CP", "start": 0, "duration": 1000 * TICKS_PER_MS, "name": "",
                         "content": {"type": "text", "text": "Compre agora"}, "properties": {"position_y": 1800.0}}
            }
        })
    }

    fn inv(online: bool) -> Inventory {
        let mut i = Inventory::default();
        i.assets.insert(
            "raw".into(),
            AssetInfo {
                id: "raw".into(),
                name: "raw".into(),
                duration_ticks: None,
                has_video: true,
                has_audio: true,
                online,
                is_image: false,
            },
        );
        i
    }

    fn input<'a>(
        s: &'a [(String, Value)],
        b: &'a BriefFacts,
        i: &'a Inventory,
        e: &'a [EditPlan],
    ) -> CheckInput<'a> {
        CheckInput {
            sequences: s,
            edit_plans: e,
            brief: b,
            inventory: i,
            cycle: 0,
        }
    }

    #[test]
    fn detects_the_deterministic_problems_with_evidence() {
        let s = vec![("main".to_owned(), seq())];
        let brief = BriefFacts {
            cta: Some("Garanta o seu hoje".into()),
            max_duration_ticks: Some(6000 * TICKS_PER_MS),
            must_include: vec![],
            must_avoid: vec!["compre".into()],
        };
        let i = inv(true);
        let f = deterministic_checks(&input(&s, &brief, &i, &[]));
        let keys: Vec<&str> = f.iter().map(|x| x.key.as_str()).collect();
        assert!(keys.contains(&"duration:main"), "{keys:?}");
        assert!(keys.contains(&"placeholder:main:S_c_body"));
        assert!(keys.contains(&"cta:main"));
        assert!(keys.iter().any(|k| k.starts_with("avoid:main:")));
        assert!(keys.iter().any(|k| k.starts_with("safearea:main:")));
        let dur = f.iter().find(|x| x.key == "duration:main").unwrap();
        assert!(matches!(
            dur.suggested_fix,
            Some(FixAction::TrimToDuration { .. })
        ));
        assert!(
            f.iter()
                .all(|x| x.confidence == 1.0 && x.source == FindingSource::Deterministic)
        );
        let rv = make_review("r", "p", 3, 0, f, Value::Null);
        assert!(!rv.pass);
        assert!(rv.score < 0.5);
    }

    #[test]
    fn offline_assets_block_and_a_clean_timeline_passes() {
        let s = vec![("main".to_owned(), seq())];
        let brief = BriefFacts::default();
        let off = inv(false);
        let f = deterministic_checks(&input(&s, &brief, &off, &[]));
        assert!(
            f.iter()
                .any(|x| x.key.starts_with("offline:") && x.severity == Severity::Blocker)
        );
        // timeline limpa
        let clean = json!({"header": {"id": "S", "width": 1080, "height": 1920}, "tracks": [{"id": "S_V1", "kind": "visual", "role": "main"}],
            "clips": {"S_c_a": {"track": "S_V1", "start": 0, "duration": 1000 * TICKS_PER_MS, "name": "a", "content": {"type": "media", "asset": "raw"}}}});
        let s = vec![("main".to_owned(), clean)];
        let on = inv(true);
        let rv = make_review(
            "r",
            "p",
            1,
            0,
            deterministic_checks(&input(&s, &brief, &on, &[])),
            Value::Null,
        );
        assert!(
            rv.pass && (rv.score - 1.0).abs() < 1e-9,
            "{:?}",
            rv.findings
        );
    }

    #[test]
    fn missing_beats_and_wrong_format_need_a_replan() {
        let ep = edit_plan_from(
            &json!({"format": {"width": 720, "height": 1280}, "estimated_duration_ms": 3000,
                    "beats": [{"id": "ghost", "role": "hook", "duration_ms": 3000, "asset": {"asset_id": "raw", "source_in_ms": 0}}]}),
            "e".into(), 1, "main").unwrap();
        let s = vec![("main".to_owned(), seq())];
        let i = inv(true);
        let b = BriefFacts::default();
        let plans = [ep];
        let f = deterministic_checks(&input(&s, &b, &i, &plans));
        let beat = f.iter().find(|x| x.key == "beat:main:ghost").unwrap();
        assert!(beat.needs_replan);
        assert!(f.iter().any(|x| x.key == "format:main" && x.needs_replan));
    }

    #[test]
    fn corrections_are_closed_vocabulary_minimal_and_deterministic() {
        let fixes = [
            Finding {
                suggested_fix: Some(FixAction::TrimToDuration {
                    clip: "c1".into(),
                    new_end_ticks: 100,
                }),
                ..finding(
                    "a".into(),
                    Severity::Major,
                    Category::Timing,
                    "e".into(),
                    "o".into(),
                    vec![],
                    None,
                )
            },
            Finding {
                suggested_fix: Some(FixAction::DeleteClip { clip: "t1".into() }),
                ..finding(
                    "b".into(),
                    Severity::Blocker,
                    Category::Brief,
                    "e".into(),
                    "o".into(),
                    vec![],
                    None,
                )
            },
            finding(
                "c".into(),
                Severity::Major,
                Category::Brief,
                "e".into(),
                "o".into(),
                vec![],
                None,
            ),
        ];
        let refs: Vec<&Finding> = fixes.iter().collect();
        let a = compile_correction("run1", 1, &refs);
        let b = compile_correction("run1", 1, &refs);
        assert_eq!(a, b);
        assert_eq!(a.commands.len(), 2);
        assert_eq!(a.skipped, vec!["c".to_owned()]);
        assert!(
            a.commands
                .iter()
                .all(|c| c["operation_id"].as_str().unwrap().starts_with("run1:c1:"))
        );
        // outro ciclo ⇒ outro namespace
        assert_ne!(
            a.commands[0]["operation_id"],
            compile_correction("run1", 2, &refs).commands[0]["operation_id"]
        );
    }

    #[test]
    fn locked_and_ignored_findings_stop_blocking() {
        let mut f = vec![
            finding(
                "k1".into(),
                Severity::Major,
                Category::Timing,
                "e".into(),
                "o".into(),
                vec![],
                None,
            ),
            finding(
                "k2".into(),
                Severity::Blocker,
                Category::Brief,
                "e".into(),
                "o".into(),
                vec![],
                None,
            ),
        ];
        let ign: BTreeSet<String> = ["k1".to_owned()].into();
        let lock: BTreeSet<String> = ["k2".to_owned()].into();
        apply_decisions(&mut f, &ign, &lock);
        let rv = make_review("r", "p", 1, 0, f, Value::Null);
        assert!(rv.pass);
        assert_eq!(rv.open_blocking().count(), 0);
    }

    #[test]
    fn oscillation_is_detected_when_the_same_finding_returns_without_progress() {
        let mk = |score_findings: Vec<Finding>, c: u32| {
            make_review("r", "p", 1, c, score_findings, Value::Null)
        };
        let f = || {
            finding(
                "k".into(),
                Severity::Major,
                Category::Timing,
                "e".into(),
                "o".into(),
                vec![],
                None,
            )
        };
        let r0 = mk(vec![f()], 0);
        let r1 = mk(vec![f()], 1);
        assert!(oscillating(&[r0.clone(), r1]));
        let r2 = mk(vec![], 1);
        assert!(!oscillating(&[r0, r2]));
        assert!(!oscillating(&[]));
    }
}
