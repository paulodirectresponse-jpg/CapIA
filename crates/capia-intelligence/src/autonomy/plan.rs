//! `ProductionPlan` (Producer) e `EditPlan` (Planner): esquemas versionados, validação semântica
//! (assets, durações, formatos), **compilação determinística** `EditPlan → comandos do Command
//! Engine` e `ValidationReport`. Tempo no plano é em **milissegundos inteiros** (nunca float de
//! segundos); a compilação converte para `Ticks` e alinha a quadro.
//!
//! Nada aqui escreve: o resultado da compilação é só JSON de comandos que o orquestrador
//! submete por `preview → apply_plan` com o ator da Run.

use super::model::AcquireSource;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PLAN_SCHEMA_VERSION: u32 = 1;
pub const TICKS_PER_MS: i64 = 705_600;
pub const MAX_BEATS: usize = 400;
pub const MAX_DELIVERABLES: usize = 24;
pub const MAX_ASSET_NEEDS: usize = 64;
pub const MAX_PLAN_TEXT: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanError {
    pub code: &'static str,
    pub message: String,
}

impl PlanError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl core::fmt::Display for PlanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PlanError {}

pub fn digest_value(v: &Value) -> String {
    let mut h = Sha256::new();
    h.update(serde_json::to_vec(v).unwrap_or_default());
    let d = h.finalize();
    let mut s = String::from("sha256:");
    for b in &d[..16] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn digest_of<T: Serialize>(t: &T) -> String {
    digest_value(&serde_json::to_value(t).unwrap_or(Value::Null))
}

// ---- ProductionPlan ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequenceStrategy {
    Standalone,
    HookPlusMaster,
    SharedMaster,
    FormatVariant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Video,
    Image,
    Audio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeedStatus {
    Open,
    Resolved,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverablePlan {
    pub key: String,
    pub sequence_strategy: SequenceStrategy,
    #[serde(default)]
    pub source_material: Vec<String>,
    #[serde(default)]
    pub hooks: Vec<String>,
    /// Deliverable (`key`) que serve de master compartilhado.
    #[serde(default)]
    pub master: Option<String>,
    /// Deliverable de origem (variante de formato/CTA).
    #[serde(default)]
    pub variant_of: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetNeed {
    pub id: String,
    pub kind: AssetKind,
    pub purpose: String,
    pub description: String,
    #[serde(default)]
    pub source_priority: Vec<AcquireSource>,
    pub required: bool,
    #[serde(default)]
    pub estimated_cost_micros: Option<u64>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub target_duration_ms: Option<i64>,
    /// URL explícita (só entra no Gateway se o adapter/host permitir).
    #[serde(default)]
    pub hint_url: Option<String>,
    #[serde(default = "open_status")]
    pub status: NeedStatus,
    #[serde(default)]
    pub resolved_asset_id: Option<String>,
}

fn open_status() -> NeedStatus {
    NeedStatus::Open
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryProposalDraft {
    pub kind: String,
    pub content: String,
    /// Escopo **sugerido** — nunca ativa sozinho (User/Client exigem aprovação humana).
    pub scope: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProductionPlan {
    pub schema_version: u32,
    pub id: String,
    pub version: u32,
    pub demand_spec_version: Option<u32>,
    pub strategy: String,
    pub deliverables: Vec<DeliverablePlan>,
    pub asset_needs: Vec<AssetNeed>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub memory_proposals: Vec<MemoryProposalDraft>,
}

impl ProductionPlan {
    pub fn digest(&self) -> String {
        digest_of(self)
    }

    pub fn deliverable(&self, key: &str) -> Option<&DeliverablePlan> {
        self.deliverables.iter().find(|d| d.key == key)
    }

    pub fn need(&self, id: &str) -> Option<&AssetNeed> {
        self.asset_needs.iter().find(|n| n.id == id)
    }

    /// Ordem de execução: masters antes dos deliverables que dependem deles.
    pub fn execution_order(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut left: Vec<&DeliverablePlan> = self.deliverables.iter().collect();
        while !left.is_empty() {
            let before = left.len();
            let mut next = Vec::new();
            for d in left {
                let dep_ready = d
                    .master
                    .as_ref()
                    .is_none_or(|m| out.contains(m) || self.deliverable(m).is_none());
                if dep_ready {
                    out.push(d.key.clone());
                } else {
                    next.push(d);
                }
            }
            left = next;
            if left.len() == before {
                // ciclo: a validação já recusa; por segurança, mantém a ordem original
                out.extend(left.iter().map(|d| d.key.clone()));
                break;
            }
        }
        out
    }
}

// ---- EditPlan ---------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Main,
    Overlay,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetSel {
    #[serde(default)]
    pub asset_id: Option<String>,
    /// Alternativa: asset ainda a adquirir (placeholder tipado).
    #[serde(default)]
    pub need_id: Option<String>,
    #[serde(default)]
    pub source_in_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextOverlay {
    pub text: String,
    #[serde(default)]
    pub start_offset_ms: i64,
    pub duration_ms: i64,
    /// `top` | `center` | `lower_third` (posição vertical por preset).
    #[serde(default)]
    pub position: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionCue {
    pub text: String,
    #[serde(default)]
    pub start_offset_ms: i64,
    pub duration_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransitionIntent {
    /// `dissolve` | `fade` | `slide_in` (corte seco = ausente).
    pub kind: String,
    pub duration_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Framing {
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub position_x: Option<f64>,
    #[serde(default)]
    pub position_y: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Beat {
    pub id: String,
    /// `hook` | `problem` | `solution` | `proof` | `offer` | `cta` | `broll` | `body` | …
    pub role: String,
    #[serde(default)]
    pub script_segment: Option<String>,
    pub duration_ms: i64,
    #[serde(default)]
    pub asset: Option<AssetSel>,
    #[serde(default = "main_layer")]
    pub layer: Layer,
    /// Só para `overlay`: instante absoluto (ms) de início; padrão = início do beat principal anterior.
    #[serde(default)]
    pub at_ms: Option<i64>,
    #[serde(default)]
    pub overlays: Vec<TextOverlay>,
    #[serde(default)]
    pub captions: Vec<CaptionCue>,
    #[serde(default)]
    pub transition: Option<TransitionIntent>,
    #[serde(default)]
    pub framing: Option<Framing>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

fn main_layer() -> Layer {
    Layer::Main
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceFormatSpec {
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_fps")]
    pub fps: u32,
}

fn default_fps() -> u32 {
    30
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GlobalEditSpec {
    /// Asset de música (opcional) e ganho (dB).
    #[serde(default)]
    pub music_asset: Option<String>,
    #[serde(default)]
    pub music_volume_db: Option<f64>,
    /// Tamanho do texto (‰ da altura) para overlays e legendas.
    #[serde(default)]
    pub text_size_permille: Option<u32>,
    #[serde(default)]
    pub cta_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetPlaceholder {
    pub need_id: String,
    pub beat_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditPlan {
    pub schema_version: u32,
    pub id: String,
    pub version: u32,
    pub deliverable_key: String,
    #[serde(default)]
    pub target_sequence: Option<String>,
    pub format: SequenceFormatSpec,
    #[serde(default)]
    pub grammar_ref: Option<String>,
    pub beats: Vec<Beat>,
    #[serde(default)]
    pub global: GlobalEditSpec,
    #[serde(default)]
    pub constraints_checked: Vec<String>,
    #[serde(default)]
    pub asset_placeholders: Vec<AssetPlaceholder>,
    pub estimated_duration_ms: i64,
    #[serde(default)]
    pub estimated_cost_micros: Option<u64>,
}

impl EditPlan {
    pub fn digest(&self) -> String {
        digest_of(self)
    }

    pub fn total_main_ms(&self) -> i64 {
        self.beats
            .iter()
            .filter(|b| b.layer == Layer::Main)
            .map(|b| b.duration_ms)
            .sum()
    }
}

// ---- inventário --------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetInfo {
    pub id: String,
    pub name: String,
    pub duration_ticks: Option<i64>,
    pub has_video: bool,
    pub has_audio: bool,
    pub online: bool,
    pub is_image: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub assets: BTreeMap<String, AssetInfo>,
}

impl Inventory {
    /// Constrói a partir da resposta de `assets.list`.
    pub fn from_assets_list(v: &Value) -> Self {
        let mut assets = BTreeMap::new();
        for a in v.as_array().into_iter().flatten() {
            let Some(id) = a["id"].as_str() else { continue };
            let kind = a["kind"].as_str().unwrap_or("");
            let status = a["status"].as_str().unwrap_or("online");
            assets.insert(
                id.to_owned(),
                AssetInfo {
                    id: id.to_owned(),
                    name: a["name"].as_str().unwrap_or("").to_owned(),
                    duration_ticks: a["duration"].as_i64(),
                    has_video: a["has_video"].as_bool().unwrap_or(kind == "video" || kind == "image"),
                    has_audio: a["has_audio"].as_bool().unwrap_or(kind == "audio"),
                    online: status == "online",
                    is_image: kind == "image",
                },
            );
        }
        Self { assets }
    }

    pub fn get(&self, id: &str) -> Option<&AssetInfo> {
        self.assets.get(id)
    }
}

// ---- JSON Schemas (saída dos papéis) ----------------------------------------------------------

pub fn production_plan_schema() -> Value {
    let str_arr = json!({"type": "array", "items": {"type": "string"}});
    json!({
        "type": "object",
        "properties": {
            "strategy": {"type": "string"},
            "deliverables": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "key": {"type": "string"},
                    "sequence_strategy": {"type": "string", "enum": ["standalone", "hook_plus_master", "shared_master", "format_variant"]},
                    "source_material": str_arr,
                    "hooks": str_arr,
                    "master": {"type": ["string", "null"]},
                    "variant_of": {"type": ["string", "null"]},
                    "notes": {"type": ["string", "null"]}
                },
                "required": ["key", "sequence_strategy"]
            }},
            "asset_needs": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "kind": {"type": "string", "enum": ["video", "image", "audio"]},
                    "purpose": {"type": "string"},
                    "description": {"type": "string"},
                    "source_priority": {"type": "array", "items": {"type": "string", "enum": ["project", "library", "gateway", "generate"]}},
                    "required": {"type": "boolean"},
                    "estimated_cost_micros": {"type": ["integer", "null"]},
                    "constraints": str_arr,
                    "target_duration_ms": {"type": ["integer", "null"]},
                    "hint_url": {"type": ["string", "null"]}
                },
                "required": ["id", "kind", "purpose", "description", "required"]
            }},
            "constraints": str_arr,
            "assumptions": str_arr,
            "risks": str_arr,
            "memory_proposals": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "kind": {"type": "string"},
                    "content": {"type": "string"},
                    "scope": {"type": "string"},
                    "evidence": str_arr,
                    "confidence": {"type": ["number", "null"]}
                },
                "required": ["kind", "content", "scope"]
            }}
        },
        "required": ["strategy", "deliverables", "asset_needs"]
    })
}

pub fn edit_plan_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "target_sequence": {"type": ["string", "null"]},
            "format": {"type": "object", "properties": {
                "width": {"type": "integer"}, "height": {"type": "integer"}, "fps": {"type": "integer"}
            }, "required": ["width", "height"]},
            "grammar_ref": {"type": ["string", "null"]},
            "beats": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "role": {"type": "string"},
                    "script_segment": {"type": ["string", "null"]},
                    "duration_ms": {"type": "integer"},
                    "asset": {"type": ["object", "null"], "properties": {
                        "asset_id": {"type": ["string", "null"]},
                        "need_id": {"type": ["string", "null"]},
                        "source_in_ms": {"type": "integer"}
                    }},
                    "layer": {"type": "string", "enum": ["main", "overlay"]},
                    "at_ms": {"type": ["integer", "null"]},
                    "overlays": {"type": "array", "items": {"type": "object", "properties": {
                        "text": {"type": "string"}, "start_offset_ms": {"type": "integer"},
                        "duration_ms": {"type": "integer"}, "position": {"type": ["string", "null"]}
                    }, "required": ["text", "duration_ms"]}},
                    "captions": {"type": "array", "items": {"type": "object", "properties": {
                        "text": {"type": "string"}, "start_offset_ms": {"type": "integer"},
                        "duration_ms": {"type": "integer"}
                    }, "required": ["text", "duration_ms"]}},
                    "transition": {"type": ["object", "null"], "properties": {
                        "kind": {"type": "string", "enum": ["dissolve", "fade", "slide_in"]},
                        "duration_ms": {"type": "integer"}
                    }},
                    "framing": {"type": ["object", "null"], "properties": {
                        "scale": {"type": ["number", "null"]},
                        "position_x": {"type": ["number", "null"]},
                        "position_y": {"type": ["number", "null"]}
                    }},
                    "notes": {"type": ["string", "null"]},
                    "depends_on": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["id", "role", "duration_ms"]
            }},
            "global": {"type": "object", "properties": {
                "music_asset": {"type": ["string", "null"]},
                "music_volume_db": {"type": ["number", "null"]},
                "text_size_permille": {"type": ["integer", "null"]},
                "cta_text": {"type": ["string", "null"]}
            }},
            "constraints_checked": {"type": "array", "items": {"type": "string"}},
            "estimated_duration_ms": {"type": "integer"},
            "estimated_cost_micros": {"type": ["integer", "null"]}
        },
        "required": ["format", "beats", "estimated_duration_ms"]
    })
}

