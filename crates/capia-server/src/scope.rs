//! Scopes least-privilege (PHASE6_COMPLETION §12). Cada operação do catálogo declara exatamente um.

use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    ProjectRead,
    ProjectWrite,
    MediaRead,
    MediaWrite,
    RunRead,
    RunStart,
    RunApprove,
    ExportRead,
    ExportStart,
    WebhookManage,
    AdminTokens,
}

pub const ALL_SCOPES: [Scope; 11] = [
    Scope::ProjectRead,
    Scope::ProjectWrite,
    Scope::MediaRead,
    Scope::MediaWrite,
    Scope::RunRead,
    Scope::RunStart,
    Scope::RunApprove,
    Scope::ExportRead,
    Scope::ExportStart,
    Scope::WebhookManage,
    Scope::AdminTokens,
];

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectRead => "project:read",
            Self::ProjectWrite => "project:write",
            Self::MediaRead => "media:read",
            Self::MediaWrite => "media:write",
            Self::RunRead => "run:read",
            Self::RunStart => "run:start",
            Self::RunApprove => "run:approve",
            Self::ExportRead => "export:read",
            Self::ExportStart => "export:start",
            Self::WebhookManage => "webhook:manage",
            Self::AdminTokens => "admin:tokens",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        ALL_SCOPES.into_iter().find(|x| x.as_str() == s)
    }
}

/// Lista de scopes validada: sem desconhecidos, sem duplicata, não vazia.
pub fn parse_scopes(raw: &[String]) -> Result<BTreeSet<Scope>, String> {
    if raw.is_empty() {
        return Err("a token needs at least one scope".into());
    }
    let mut out = BTreeSet::new();
    for r in raw {
        let s = Scope::parse(r).ok_or_else(|| format!("unknown scope `{r}`"))?;
        if !out.insert(s) {
            return Err(format!("duplicate scope `{r}`"));
        }
    }
    Ok(out)
}
