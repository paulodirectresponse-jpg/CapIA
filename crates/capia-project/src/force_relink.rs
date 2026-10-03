//! *Force relink* (ADR-058): trocar o arquivo de um asset por **outro conteúdo**, de forma
//! explícita. Diferente do relink por conteúdo (que só aceita o mesmo hash), aqui o hash muda:
//! o arquivo novo é analisado (probe + SHA-256 + impressão), os clips dependentes são validados
//! pelo engine (`update_asset`: nada de trim silencioso — conflito estruturado) e os derivados do
//! conteúdo antigo (índice, waveform, proxy) são invalidados. O `AssetId` **não** muda.

use crate::assets::doc_asset;
use crate::error::ProjectError;
use crate::project::{Project, now_ms};
use capia_assets::{AssetError, AssetErrorCode, GcReport, prepare_import};
use capia_commands::{Actor, Command, CommandEnvelope, CommitResult, Transaction};
use capia_media::MediaProbe;
use capia_model::AssetId;
use capia_store::{CatalogEventKind, CatalogOp};
use serde::Serialize;
use serde_json::json;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct DependentClip {
    pub sequence: String,
    pub clip: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ForceRelinkResult {
    pub asset_id: AssetId,
    pub old_hash: String,
    pub new_hash: String,
    /// `true` ⇒ o arquivo tinha o MESMO conteúdo: foi só um relink comum.
    pub same_content: bool,
    pub from: String,
    pub to: String,
    pub old_duration_ticks: Option<i64>,
    pub new_duration_ticks: Option<i64>,
    /// Clips do documento que usam o asset (todos revalidados contra a nova mídia).
    pub dependents: Vec<DependentClip>,
    /// `true` em `dry_run`: nada foi gravado.
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<CommitResult>,
    pub caches_invalidated: GcReport,
}

impl Project {
    /// Clips do documento que referenciam `id`.
    pub fn dependents_of(&self, id: &AssetId) -> Vec<DependentClip> {
        self.document()
            .sequences()
            .flat_map(|(sid, s)| {
                s.clips()
                    .filter(|c| c.content.asset() == Some(id))
                    .map(move |c| DependentClip {
                        sequence: sid.to_string(),
                        clip: c.id.to_string(),
                    })
            })
            .collect()
    }

    /// Troca o arquivo de `id` por `new_file` (conteúdo diferente permitido). `dry_run` só
    /// analisa: devolve o mesmo resultado/conflitos sem gravar nada.
    pub fn force_relink_asset(
        &mut self,
        actor: &Actor,
        id: &AssetId,
        new_file: &Path,
        probe: &dyn MediaProbe,
        dry_run: bool,
    ) -> Result<ForceRelinkResult, ProjectError> {
        let old = self.managed(id)?;
        let now = now_ms();
        let dir = self.project_dir();
        let prepared = prepare_import(new_file, dir.as_deref(), probe, now)?;
        let fresh = prepared.record;
        let dependents = self.dependents_of(id);
        let from = old.location.path.clone();
        let to = fresh.location.path.clone();
        let mut result = ForceRelinkResult {
            asset_id: id.clone(),
            old_hash: old.content_hash.to_string(),
            new_hash: fresh.content_hash.to_string(),
            same_content: fresh.content_hash == old.content_hash,
            from,
            to,
            old_duration_ticks: old.media.duration.map(|d| d.0),
            new_duration_ticks: fresh.media.duration.map(|d| d.0),
            dependents,
            dry_run,
            commit: None,
            caches_invalidated: GcReport::default(),
        };
        if result.same_content {
            if !dry_run {
                self.apply_relink_unchecked(id, &prepared.path, "force")?;
            }
            return Ok(result);
        }
        // outro asset já tem esse conteúdo: forçar criaria duas identidades para o mesmo hash
        if let Some(other) = self.catalog().find_by_hash(&fresh.content_hash)?
            && other.asset_id != *id
        {
            return Err(ProjectError::Asset(
                AssetError::new(
                    AssetErrorCode::AssetPathInvalid,
                    format!(
                        "the new file already is asset {} in this project; refusing to give one content two identities",
                        other.asset_id
                    ),
                )
                .with_details(json!({
                    "code": "FORCE_RELINK_DUPLICATE_CONTENT",
                    "asset_id": id.as_str(),
                    "existing_asset_id": other.asset_id.as_str(),
                    "hash": fresh.content_hash.as_str(),
                })),
            ));
        }
        // registro novo do catálogo: MESMO id, conteúdo/metadados do arquivo novo; os aliases
        // antigos descrevem outro conteúdo e não valem mais
        let mut rec = fresh.clone();
        rec.asset_id = id.clone();
        rec.imported_ms = old.imported_ms;
        rec.known_paths = Vec::new();
        rec.display_name = old.display_name.clone();
        // lado do documento: `update_asset` valida TODOS os clips dependentes
        let in_doc = self.document().asset(id).cloned();
        let mut new_doc = doc_asset(&rec);
        if let Some(cur) = &in_doc {
            new_doc.name = cur.name.clone();
        }
        let revision = self.document().revision;
        let tx = Transaction {
            transaction_id: None,
            label: format!("force relink {}", rec.display_name),
            base_revision: None,
            commands: vec![CommandEnvelope {
                operation_id: format!(
                    "force-relink:{id}:r{revision}:{}",
                    &fresh.content_hash.hex()[..16]
                ),
                reference: None,
                command: Command::UpdateAsset { asset: new_doc },
            }],
            max_ops: None,
        };
        let op = CatalogOp {
            record: rec,
            event: CatalogEventKind::ForceRelink,
            detail: json!({
                "from": result.from,
                "to": result.to,
                "old_hash": result.old_hash,
                "new_hash": result.new_hash,
                "dependents": result.dependents.len(),
            }),
            at_ms: now,
        };
        if dry_run {
            if in_doc.is_some() {
                // o engine valida os clips (conflitos viram `CONFLICT` estruturado) sem gravar
                self.preview(actor, tx)?;
            }
            return Ok(result);
        }
        if in_doc.is_some() {
            // documento + catálogo na MESMA transação SQLite
            self.queue_catalog(op)?;
            let committed = self.execute(actor, tx);
            self.clear_catalog_queue();
            result.commit = Some(committed?);
        } else {
            self.catalog_mut().apply(&op)?;
        }
        result.caches_invalidated = self.invalidate_derived(&old.content_hash)?;
        Ok(result)
    }
}