// ---- construção a partir da saída (validada por schema) do modelo -----------------------------

fn clip_text(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect()
}

fn slug(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .take(40)
        .collect();
    if out.is_empty() {
        out.push('x');
    }
    out
}

pub fn production_plan_from(
    raw: &Value,
    id: String,
    version: u32,
    demand_spec_version: Option<u32>,
) -> Result<ProductionPlan, PlanError> {
    #[derive(Deserialize)]
    struct Raw {
        strategy: String,
        deliverables: Vec<DeliverablePlan>,
        asset_needs: Vec<AssetNeed>,
        #[serde(default)]
        constraints: Vec<String>,
        #[serde(default)]
        assumptions: Vec<String>,
        #[serde(default)]
        risks: Vec<String>,
        #[serde(default)]
        memory_proposals: Vec<MemoryProposalDraft>,
    }
    let r: Raw = serde_json::from_value(raw.clone())
        .map_err(|e| PlanError::new("PLAN_SCHEMA", e.to_string()))?;
    let mut plan = ProductionPlan {
        schema_version: PLAN_SCHEMA_VERSION,
        id,
        version,
        demand_spec_version,
        strategy: clip_text(&r.strategy, MAX_PLAN_TEXT),
        deliverables: r.deliverables,
        asset_needs: r.asset_needs,
        constraints: r.constraints.iter().map(|s| clip_text(s, 400)).collect(),
        assumptions: r.assumptions.iter().map(|s| clip_text(s, 400)).collect(),
        risks: r.risks.iter().map(|s| clip_text(s, 400)).collect(),
        memory_proposals: r.memory_proposals,
    };
    for d in &mut plan.deliverables {
        d.key = slug(&d.key);
        d.master = d.master.take().map(|m| slug(&m));
        d.variant_of = d.variant_of.take().map(|m| slug(&m));
        d.notes = d.notes.take().map(|n| clip_text(&n, 400));
    }
    for n in &mut plan.asset_needs {
        n.id = slug(&n.id);
        n.purpose = clip_text(&n.purpose, 200);
        n.description = clip_text(&n.description, 600);
        n.status = NeedStatus::Open;
        n.resolved_asset_id = None;
        if n.source_priority.is_empty() {
            n.source_priority = vec![
                AcquireSource::Project,
                AcquireSource::Library,
                AcquireSource::Gateway,
                AcquireSource::Generate,
            ];
        }
    }
    validate_production_plan(&plan)?;
    Ok(plan)
}

