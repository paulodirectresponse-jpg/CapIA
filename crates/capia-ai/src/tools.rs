//! Tool System (AI_SYSTEM §5): registry versionado, schemas, permissões, efeitos colaterais,
//! gate de autorização, limites de saída, `operation_id` determinístico e trilha de auditoria.
//! Aqui só há **contrato e política**; os executores moram em `capia-intelligence`, que fala com o
//! engine por uma fachada. **Não existem** tools de shell, filesystem, HTTP genérico, configurações
//! ou segredos — e o registry recusa registrá-las (por construção, não por prompt).

use crate::types::{ToolCallOut, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Permission {
    ReadProject,
    ReadMedia,
    WriteTimeline,
    ManageAssets,
    NetworkFetch,
    SpendMoney,
    RunHeavyCompute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SideEffect {
    None,
    Document,
    Network,
    Spend,
    LocalCompute,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub version: u32,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub permission: Permission,
    pub side_effect: SideEffect,
    pub timeout_ms: u64,
    pub idempotent: bool,
    pub cost_hint: Option<String>,
}

/// Prefixos/nomes que **nunca** viram tool (a lista é parte do contrato de segurança).
const FORBIDDEN_PREFIXES: &[&str] = &[
    "shell",
    "exec",
    "process",
    "os.",
    "fs.",
    "file.",
    "files.",
    "path.",
    "http.",
    "https.",
    "fetch.",
    "curl",
    "net.",
    "network.",
    "socket",
    "settings.",
    "config.",
    "secrets.",
    "secret.",
    "credentials.",
    "credential.",
    "env.",
    "keychain",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ToolErrorCode {
    UnknownTool,
    NotAllowedForTask,
    RoleDenied,
    PermissionDenied,
    InvalidArguments,
    InvalidResult,
    SideEffectNotApproved,
    BudgetExceeded,
    PrivacyBlocked,
    Timeout,
    Failed,
}

impl ToolError {
    pub fn new(code: ToolErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: capia_secrets::redact(&message.into()),
        }
    }
}

impl core::fmt::Display for ToolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ToolError {}

#[derive(Clone, Debug, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, ToolDef>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, def: ToolDef) -> Result<(), String> {
        let n = &def.name;
        if n.is_empty()
            || n.len() > 64
            || !n
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.'))
        {
            return Err(format!("invalid tool name `{n}`"));
        }
        let lower = n.to_ascii_lowercase();
        if FORBIDDEN_PREFIXES
            .iter()
            .any(|p| lower == p.trim_end_matches('.') || lower.starts_with(p))
        {
            return Err(format!(
                "tool `{n}` is forbidden: the AI never gets shell, filesystem, generic network, settings or secrets tools"
            ));
        }
        jsonschema::validator_for(&def.input_schema)
            .map_err(|e| format!("tool `{n}` input schema invalid: {e}"))?;
        jsonschema::validator_for(&def.output_schema)
            .map_err(|e| format!("tool `{n}` output schema invalid: {e}"))?;
        if self.tools.contains_key(n) {
            return Err(format!("tool `{n}` already registered"));
        }
        self.tools.insert(def.name.clone(), def);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&ToolDef> {
        self.tools.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ToolDef> {
        self.tools.values()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Specs entregues ao modelo: só o que a política permite (o modelo não "vê" o resto).
    pub fn specs_for(&self, policy: &ToolPolicy) -> Vec<ToolSpec> {
        self.tools
            .values()
            .filter(|t| policy.visible(t))
            .map(|t| ToolSpec {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
            })
            .collect()
    }

    /// Cadeia de verificação completa antes de executar (PHASE4 §15), na ordem da spec.
    pub fn authorize<'a>(
        &'a self,
        policy: &ToolPolicy,
        call: &ToolCallOut,
        gates: Gates,
    ) -> Result<&'a ToolDef, ToolError> {
        let def = self.tools.get(&call.name).ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::UnknownTool,
                format!("tool `{}` does not exist", call.name),
            )
        })?;
        // 1) a tarefa/stage permite esta tool?
        if !policy.allowed_tools.contains(&def.name) {
            return Err(ToolError::new(
                ToolErrorCode::NotAllowedForTask,
                format!("tool `{}` is not available in this task", def.name),
            ));
        }
        // 2) o papel permite?
        if let Some(role_set) = &policy.role_allowed
            && !role_set.contains(&def.name)
        {
            return Err(ToolError::new(
                ToolErrorCode::RoleDenied,
                format!("the current role cannot use `{}`", def.name),
            ));
        }
        // 3) permissão concedida?
        if !policy.granted.contains(&def.permission) {
            return Err(ToolError::new(
                ToolErrorCode::PermissionDenied,
                format!("permission {:?} is not granted", def.permission),
            ));
        }
        // 4) argumentos válidos?
        validate_against(
            &def.input_schema,
            &call.arguments,
            ToolErrorCode::InvalidArguments,
        )?;
        // 5) orçamento/privacidade
        if !gates.budget_ok {
            return Err(ToolError::new(
                ToolErrorCode::BudgetExceeded,
                "task budget does not allow this tool",
            ));
        }
        if !gates.privacy_ok {
            return Err(ToolError::new(
                ToolErrorCode::PrivacyBlocked,
                "privacy policy does not allow this tool",
            ));
        }
        // 6) efeito colateral aprovado quando necessário
        if def.side_effect != SideEffect::None
            && !policy.approved_side_effects.contains(&def.side_effect)
        {
            return Err(ToolError::new(
                ToolErrorCode::SideEffectNotApproved,
                format!("side effect {:?} requires approval", def.side_effect),
            ));
        }
        Ok(def)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Gates {
    pub budget_ok: bool,
    pub privacy_ok: bool,
}