pub fn validate_production_plan(p: &ProductionPlan) -> Result<(), PlanError> {
    if p.deliverables.is_empty() {
        return Err(PlanError::new("PLAN_EMPTY", "the plan has no deliverables"));
    }
    if p.deliverables.len() > MAX_DELIVERABLES || p.asset_needs.len() > MAX_ASSET_NEEDS {
        return Err(PlanError::new("PLAN_LIMIT", "the plan is too large"));
    }
    let mut keys = BTreeSet::new();
    for d in &p.deliverables {
        if d.key.is_empty() || !keys.insert(d.key.clone()) {
            return Err(PlanError::new(
                "PLAN_DUPLICATE_DELIVERABLE",
                format!("deliverable `{}` is empty or repeated", d.key),
            ));
        }
    }
    for d in &p.deliverables {
        for r in d.master.iter().chain(d.variant_of.iter()) {
            if !keys.contains(r) || r == &d.key {
                return Err(PlanError::new(
                    "PLAN_BAD_REFERENCE",
                    format!("deliverable `{}` references unknown/own `{r}`", d.key),
                ));
            }
        }
        if matches!(
            d.sequence_strategy,
            SequenceStrategy::HookPlusMaster | SequenceStrategy::SharedMaster
        ) && d.master.is_none()
        {
            return Err(PlanError::new(
                "PLAN_MASTER_MISSING",
                format!("deliverable `{}` needs a `master`", d.key),
            ));
        }
    }
    // ciclo de masters
    for d in &p.deliverables {
        let mut cur = d.master.clone();
        let mut hops = 0;
        while let Some(m) = cur {
            hops += 1;
            if m == d.key || hops > p.deliverables.len() {
                return Err(PlanError::new("PLAN_MASTER_CYCLE", "master references form a cycle"));
            }
            cur = p.deliverable(&m).and_then(|x| x.master.clone());
        }
    }
    let mut needs = BTreeSet::new();
    for n in &p.asset_needs {
        if n.id.is_empty() || !needs.insert(n.id.clone()) {
            return Err(PlanError::new(
                "PLAN_DUPLICATE_NEED",
                format!("asset need `{}` is empty or repeated", n.id),
            ));
        }
    }
    Ok(())
}

pub fn edit_plan_from(
    raw: &Value,
    id: String,
    version: u32,
    deliverable_key: &str,
) -> Result<EditPlan, PlanError> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        target_sequence: Option<String>,
        format: SequenceFormatSpec,
        #[serde(default)]
        grammar_ref: Option<String>,
        beats: Vec<Beat>,
        #[serde(default)]
        global: GlobalEditSpec,
        #[serde(default)]
        constraints_checked: Vec<String>,
        estimated_duration_ms: i64,
        #[serde(default)]
        estimated_cost_micros: Option<u64>,
    }
    let r: Raw = serde_json::from_value(raw.clone())
        .map_err(|e| PlanError::new("PLAN_SCHEMA", e.to_string()))?;
    let mut beats = r.beats;
    for b in &mut beats {
        b.id = slug(&b.id);
        b.script_segment = b.script_segment.take().map(|s| clip_text(&s, 600));
        for o in &mut b.overlays {
            o.text = clip_text(&o.text, 200);
        }
        for c in &mut b.captions {
            c.text = clip_text(&c.text, 200);
        }
        b.notes = b.notes.take().map(|s| clip_text(&s, 300));
    }
    let mut placeholders = Vec::new();
    for b in &beats {
        if let Some(n) = b.asset.as_ref().and_then(|a| a.need_id.clone()) {
            placeholders.push(AssetPlaceholder {
                need_id: slug(&n),
                beat_id: b.id.clone(),
            });
        }
    }
    for b in &mut beats {
        if let Some(a) = b.asset.as_mut()
            && let Some(n) = a.need_id.take()
        {
            a.need_id = Some(slug(&n));
        }
    }
    let plan = EditPlan {
        schema_version: PLAN_SCHEMA_VERSION,
        id,
        version,
        deliverable_key: deliverable_key.to_owned(),
        target_sequence: r.target_sequence,
        format: r.format,
        grammar_ref: r.grammar_ref,
        beats,
        global: r.global,
        constraints_checked: r.constraints_checked,
        asset_placeholders: placeholders,
        estimated_duration_ms: r.estimated_duration_ms,
        estimated_cost_micros: r.estimated_cost_micros,
    };
    validate_edit_plan_shape(&plan)?;
    Ok(plan)
}

/// Validação **estrutural** (sem inventário): ids, durações, formato, limites.
pub fn validate_edit_plan_shape(p: &EditPlan) -> Result<(), PlanError> {
    if !matches!(p.format.fps, 24 | 25 | 30 | 60) {
        return Err(PlanError::new("PLAN_FORMAT", "fps must be 24, 25, 30 or 60"));
    }
    if !(16..=7680).contains(&p.format.width) || !(16..=7680).contains(&p.format.height) {
        return Err(PlanError::new("PLAN_FORMAT", "frame size out of range"));
    }
    if p.beats.is_empty() || p.beats.len() > MAX_BEATS {
        return Err(PlanError::new("PLAN_BEATS", "the plan needs 1..=400 beats"));
    }
    let mut ids = BTreeSet::new();
    for b in &p.beats {
        if b.id.is_empty() || !ids.insert(b.id.clone()) {
            return Err(PlanError::new(
                "PLAN_DUPLICATE_BEAT",
                format!("beat `{}` is empty or repeated", b.id),
            ));
        }
        if b.duration_ms <= 0 || b.duration_ms > 6 * 3600 * 1000 {
            return Err(PlanError::new(
                "PLAN_DURATION",
                format!("beat `{}` has an invalid duration", b.id),
            ));
        }
        if let Some(a) = &b.asset {
            if a.asset_id.is_some() == a.need_id.is_some() {
                return Err(PlanError::new(
                    "PLAN_ASSET_SEL",
                    format!("beat `{}` must set exactly one of asset_id/need_id", b.id),
                ));
            }
            if a.source_in_ms < 0 {
                return Err(PlanError::new(
                    "PLAN_RANGE",
                    format!("beat `{}` has a negative source_in", b.id),
                ));
            }
        }
        for o in &b.overlays {
            if o.duration_ms <= 0 || o.start_offset_ms < 0 || o.start_offset_ms >= b.duration_ms {
                return Err(PlanError::new(
                    "PLAN_OVERLAY",
                    format!("beat `{}` has an overlay outside the beat", b.id),
                ));
            }
        }
        for c in &b.captions {
            if c.duration_ms <= 0 || c.start_offset_ms < 0 || c.start_offset_ms >= b.duration_ms {
                return Err(PlanError::new(
                    "PLAN_CAPTION",
                    format!("beat `{}` has a caption outside the beat", b.id),
                ));
            }
        }
        if let Some(t) = &b.transition
            && (t.duration_ms <= 0
                || t.duration_ms > b.duration_ms
                || !matches!(t.kind.as_str(), "dissolve" | "fade" | "slide_in"))
        {
            return Err(PlanError::new(
                "PLAN_TRANSITION",
                format!("beat `{}` has an invalid transition", b.id),
            ));
        }
        if let Some(f) = &b.framing
            && f.scale.is_some_and(|s| !(0.05..=20.0).contains(&s))
        {
            return Err(PlanError::new("PLAN_FRAMING", "scale out of range"));
        }
    }
    for b in &p.beats {
        for d in &b.depends_on {
            if !ids.contains(d) || d == &b.id {
                return Err(PlanError::new(
                    "PLAN_DEPENDENCY",
                    format!("beat `{}` depends on unknown/own `{d}`", b.id),
                ));
            }
        }
    }
    if p.beats.iter().all(|b| b.layer != Layer::Main) {
        return Err(PlanError::new("PLAN_BEATS", "the plan needs at least one main beat"));
    }
    Ok(())
}

/// Validação **semântica** contra o inventário e as necessidades de asset: todo asset existe e
/// está online, os trechos cabem na fonte e o plano de produção referencia só needs conhecidos.
pub fn validate_edit_plan_semantics(
    p: &EditPlan,
    inv: &Inventory,
    needs: &[AssetNeed],
) -> Vec<PlanError> {
    let mut errs = Vec::new();
    for b in &p.beats {
        let Some(a) = &b.asset else { continue };
        if let Some(id) = &a.asset_id {
            match inv.get(id) {
                None => errs.push(PlanError::new(
                    "PLAN_ASSET_UNKNOWN",
                    format!("beat `{}` uses unknown asset `{id}`", b.id),
                )),
                Some(info) => {
                    if !info.online {
                        errs.push(PlanError::new(
                            "PLAN_ASSET_OFFLINE",
                            format!("beat `{}` uses offline asset `{id}`", b.id),
                        ));
                    }
                    if let Some(d) = info.duration_ticks {
                        let end = (a.source_in_ms + b.duration_ms).saturating_mul(TICKS_PER_MS);
                        if !info.is_image && end > d {
                            errs.push(PlanError::new(
                                "PLAN_RANGE",
                                format!(
                                    "beat `{}` needs source [{}..{}] ms but the asset has {} ms",
                                    b.id,
                                    a.source_in_ms,
                                    a.source_in_ms + b.duration_ms,
                                    d / TICKS_PER_MS
                                ),
                            ));
                        }
                    }
                }
            }
        }
        if let Some(n) = &a.need_id
            && !needs.iter().any(|x| &x.id == n)
        {
            errs.push(PlanError::new(
                "PLAN_NEED_UNKNOWN",
                format!("beat `{}` references unknown asset need `{n}`", b.id),
            ));
        }
    }
    if let Some(m) = &p.global.music_asset
        && inv.get(m).is_none()
    {
        errs.push(PlanError::new(
            "PLAN_ASSET_UNKNOWN",
            format!("music asset `{m}` is unknown"),
        ));
    }
    errs
}

// ---- compilação -------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Compiled {
    pub deliverable: String,
    pub sequence_id: String,
    pub commands: Vec<Value>,
    /// Needs que ainda estão como placeholder neste plano compilado.
    pub unresolved: Vec<String>,
    pub clip_count: usize,
    pub duration_ticks: i64,
}