impl Default for Gates {
    fn default() -> Self {
        Self {
            budget_ok: true,
            privacy_ok: true,
        }
    }
}

/// Política efetiva de uma tarefa.
#[derive(Clone, Debug, Default)]
pub struct ToolPolicy {
    pub allowed_tools: BTreeSet<String>,
    pub role_allowed: Option<BTreeSet<String>>,
    pub granted: BTreeSet<Permission>,
    pub approved_side_effects: BTreeSet<SideEffect>,
}

impl ToolPolicy {
    fn visible(&self, t: &ToolDef) -> bool {
        self.allowed_tools.contains(&t.name)
            && self
                .role_allowed
                .as_ref()
                .is_none_or(|r| r.contains(&t.name))
            && self.granted.contains(&t.permission)
    }

    /// Política só-leitura: tools de leitura/análise local; **sem** escrita, rede nem gasto.
    pub fn read_only(reg: &ToolRegistry) -> Self {
        let granted: BTreeSet<Permission> = [
            Permission::ReadProject,
            Permission::ReadMedia,
            Permission::RunHeavyCompute,
        ]
        .into_iter()
        .collect();
        Self {
            allowed_tools: reg
                .iter()
                .filter(|t| {
                    granted.contains(&t.permission) && t.side_effect != SideEffect::Document
                })
                .map(|t| t.name.clone())
                .collect(),
            role_allowed: None,
            granted,
            approved_side_effects: [SideEffect::LocalCompute].into_iter().collect(),
        }
    }

    /// Política de escrita pontual: leitura + `timeline.preview/apply_plan` (apply só aprovado).
    pub fn point_write(reg: &ToolRegistry, approve_apply: bool) -> Self {
        let mut p = Self::read_only(reg);
        p.granted.insert(Permission::WriteTimeline);
        for t in reg
            .iter()
            .filter(|t| t.permission == Permission::WriteTimeline)
        {
            p.allowed_tools.insert(t.name.clone());
        }
        if approve_apply {
            p.approved_side_effects.insert(SideEffect::Document);
        }
        p
    }
}

pub fn validate_against(schema: &Value, v: &Value, code: ToolErrorCode) -> Result<(), ToolError> {
    crate::dispatcher::validate(v, schema).map_err(|m| ToolError::new(code, m))
}

pub fn validate_result(def: &ToolDef, v: &Value) -> Result<(), ToolError> {
    validate_against(&def.output_schema, v, ToolErrorCode::InvalidResult)
}

/// `operation_id` determinístico por `task + step + índice` (retry não duplica edição; ADR-029).
pub fn operation_id(task_id: &str, step: u32, index: u32) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"capia-op/v1\0");
    h.update(task_id.as_bytes());
    h.update([0]);
    h.update(step.to_be_bytes());
    h.update(index.to_be_bytes());
    format!("op_{}", &crate::types::hex(&h.finalize())[..24])
}

pub const MAX_ARRAY_ITEMS: usize = 50;
pub const MAX_STRING_CHARS: usize = 2_000;