#[derive(Debug)]
pub struct CompileEnv<'a> {
    pub run_id: &'a str,
    /// Namespace de operation ids (`p<versão do plano>` ou `c<ciclo>` nas correções).
    pub ns: &'a str,
    pub inventory: &'a Inventory,
    /// need id → asset id (já adquiridos).
    pub resolutions: &'a BTreeMap<String, String>,
}

fn frame_ticks(fps: u32) -> i64 {
    705_600_000 / i64::from(fps.max(1))
}

fn align(ms: i64, fps: u32) -> i64 {
    let f = frame_ticks(fps);
    let t = ms.saturating_mul(TICKS_PER_MS);
    ((t + f / 2) / f).max(1) * f
}

fn align_start(ms: i64, fps: u32) -> i64 {
    let f = frame_ticks(fps);
    let t = ms.max(0).saturating_mul(TICKS_PER_MS);
    ((t + f / 2) / f) * f
}

pub fn sequence_id_for(run_id: &str, deliverable: &str) -> String {
    format!("sq_{}_{}", slug(run_id), slug(deliverable))
}

fn pos_y(position: Option<&str>, height: u32) -> f64 {
    let permille: i64 = match position {
        Some("top") => -330,
        Some("center") => 0,
        _ => 315,
    };
    ((permille * i64::from(height)) / 1000) as f64
}

/// Compila **um** `EditPlan` em comandos do Command Engine. Determinístico: mesma entrada ⇒ mesmos
/// comandos, ids e `operation_id`s (retry/resume nunca duplicam).
pub fn compile_edit_plan(env: &CompileEnv<'_>, p: &EditPlan) -> Result<Compiled, PlanError> {
    validate_edit_plan_shape(p)?;
    let sid = p
        .target_sequence
        .clone()
        .unwrap_or_else(|| sequence_id_for(env.run_id, &p.deliverable_key));
    let create = p.target_sequence.is_none();
    let fps = p.format.fps;
    let op = |kind: &str, i: usize| format!("{}:{}:{}:{kind}:{i}", env.run_id, env.ns, p.deliverable_key);
    let mut cmds: Vec<Value> = Vec::new();
    let mut n = 0usize;
    let mut push = |cmds: &mut Vec<Value>, kind: &str, v: Value| {
        let mut v = v;
        v["operation_id"] = json!(op(kind, n));
        n += 1;
        cmds.push(v);
    };
    let tid = |name: &str| format!("{sid}_{name}");
    if create {
        push(
            &mut cmds,
            "seq",
            json!({"type": "create_sequence", "id": sid, "name": format!("{} ({})", p.deliverable_key, env.run_id),
                   "frame_rate": fps.to_string(), "width": p.format.width, "height": p.format.height}),
        );
    }
    let mut tracks_made: BTreeSet<&str> = BTreeSet::new();
    let mut ensure_track =
        |cmds: &mut Vec<Value>, push: &mut dyn FnMut(&mut Vec<Value>, &str, Value), name: &'static str, kind: &str, role: &str, label: &str| {
            if create && tracks_made.insert(name) {
                push(
                    cmds,
                    "track",
                    json!({"type": "add_track", "sequence": sid, "id": tid(name), "kind": kind, "role": role, "name": label}),
                );
            }
        };
    let mut unresolved = Vec::new();
    let mut clip_count = 0usize;
    let mut cursor_ms: i64 = 0;
    let mut last_main_start_ms: i64 = 0;
    let mut text_clip_idx = 0usize;
    let mut pushf = |cmds: &mut Vec<Value>, kind: &str, v: Value| push(cmds, kind, v);
    for b in &p.beats {
        let (start_ms, track_name, track_label) = match b.layer {
            Layer::Main => {
                last_main_start_ms = cursor_ms;
                (cursor_ms, "V1", "Main")
            }
            Layer::Overlay => (b.at_ms.unwrap_or(last_main_start_ms), "V2", "Overlay"),
        };
        let start = align_start(start_ms, fps);
        let dur = align(b.duration_ms, fps);
        let clip_id = format!("{}_{}", tid("c"), b.id);
        // resolve a fonte
        let (content, audio_only, is_placeholder, name) = match &b.asset {
            None => (
                json!({"type": "solid", "color": "#000000"}),
                false,
                false,
                b.role.clone(),
            ),
            Some(sel) => {
                let asset_id = sel.asset_id.clone().or_else(|| {
                    sel.need_id
                        .as_ref()
                        .and_then(|n| env.resolutions.get(n).cloned())
                });
                match asset_id {
                    Some(id) => {
                        let info = env.inventory.get(&id);
                        let (hv, ha) = info.map_or((true, true), |i| (i.has_video, i.has_audio));
                        let image = info.is_some_and(|i| i.is_image);
                        if image {
                            (json!({"type": "image", "asset": id}), false, false, b.role.clone())
                        } else if !hv && ha {
                            (
                                json!({"type": "media", "asset": id, "has_video": false, "has_audio": true}),
                                true,
                                false,
                                b.role.clone(),
                            )
                        } else {
                            // b-roll em overlay não traz o áudio da fonte
                            let audio = ha && b.layer == Layer::Main;
                            (
                                json!({"type": "media", "asset": id, "has_video": hv, "has_audio": audio}),
                                false,
                                false,
                                b.role.clone(),
                            )
                        }
                    }
                    None => {
                        let need = sel.need_id.clone().unwrap_or_default();
                        unresolved.push(need.clone());
                        (
                            json!({"type": "solid", "color": "#2B2B2B"}),
                            false,
                            true,
                            format!("PLACEHOLDER:{need}"),
                        )
                    }
                }
            }
        };
        let (kind, role, label, tname): (&str, &str, &str, &'static str) = if audio_only {
            ("audio", "voice", "Voice", "A1")
        } else if b.layer == Layer::Main {
            ("visual", "main", track_label, "V1")
        } else {
            ("visual", "overlay", track_label, "V2")
        };
        let _ = track_name;
        ensure_track(&mut cmds, &mut pushf, tname, kind, role, label);
        let source_in = b
            .asset
            .as_ref()
            .map_or(0, |a| align_start(a.source_in_ms, fps));
        let source_in = if is_placeholder || matches!(content["type"].as_str(), Some("solid" | "image")) {
            0
        } else {
            source_in
        };
        pushf(
            &mut cmds,
            "clip",
            json!({"type": "insert_clip", "track": tid(tname), "start": start,
                   "clip": {"id": clip_id, "name": name, "duration": dur, "content": content,
                            "source_in": source_in, "speed": "1", "reversed": false, "properties": {}}}),
        );
        clip_count += 1;
        if let Some(t) = &b.transition {
            pushf(
                &mut cmds,
                "trans",
                json!({"type": "set_transition", "clip": clip_id,
                       "transition": {"kind": t.kind, "duration": align(t.duration_ms, fps)}}),
            );
        }
        if let Some(f) = &b.framing {
            for (prop, val) in [
                ("scale", f.scale),
                ("position_x", f.position_x),
                ("position_y", f.position_y),
            ] {
                if let Some(v) = val {
                    pushf(
                        &mut cmds,
                        "prop",
                        json!({"type": "set_property", "clip": clip_id, "prop": prop, "value": v}),
                    );
                }
            }
        }
        let size = p.global.text_size_permille.unwrap_or(55).clamp(10, 500);
        for o in &b.overlays {
            ensure_track(&mut cmds, &mut pushf, "TX", "visual", "text", "Text");
            let id = format!("{}_{}", tid("t"), text_clip_idx);
            text_clip_idx += 1;
            pushf(
                &mut cmds,
                "text",
                json!({"type": "insert_clip", "track": tid("TX"),
                       "start": align_start(start_ms + o.start_offset_ms, fps),
                       "clip": {"id": id, "name": "", "duration": align(o.duration_ms.min(b.duration_ms - o.start_offset_ms), fps),
                                "content": {"type": "text", "text": o.text,
                                            "style": {"size_permille": size, "weight": 800, "color": "#FFFFFF", "background": "#000000B3"}},
                                "source_in": 0, "speed": "1", "reversed": false, "properties": {}}}),
            );
            clip_count += 1;
            pushf(
                &mut cmds,
                "textpos",
                json!({"type": "set_property", "clip": id, "prop": "position_y",
                       "value": pos_y(o.position.as_deref(), p.format.height)}),
            );
        }
        for c in &b.captions {
            ensure_track(&mut cmds, &mut pushf, "CP", "visual", "captions", "Captions");
            let id = format!("{}_{}", tid("k"), text_clip_idx);
            text_clip_idx += 1;
            pushf(
                &mut cmds,
                "cap",
                json!({"type": "insert_clip", "track": tid("CP"),
                       "start": align_start(start_ms + c.start_offset_ms, fps),
                       "clip": {"id": id, "name": "", "duration": align(c.duration_ms.min(b.duration_ms - c.start_offset_ms), fps),
                                "content": {"type": "text", "text": c.text,
                                            "style": {"size_permille": size, "weight": 700, "color": "#FFFFFF", "background": "#000000B3"}},
                                "source_in": 0, "speed": "1", "reversed": false, "properties": {}}}),
            );
            clip_count += 1;
            pushf(
                &mut cmds,
                "cappos",
                json!({"type": "set_property", "clip": id, "prop": "position_y",
                       "value": pos_y(None, p.format.height)}),
            );
        }
        if b.layer == Layer::Main {
            cursor_ms += b.duration_ms;
        }
    }
    let total_ticks = align(cursor_ms.max(1), fps);
    if let Some(m) = &p.global.music_asset {
        let info = env.inventory.get(m);
        let max = info
            .and_then(|i| i.duration_ticks)
            .map_or(total_ticks, |d| d.min(total_ticks));
        let dur = (max / frame_ticks(fps)).max(1) * frame_ticks(fps);
        ensure_track(&mut cmds, &mut pushf, "A2", "audio", "music", "Music");
        let id = format!("{}_music", tid("c"));
        pushf(
            &mut cmds,
            "music",
            json!({"type": "insert_clip", "track": tid("A2"), "start": 0,
                   "clip": {"id": id, "name": "music", "duration": dur,
                            "content": {"type": "media", "asset": m, "has_video": false, "has_audio": true},
                            "source_in": 0, "speed": "1", "reversed": false, "properties": {}}}),
        );
        clip_count += 1;
        if let Some(db) = p.global.music_volume_db {
            pushf(
                &mut cmds,
                "musicvol",
                json!({"type": "set_property", "clip": id, "prop": "volume_db", "value": db}),
            );
        }
    }
    unresolved.sort();
    unresolved.dedup();
    Ok(Compiled {
        deliverable: p.deliverable_key.clone(),
        sequence_id: sid,
        commands: cmds,
        unresolved,
        clip_count,
        duration_ticks: total_ticks,
    })
}

/// Compila **toda** a produção. Deliverables com master (`hook_plus_master`/`shared_master`) entram
/// na **mesma** transação do master (o preview executa sobre cópia de trabalho, então o nested pode
/// referenciar a sequence ainda não aplicada) e recebem um `insert_nested` do master ao final.
pub fn compile_production(
    env: &CompileEnv<'_>,
    plan: &ProductionPlan,
    edits: &[EditPlan],
) -> Result<Vec<TxUnit>, PlanError> {
    let mut compiled: BTreeMap<String, Compiled> = BTreeMap::new();
    let mut units: Vec<TxUnit> = Vec::new();
    let order = plan.execution_order();
    for key in &order {
        let Some(ep) = edits.iter().find(|e| &e.deliverable_key == key) else {
            return Err(PlanError::new(
                "PLAN_EDIT_MISSING",
                format!("deliverable `{key}` has no EditPlan"),
            ));
        };
        let d = plan.deliverable(key);
        let mut c = compile_edit_plan(env, ep)?;
        let master = d.and_then(|d| d.master.clone());
        if let Some(mk) = &master {
            let Some(m) = compiled.get(mk) else {
                return Err(PlanError::new(
                    "PLAN_MASTER_ORDER",
                    format!("master `{mk}` must be compiled before `{key}`"),
                ));
            };
            let fps = ep.format.fps;
            let hook_end = c.duration_ticks;
            let track = format!("{}_VM", c.sequence_id);
            let n = c.commands.len();
            c.commands.push(json!({
                "operation_id": format!("{}:{}:{}:mastertrack:{n}", env.run_id, env.ns, key),
                "type": "add_track", "sequence": c.sequence_id, "id": track, "kind": "visual", "role": "main", "name": "Master"
            }));
            c.commands.push(json!({
                "operation_id": format!("{}:{}:{}:master:{}", env.run_id, env.ns, key, n + 1),
                "type": "insert_nested", "track": track, "start": hook_end.div_euclid(frame_ticks(fps)) * frame_ticks(fps),
                "sequence": m.sequence_id, "id": format!("{}_master", c.sequence_id),
                "name": format!("master:{mk}"), "follow_length": true
            }));
            c.clip_count += 1;
            c.duration_ticks += m.duration_ticks;
        }
        compiled.insert(key.clone(), c.clone());
        // mesma transação do master quando houver
        match master.and_then(|mk| units.iter().position(|u| u.deliverables.contains(&mk))) {
            Some(i) => {
                units[i].deliverables.push(key.clone());
                units[i].commands.extend(c.commands.clone());
                units[i].unresolved.extend(c.unresolved.clone());
                units[i].sequences.push((key.clone(), c.sequence_id.clone()));
            }
            None => units.push(TxUnit {
                deliverables: vec![key.clone()],
                commands: c.commands.clone(),
                unresolved: c.unresolved.clone(),
                sequences: vec![(key.clone(), c.sequence_id.clone())],
            }),
        }
    }
    for u in &mut units {
        u.unresolved.sort();
        u.unresolved.dedup();
    }
    Ok(units)
}