/// Limita a saída de uma tool que vai ao modelo: arrays, strings e tamanho total. `true` ⇒ truncou.
pub fn bound_output(v: &Value, max_bytes: usize) -> (Value, bool) {
    fn go(v: &Value, truncated: &mut bool) -> Value {
        match v {
            Value::Array(a) => {
                if a.len() > MAX_ARRAY_ITEMS {
                    *truncated = true;
                }
                let mut items: Vec<Value> = a
                    .iter()
                    .take(MAX_ARRAY_ITEMS)
                    .map(|x| go(x, truncated))
                    .collect();
                if a.len() > MAX_ARRAY_ITEMS {
                    items.push(json!({"_truncated": true, "omitted": a.len() - MAX_ARRAY_ITEMS}));
                }
                Value::Array(items)
            }
            Value::Object(m) => Value::Object(
                m.iter()
                    .map(|(k, x)| (k.clone(), go(x, truncated)))
                    .collect(),
            ),
            Value::String(s) if s.chars().count() > MAX_STRING_CHARS => {
                *truncated = true;
                let mut t: String = s.chars().take(MAX_STRING_CHARS).collect();
                t.push('…');
                Value::String(t)
            }
            other => other.clone(),
        }
    }
    let mut truncated = false;
    let out = go(v, &mut truncated);
    let s = out.to_string();
    if s.len() > max_bytes {
        let cut: String = s.chars().take(max_bytes.saturating_sub(64)).collect();
        return (json!({"_truncated": true, "partial_json": cut}), true);
    }
    (out, truncated)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub task_id: String,
    pub step: u32,
    pub tool: String,
    pub version: u32,
    pub permission: Option<Permission>,
    pub side_effect: Option<SideEffect>,
    pub status: String,
    pub error: Option<String>,
    pub input_digest: String,
    pub output_bytes: usize,
    pub started_ms: u64,
    pub finished_ms: u64,
    pub transaction_ref: Option<String>,
}

pub fn input_digest(v: &Value) -> String {
    use sha2::{Digest, Sha256};
    crate::types::hex(&Sha256::digest(crate::types::canonical_json(v).as_bytes()))
}