/// Uma transação atômica de produção (um ou mais deliverables).
#[derive(Clone, Debug)]
pub struct TxUnit {
    pub deliverables: Vec<String>,
    pub commands: Vec<Value>,
    pub unresolved: Vec<String>,
    pub sequences: Vec<(String, String)>,
}

impl TxUnit {
    pub fn key(&self) -> String {
        self.deliverables.join("+")
    }

    pub fn digest(&self) -> String {
        digest_value(&Value::Array(self.commands.clone()))
    }
}

// ---- relatório de validação -------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitValidation {
    pub unit: String,
    pub deliverables: Vec<String>,
    pub command_count: usize,
    pub op_count: usize,
    pub base_revision: u64,
    pub diff_digest: String,
    pub plan_digest: String,
    /// `plan_token` do `preview` — só vale enquanto o engine o guardar; o EDIT sempre refaz o
    /// preview e compara o `diff_digest` antes de aplicar.
    pub plan_token: Option<String>,
    pub already_applied: bool,
    pub destructive_ops: u32,
    pub sequences: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostEstimate {
    pub llm_micros: u64,
    pub acquisition_micros: u64,
    pub generation_micros: u64,
    pub total_micros: u64,
    /// Há preço desconhecido (custo desconhecido ≠ zero).
    pub has_unknown: bool,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub plan_id: String,
    pub valid: bool,
    pub plan_digest: String,
    pub diff_digest: String,
    pub target_revision: u64,
    pub command_count: usize,
    pub sequences_affected: Vec<String>,
    pub assets_missing: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub estimated_cost: CostEstimate,
    pub approval_required: bool,
    /// `placeholders` ainda presentes no plano compilado.
    pub placeholder_state: Vec<String>,
    pub units: Vec<UnitValidation>,
}

/// Quantas operações destrutivas (remoções) há nos comandos.
pub fn destructive_count(cmds: &[Value]) -> u32 {
    cmds.iter()
        .filter(|c| {
            matches!(
                c["type"].as_str(),
                Some("delete_clip" | "delete_track" | "delete_sequence" | "flatten_nested")
            )
        })
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv() -> Inventory {
        let mut i = Inventory::default();
        i.assets.insert(
            "raw".into(),
            AssetInfo {
                id: "raw".into(),
                name: "raw".into(),
                duration_ticks: Some(60_000 * TICKS_PER_MS),
                has_video: true,
                has_audio: true,
                online: true,
                is_image: false,
            },
        );
        i.assets.insert(
            "music".into(),
            AssetInfo {
                id: "music".into(),
                name: "m".into(),
                duration_ticks: Some(10_000 * TICKS_PER_MS),
                has_video: false,
                has_audio: true,
                online: true,
                is_image: false,
            },
        );
        i
    }

    fn edit(beats: Value) -> EditPlan {
        edit_plan_from(
            &json!({"format": {"width": 1080, "height": 1920, "fps": 30}, "beats": beats,
                    "estimated_duration_ms": 8000,
                    "global": {"music_asset": "music", "music_volume_db": -18.0}}),
            "e1".into(),
            1,
            "main",
        )
        .unwrap()
    }

    fn beats() -> Value {
        json!([
            {"id": "hook", "role": "hook", "duration_ms": 3000, "asset": {"asset_id": "raw", "source_in_ms": 0},
             "overlays": [{"text": "Olha isso", "start_offset_ms": 0, "duration_ms": 1500}],
             "captions": [{"text": "Olá pessoal", "start_offset_ms": 0, "duration_ms": 1200}]},
            {"id": "body", "role": "body", "duration_ms": 5000, "asset": {"need_id": "broll1", "source_in_ms": 0},
             "transition": {"kind": "fade", "duration_ms": 400}, "framing": {"scale": 1.1}},
            {"id": "cta", "role": "cta", "duration_ms": 2000, "asset": {"asset_id": "raw", "source_in_ms": 4000}}
        ])
    }

    #[test]
    fn compile_is_deterministic_and_marks_placeholders() {
        let inv = inv();
        let res = BTreeMap::new();
        let env = CompileEnv { run_id: "run-1", ns: "p1", inventory: &inv, resolutions: &res };
        let ep = edit(beats());
        let a = compile_edit_plan(&env, &ep).unwrap();
        let b = compile_edit_plan(&env, &ep).unwrap();
        assert_eq!(a.commands, b.commands);
        assert_eq!(a.unresolved, vec!["broll1".to_owned()]);
        assert!(a.commands.iter().any(|c| c["clip"]["name"] == "PLACEHOLDER:broll1"));
        // todos os operation_ids são únicos e prefixados pela Run/namespace
        let ids: BTreeSet<_> = a.commands.iter().map(|c| c["operation_id"].as_str().unwrap().to_owned()).collect();
        assert_eq!(ids.len(), a.commands.len());
        assert!(ids.iter().all(|i| i.starts_with("run-1:p1:main:")));
        assert_eq!(a.duration_ticks, 10_000 * TICKS_PER_MS);
    }

    #[test]
    fn resolving_a_need_replaces_the_placeholder_with_media() {
        let mut inv = inv();
        inv.assets.insert(
            "stock".into(),
            AssetInfo { id: "stock".into(), name: "s".into(), duration_ticks: Some(20_000 * TICKS_PER_MS),
                        has_video: true, has_audio: true, online: true, is_image: false },
        );
        let mut res = BTreeMap::new();
        res.insert("broll1".to_owned(), "stock".to_owned());
        let env = CompileEnv { run_id: "run-1", ns: "p2", inventory: &inv, resolutions: &res };
        let c = compile_edit_plan(&env, &edit(beats())).unwrap();
        assert!(c.unresolved.is_empty());
        let clip = c.commands.iter().find(|c| c["clip"]["id"].as_str().is_some_and(|i| i.ends_with("_body"))).unwrap();
        assert_eq!(clip["clip"]["content"]["asset"], "stock");
        // namespace novo ⇒ operation ids novos (nova versão do plano)
        assert!(c.commands[0]["operation_id"].as_str().unwrap().contains(":p2:"));
    }

    #[test]
    fn semantic_validation_catches_unknown_offline_and_out_of_range() {
        let mut i = inv();
        let ep = edit(json!([{"id": "a", "role": "hook", "duration_ms": 3000, "asset": {"asset_id": "ghost", "source_in_ms": 0}},
                              {"id": "b", "role": "body", "duration_ms": 5000, "asset": {"asset_id": "raw", "source_in_ms": 58000}}]));
        let errs = validate_edit_plan_semantics(&ep, &i, &[]);
        let codes: Vec<_> = errs.iter().map(|e| e.code).collect();
        assert!(codes.contains(&"PLAN_ASSET_UNKNOWN"), "{codes:?}");
        assert!(codes.contains(&"PLAN_RANGE"), "{codes:?}");
        i.assets.get_mut("raw").unwrap().online = false;
        let errs = validate_edit_plan_semantics(&ep, &i, &[]);
        assert!(errs.iter().any(|e| e.code == "PLAN_ASSET_OFFLINE"));
    }

    #[test]
    fn malformed_plans_are_rejected_not_partially_accepted() {
        let bad = [
            json!({"format": {"width": 1080, "height": 1920, "fps": 31}, "beats": [{"id": "a", "role": "x", "duration_ms": 1000}], "estimated_duration_ms": 1000}),
            json!({"format": {"width": 1080, "height": 1920}, "beats": [], "estimated_duration_ms": 0}),
            json!({"format": {"width": 1080, "height": 1920}, "beats": [{"id": "a", "role": "x", "duration_ms": -5}], "estimated_duration_ms": 1}),
            json!({"format": {"width": 1080, "height": 1920}, "beats": [{"id": "a", "role": "x", "duration_ms": 1000}, {"id": "a", "role": "x", "duration_ms": 1000}], "estimated_duration_ms": 2000}),
            json!({"format": {"width": 1080, "height": 1920}, "beats": [{"id": "a", "role": "x", "duration_ms": 1000, "asset": {"asset_id": "r", "need_id": "n"}}], "estimated_duration_ms": 1000}),
            json!({"format": {"width": 1080, "height": 1920}, "beats": [{"id": "a", "role": "x", "duration_ms": 1000, "overlays": [{"text": "t", "start_offset_ms": 5000, "duration_ms": 10}]}], "estimated_duration_ms": 1000}),
        ];
        for b in bad {
            assert!(edit_plan_from(&b, "e".into(), 1, "m").is_err(), "{b}");
        }
        assert!(production_plan_from(&json!({"strategy": "s", "deliverables": [], "asset_needs": []}), "p".into(), 1, None).is_err());
        let cyc = json!({"strategy": "s", "asset_needs": [], "deliverables": [
            {"key": "a", "sequence_strategy": "shared_master", "master": "b"},
            {"key": "b", "sequence_strategy": "shared_master", "master": "a"}]});
        assert_eq!(production_plan_from(&cyc, "p".into(), 1, None).unwrap_err().code, "PLAN_MASTER_CYCLE");
    }

    #[test]
    fn hook_plus_master_shares_one_transaction_and_nests_the_master() {
        let inv = inv();
        let res = BTreeMap::new();
        let pp = production_plan_from(
            &json!({"strategy": "hooks", "asset_needs": [], "deliverables": [
                {"key": "body_master", "sequence_strategy": "standalone"},
                {"key": "hook_a", "sequence_strategy": "hook_plus_master", "master": "body_master"},
                {"key": "hook_b", "sequence_strategy": "hook_plus_master", "master": "body_master"}]}),
            "pp".into(), 1, None,
        ).unwrap();
        assert_eq!(pp.execution_order()[0], "body_master");
        let mk = |k: &str, id: &str| {
            edit_plan_from(
                &json!({"format": {"width": 1080, "height": 1920}, "estimated_duration_ms": 3000,
                        "beats": [{"id": "b1", "role": "hook", "duration_ms": 3000, "asset": {"asset_id": "raw", "source_in_ms": 0}}]}),
                id.into(), 1, k,
            ).unwrap()
        };
        let edits = vec![mk("hook_a", "e2"), mk("body_master", "e1"), mk("hook_b", "e3")];
        let env = CompileEnv { run_id: "run-1", ns: "p1", inventory: &inv, resolutions: &res };
        let units = compile_production(&env, &pp, &edits).unwrap();
        assert_eq!(units.len(), 1, "shared master ⇒ one atomic transaction");
        assert_eq!(units[0].deliverables.len(), 3);
        let nested: Vec<_> = units[0].commands.iter().filter(|c| c["type"] == "insert_nested").collect();
        assert_eq!(nested.len(), 2);
        assert!(nested.iter().all(|c| c["follow_length"] == true));
        let master_seq = sequence_id_for("run-1", "body_master");
        assert!(nested.iter().all(|c| c["sequence"] == master_seq));
        // ids distintos por variante
        let seqs: BTreeSet<_> = units[0].sequences.iter().map(|s| s.1.clone()).collect();
        assert_eq!(seqs.len(), 3);
    }

    #[test]
    fn independent_deliverables_get_independent_transactions() {
        let inv = inv();
        let res = BTreeMap::new();
        let pp = production_plan_from(
            &json!({"strategy": "s", "asset_needs": [], "deliverables": [
                {"key": "a", "sequence_strategy": "standalone"}, {"key": "b", "sequence_strategy": "format_variant", "variant_of": "a"}]}),
            "pp".into(), 1, None,
        ).unwrap();
        let mk = |k: &str| edit_plan_from(
            &json!({"format": {"width": 1080, "height": 1920}, "estimated_duration_ms": 2000,
                    "beats": [{"id": "b1", "role": "x", "duration_ms": 2000, "asset": {"asset_id": "raw", "source_in_ms": 0}}]}),
            format!("e_{k}"), 1, k).unwrap();
        let env = CompileEnv { run_id: "r", ns: "p1", inventory: &inv, resolutions: &res };
        let units = compile_production(&env, &pp, &[mk("a"), mk("b")]).unwrap();
        assert_eq!(units.len(), 2);
        let missing = compile_production(&env, &pp, &[mk("a")]).unwrap_err();
        assert_eq!(missing.code, "PLAN_EDIT_MISSING");
    }
}