/// Catálogo da Fase 4 (nomes estáveis de `AI_SYSTEM.md` §5 + ferramentas de análise que produzem
/// comandos). Só contratos; executores em `capia-intelligence`.
pub fn phase4_catalog() -> Vec<ToolDef> {
    fn obj(props: Value, required: &[&str]) -> Value {
        json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
    }
    fn any_obj() -> Value {
        json!({"type": "object"})
    }
    #[allow(clippy::too_many_arguments)]
    fn t(
        name: &str,
        desc: &str,
        input: Value,
        perm: Permission,
        effect: SideEffect,
        timeout_ms: u64,
        idempotent: bool,
        cost: Option<&str>,
    ) -> ToolDef {
        ToolDef {
            name: name.into(),
            version: 1,
            description: desc.into(),
            input_schema: input,
            output_schema: any_obj(),
            permission: perm,
            side_effect: effect,
            timeout_ms,
            idempotent,
            cost_hint: cost.map(str::to_owned),
        }
    }
    let seq = json!({"type": "string", "description": "sequence id"});
    let asset = json!({"type": "string", "description": "asset id"});
    let secs = json!({"type": "number", "minimum": 0});
    vec![
        t(
            "project.read_brief",
            "Summary of the open project: name, sequences, asset counts, offline media.",
            obj(json!({}), &[]),
            Permission::ReadProject,
            SideEffect::None,
            5_000,
            true,
            None,
        ),
        t(
            "project.list_sequences",
            "List the project's sequences with duration and format.",
            obj(json!({}), &[]),
            Permission::ReadProject,
            SideEffect::None,
            5_000,
            true,
            None,
        ),
        t(
            "timeline.get_state",
            "Paginated digest of a sequence (times in seconds and frames). Use detail=outline first.",
            obj(
                json!({"sequence": seq, "detail": {"type": "string", "enum": ["outline", "clips", "full"]},
                       "from_s": secs, "to_s": secs, "page": {"type": "integer", "minimum": 0},
                       "page_size": {"type": "integer", "minimum": 1, "maximum": 200}}),
                &["sequence"],
            ),
            Permission::ReadProject,
            SideEffect::None,
            10_000,
            true,
            None,
        ),
        t(
            "timeline.query_clips",
            "Find clips by track, text, asset or time range.",
            obj(
                json!({"sequence": seq, "track": {"type": "string"}, "text": {"type": "string", "maxLength": 200}, "asset": asset,
                       "from_s": secs, "to_s": secs, "limit": {"type": "integer", "minimum": 1, "maximum": 200}}),
                &["sequence"],
            ),
            Permission::ReadProject,
            SideEffect::None,
            10_000,
            true,
            None,
        ),
        t(
            "timeline.preview",
            "Prepare a batch of engine commands WITHOUT applying: returns a plan_token and a diff summary.",
            obj(
                json!({"label": {"type": "string", "maxLength": 120},
                       "commands": {"type": "array", "minItems": 1, "maxItems": 500, "items": {"type": "object"}}}),
                &["label", "commands"],
            ),
            Permission::WriteTimeline,
            SideEffect::None,
            15_000,
            false,
            None,
        ),
        t(
            "timeline.apply_plan",
            "Apply a previously previewed plan by its token. The document changes only here (undoable).",
            obj(
                json!({"plan_token": {"type": "string", "minLength": 8, "maxLength": 512}}),
                &["plan_token"],
            ),
            Permission::WriteTimeline,
            SideEffect::Document,
            15_000,
            true,
            None,
        ),
        t(
            "assets.search",
            "Search project assets by name/kind.",
            obj(
                json!({"query": {"type": "string", "maxLength": 200}, "kind": {"type": "string", "enum": ["video", "audio", "image"]}, "limit": {"type": "integer", "minimum": 1, "maximum": 100}}),
                &[],
            ),
            Permission::ReadProject,
            SideEffect::None,
            10_000,
            true,
            None,
        ),
        t(
            "assets.get",
            "Details of one asset (metadata, availability).",
            obj(json!({"asset": asset}), &["asset"]),
            Permission::ReadProject,
            SideEffect::None,
            5_000,
            true,
            None,
        ),
        t(
            "assets.get_transcript",
            "Return the cached transcript of an asset (does not run transcription).",
            obj(
                json!({"asset": asset, "from_s": secs, "to_s": secs}),
                &["asset"],
            ),
            Permission::ReadProject,
            SideEffect::None,
            5_000,
            true,
            None,
        ),
        t(
            "media.analyze",
            "Deterministic local analysis of a media asset (duration, scenes, speech regions, silence).",
            obj(json!({"asset": asset}), &["asset"]),
            Permission::RunHeavyCompute,
            SideEffect::LocalCompute,
            300_000,
            true,
            Some("local compute"),
        ),
        t(
            "media.sample_frames",
            "Sample up to 16 reduced frames at deterministic timestamps (never uploads video).",
            obj(
                json!({"asset": asset, "count": {"type": "integer", "minimum": 1, "maximum": 16}, "width": {"type": "integer", "minimum": 64, "maximum": 512}, "from_s": secs, "to_s": secs, "contact_sheet": {"type": "boolean"}}),
                &["asset", "count"],
            ),
            Permission::ReadMedia,
            SideEffect::LocalCompute,
            60_000,
            true,
            None,
        ),
        t(
            "media.transcribe",
            "Transcribe an asset (cached by content hash + provider + params). May use a cloud provider only if privacy allows.",
            obj(
                json!({"asset": asset, "language": {"type": "string", "maxLength": 12}, "word_timestamps": {"type": "boolean"}}),
                &["asset"],
            ),
            Permission::RunHeavyCompute,
            SideEffect::LocalCompute,
            900_000,
            true,
            Some("provider cost if cloud"),
        ),
        t(
            "media.detect_scenes",
            "Detect shot boundaries locally.",
            obj(json!({"asset": asset}), &["asset"]),
            Permission::RunHeavyCompute,
            SideEffect::LocalCompute,
            300_000,
            true,
            Some("local compute"),
        ),
        t(
            "reference.analyze",
            "Run the Reference Analyzer on a reference video and store its ReferenceGrammar.",
            obj(json!({"asset": asset}), &["asset"]),
            Permission::RunHeavyCompute,
            SideEffect::LocalCompute,
            900_000,
            true,
            Some("local + optional vision"),
        ),
        t(
            "reference.get_grammar",
            "Return the stored ReferenceGrammar of a reference asset.",
            obj(json!({"asset": asset}), &["asset"]),
            Permission::ReadProject,
            SideEffect::None,
            5_000,
            true,
            None,
        ),
        t(
            "render.frame",
            "Render one frame of a sequence (reduced size) for inspection.",
            obj(
                json!({"sequence": seq, "at_s": secs, "width": {"type": "integer", "minimum": 64, "maximum": 512}}),
                &["sequence", "at_s"],
            ),
            Permission::ReadMedia,
            SideEffect::None,
            30_000,
            true,
            None,
        ),
        t(
            "captions.build_commands",
            "Build the engine commands that create editable caption clips from a transcript (does not apply).",
            obj(
                json!({"sequence": seq, "asset": asset, "style": {"type": "string", "maxLength": 40}, "max_chars_per_block": {"type": "integer", "minimum": 8, "maximum": 80}, "max_lines": {"type": "integer", "minimum": 1, "maximum": 3}}),
                &["sequence", "asset"],
            ),
            Permission::ReadProject,
            SideEffect::None,
            30_000,
            true,
            None,
        ),
        t(
            "silence.propose_cuts",
            "Detect silences in a clip's audio and propose ripple cuts as engine commands (does not apply).",
            obj(
                json!({"sequence": seq, "clip": {"type": "string"}, "min_silence_ms": {"type": "integer", "minimum": 100, "maximum": 5000}, "threshold_db": {"type": "number", "minimum": -80, "maximum": -10}, "padding_ms": {"type": "integer", "minimum": 0, "maximum": 500}}),
                &["sequence", "clip"],
            ),
            Permission::RunHeavyCompute,
            SideEffect::LocalCompute,
            120_000,
            true,
            None,
        ),
        t(
            "project.get_demand_spec",
            "Return the latest DemandSpec stored in the project.",
            obj(json!({}), &[]),
            Permission::ReadProject,
            SideEffect::None,
            5_000,
            true,
            None,
        ),
    ]
}

pub fn phase4_registry() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    for t in phase4_catalog() {
        // o catálogo é estático e testado: um erro aqui é bug de desenvolvimento
        if let Err(e) = r.register(t) {
            debug_assert!(false, "{e}");
        }
    }
    r
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn call(name: &str, args: Value) -> ToolCallOut {
        ToolCallOut {
            id: "c".into(),
            name: name.into(),
            arguments: args,
        }
    }

    #[test]
    fn catalog_registers_and_has_no_forbidden_tools() {
        let r = phase4_registry();
        assert_eq!(r.len(), phase4_catalog().len());
        for must in [
            "timeline.preview",
            "timeline.apply_plan",
            "media.transcribe",
            "reference.analyze",
            "render.frame",
        ] {
            assert!(r.get(must).is_some(), "{must}");
        }
        for t in r.iter() {
            assert!(
                !t.name.starts_with("shell")
                    && !t.name.starts_with("fs.")
                    && !t.name.starts_with("http.")
            );
            assert_ne!(
                t.permission,
                Permission::NetworkFetch,
                "Fase 4 não expõe rede ao modelo: {}",
                t.name
            );
            assert_ne!(
                t.permission,
                Permission::SpendMoney,
                "Fase 4 não expõe gasto ao modelo: {}",
                t.name
            );
        }
    }

    #[test]
    fn registry_refuses_shell_fs_http_settings_secrets() {
        let mut r = ToolRegistry::new();
        for bad in [
            "shell.run",
            "exec",
            "fs.read",
            "file.write",
            "http.get",
            "fetch.url",
            "settings.get",
            "secrets.read",
            "credentials.list",
            "env.get",
            "net.connect",
        ] {
            let d = ToolDef {
                name: bad.into(),
                version: 1,
                description: String::new(),
                input_schema: json!({"type": "object"}),
                output_schema: json!({"type": "object"}),
                permission: Permission::ReadProject,
                side_effect: SideEffect::None,
                timeout_ms: 1,
                idempotent: true,
                cost_hint: None,
            };
            assert!(r.register(d).is_err(), "{bad} deveria ser recusada");
        }
    }

    #[test]
    fn gate_order_unknown_task_role_permission_args_budget_privacy_effect() {
        let reg = phase4_registry();
        let ro = ToolPolicy::read_only(&reg);
        let g = Gates::default();
        assert_eq!(
            reg.authorize(&ro, &call("nope.x", json!({})), g)
                .unwrap_err()
                .code,
            ToolErrorCode::UnknownTool
        );
        // escrita não está na política só-leitura
        assert_eq!(
            reg.authorize(
                &ro,
                &call("timeline.apply_plan", json!({"plan_token": "abcdefgh"})),
                g
            )
            .unwrap_err()
            .code,
            ToolErrorCode::NotAllowedForTask
        );
        // papel restringe
        let mut role = ro.clone();
        role.role_allowed = Some(["project.read_brief".to_owned()].into_iter().collect());
        assert_eq!(
            reg.authorize(&role, &call("assets.get", json!({"asset": "a"})), g)
                .unwrap_err()
                .code,
            ToolErrorCode::RoleDenied
        );
        // permissão não concedida
        let mut noperm = ro.clone();
        noperm.granted.remove(&Permission::ReadProject);
        assert_eq!(
            reg.authorize(&noperm, &call("assets.get", json!({"asset": "a"})), g)
                .unwrap_err()
                .code,
            ToolErrorCode::PermissionDenied
        );
        // argumentos inválidos (campo extra + tipo errado)
        assert_eq!(
            reg.authorize(&ro, &call("assets.get", json!({"asset": 5})), g)
                .unwrap_err()
                .code,
            ToolErrorCode::InvalidArguments
        );
        assert_eq!(
            reg.authorize(
                &ro,
                &call("assets.get", json!({"asset": "a", "path": "/etc/passwd"})),
                g
            )
            .unwrap_err()
            .code,
            ToolErrorCode::InvalidArguments
        );
        // orçamento/privacidade
        assert_eq!(
            reg.authorize(
                &ro,
                &call("assets.get", json!({"asset": "a"})),
                Gates {
                    budget_ok: false,
                    privacy_ok: true
                }
            )
            .unwrap_err()
            .code,
            ToolErrorCode::BudgetExceeded
        );
        assert_eq!(
            reg.authorize(
                &ro,
                &call("assets.get", json!({"asset": "a"})),
                Gates {
                    budget_ok: true,
                    privacy_ok: false
                }
            )
            .unwrap_err()
            .code,
            ToolErrorCode::PrivacyBlocked
        );
        // escrita: apply exige efeito aprovado
        let w = ToolPolicy::point_write(&reg, false);
        assert_eq!(
            reg.authorize(
                &w,
                &call("timeline.apply_plan", json!({"plan_token": "abcdefgh"})),
                g
            )
            .unwrap_err()
            .code,
            ToolErrorCode::SideEffectNotApproved
        );
        let w = ToolPolicy::point_write(&reg, true);
        assert!(
            reg.authorize(
                &w,
                &call("timeline.apply_plan", json!({"plan_token": "abcdefgh"})),
                g
            )
            .is_ok()
        );
        // preview não tem efeito colateral
        assert!(
            reg.authorize(
                &ToolPolicy::point_write(&reg, false),
                &call(
                    "timeline.preview",
                    json!({"label": "x", "commands": [{"a": 1}]})
                ),
                g
            )
            .is_ok()
        );
    }

    #[test]
    fn model_only_sees_allowed_tools() {
        let reg = phase4_registry();
        let ro = reg.specs_for(&ToolPolicy::read_only(&reg));
        assert!(
            ro.iter()
                .all(|s| s.name != "timeline.apply_plan" && s.name != "timeline.preview")
        );
        let w = reg.specs_for(&ToolPolicy::point_write(&reg, false));
        assert!(w.iter().any(|s| s.name == "timeline.preview"));
    }

    #[test]
    fn operation_ids_are_deterministic_and_distinct() {
        assert_eq!(operation_id("t1", 2, 0), operation_id("t1", 2, 0));
        assert_ne!(operation_id("t1", 2, 0), operation_id("t1", 2, 1));
        assert_ne!(operation_id("t1", 2, 0), operation_id("t2", 2, 0));
        assert!(operation_id("t", 0, 0).starts_with("op_"));
    }

    #[test]
    fn output_is_bounded() {
        let big = json!({"items": (0..500).collect::<Vec<_>>(), "s": "x".repeat(10_000)});
        let (v, trunc) = bound_output(&big, 100_000);
        assert!(trunc);
        assert_eq!(v["items"].as_array().unwrap().len(), MAX_ARRAY_ITEMS + 1);
        assert!(v["s"].as_str().unwrap().chars().count() <= MAX_STRING_CHARS + 1);
        let (v, trunc) = bound_output(&big, 300);
        assert!(trunc && v["_truncated"] == json!(true));
        let (small, t) = bound_output(&json!({"a": 1}), 1000);
        assert!(!t && small["a"] == 1);
    }
}
