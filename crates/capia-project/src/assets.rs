//! Assets do projeto: import atômico, listagem, verify, relink e miniaturas (ADR-046..049).
//!
//! O **documento** guarda só o asset lógico (id/nome/duração/flags — o que a edição precisa); o
//! **catálogo** (arquivo, hash, metadados, disponibilidade) vive nas tabelas do `.capia` e **não**
//! entra no undo. O import grava as duas camadas na MESMA transação SQLite.

use crate::error::ProjectError;
use crate::project::Project;
use capia_assets::{
    AssetLocation, AssetRecord, Availability, CacheDir, ContentHash, asset_id_for, check_relink,
    ensure_thumbnail, prepare_import, quick_status, verify_content,
};
use capia_commands::{Actor, Command, CommandEnvelope, CommitResult, Transaction};
use capia_media::{MediaProbe, MediaToolchain};
use capia_model::{Asset, AssetId};
use capia_store::{CatalogEventKind, CatalogOp};
use capia_time::Ticks;
use serde::Serialize;
use serde_json::json;
use std::path::{Path, PathBuf};

/// Máximo de aliases (caminhos alternativos) guardados por asset.
const MAX_KNOWN_PATHS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportOutcome {
    /// Conteúdo novo: asset criado no catálogo e no documento.
    Created,
    /// Conteúdo já conhecido pelo catálogo, mas o asset tinha saído do documento: registrado de novo.
    Reregistered,
    /// O documento já tinha o id (registrado "à mão"): só o catálogo foi preenchido.
    Adopted,
    /// Mesmo conteúdo, mesmo caminho já conhecido: nada mudou.
    Existing,
    /// Mesmo conteúdo, caminho novo: guardado como alias.
    Aliased,
    /// O asset estava offline/modificado e o novo caminho tem o conteúdo certo: relink automático.
    Relinked,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImportResult {
    pub asset_id: AssetId,
    pub outcome: ImportOutcome,
    pub record: AssetRecord,
    /// Presente quando o documento mudou (um commit do engine).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<CommitResult>,
}

/// Visão de um asset: documento ∪ catálogo, com a disponibilidade **calculada agora**.
#[derive(Clone, Debug, Serialize)]
pub struct AssetView {
    pub asset_id: AssetId,
    pub in_document: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<Asset>,
    /// `None` = asset lógico sem arquivo (registrado por `register_asset`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog: Option<AssetRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct VerifyResult {
    pub asset_id: AssetId,
    pub status: Availability,
    pub expected_hash: ContentHash,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_hash: Option<ContentHash>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RelinkResult {
    pub asset_id: AssetId,
    pub from: String,
    pub to: String,
    pub record: AssetRecord,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

pub(crate) fn doc_asset(rec: &AssetRecord) -> Asset {
    let m = &rec.media;
    Asset {
        id: rec.asset_id.clone(),
        name: rec.display_name.clone(),
        duration: m.duration,
        has_video: m.has_video(),
        has_audio: m.has_audio(),
        offline: false,
    }
}

pub(crate) fn push_alias(known: &mut Vec<String>, path: &str) {
    if !known.iter().any(|k| k == path) {
        known.push(path.to_owned());
        if known.len() > MAX_KNOWN_PATHS {
            known.remove(0);
        }
    }
}

impl Project {
    pub(crate) fn project_dir(&self) -> Option<PathBuf> {
        std::path::absolute(self.path())
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
    }

    /// Diretório de cache padrão deste projeto (descartável; ADR-049).
    pub fn cache_dir(&self) -> CacheDir {
        CacheDir::beside_project(self.path())
    }

    /// Importa um arquivo de mídia (ADR-046): verificar → hash → probe → registrar. Tudo antes da
    /// primeira escrita; probe falho não deixa nada registrado. Idempotente por **conteúdo**.
    pub fn import_asset(
        &mut self,
        actor: &Actor,
        path: &Path,
        probe: &dyn MediaProbe,
    ) -> Result<ImportResult, ProjectError> {
        let now = now_ms();
        let dir = self.project_dir();
        let prepared = prepare_import(path, dir.as_deref(), probe, now)?;
        self.commit_prepared(actor, prepared.record, now)
    }

    /// Segunda metade do import: classifica o registro **já preparado** (hash+probe feitos, de
    /// forma síncrona ou por job) contra o catálogo/documento e grava. Sem IO de mídia.
    pub(crate) fn commit_prepared(
        &mut self,
        actor: &Actor,
        mut rec: AssetRecord,
        now: u64,
    ) -> Result<ImportResult, ProjectError> {
        let dir = self.project_dir();
        let new_path = rec.location.path.clone();
        let existing = self.catalog().find_by_hash(&rec.content_hash)?;
        let in_doc = self.document().asset(&rec.asset_id).is_some();

        match (existing, in_doc) {
            (None, false) => {
                let commit = self.register_with_catalog(
                    actor,
                    &rec,
                    CatalogOp {
                        record: rec.clone(),
                        event: CatalogEventKind::Import,
                        detail: json!({ "path": new_path }),
                        at_ms: now,
                    },
                )?;
                Ok(ImportResult {
                    asset_id: rec.asset_id.clone(),
                    outcome: ImportOutcome::Created,
                    record: rec,
                    commit: Some(commit),
                })
            }
            (None, true) => {
                self.catalog_mut().apply(&CatalogOp {
                    record: rec.clone(),
                    event: CatalogEventKind::Import,
                    detail: json!({ "path": new_path, "adopted": true }),
                    at_ms: now,
                })?;
                Ok(ImportResult {
                    asset_id: rec.asset_id.clone(),
                    outcome: ImportOutcome::Adopted,
                    record: rec,
                    commit: None,
                })
            }
            (Some(old), false) => {
                // reaproveita metadados já normalizados; só a localização é a do import atual
                rec.known_paths = old.known_paths.clone();
                push_alias(&mut rec.known_paths, &old.location.path);
                rec.media = old.media.clone();
                rec.imported_ms = old.imported_ms;
                let commit = self.register_with_catalog(
                    actor,
                    &rec,
                    CatalogOp {
                        record: rec.clone(),
                        event: CatalogEventKind::Reimport,
                        detail: json!({ "path": new_path }),
                        at_ms: now,
                    },
                )?;
                Ok(ImportResult {
                    asset_id: rec.asset_id.clone(),
                    outcome: ImportOutcome::Reregistered,
                    record: rec,
                    commit: Some(commit),
                })
            }
            (Some(mut old), true) => {
                let known_here =
                    old.location.path == new_path || old.known_paths.contains(&new_path);
                let (status, _) = quick_status(&old, dir.as_deref());
                if status == Availability::Online {
                    if known_here {
                        old.status = Availability::Online;
                        return Ok(ImportResult {
                            asset_id: old.asset_id.clone(),
                            outcome: ImportOutcome::Existing,
                            record: old,
                            commit: None,
                        });
                    }
                    push_alias(&mut old.known_paths, &new_path);
                    old.status_checked_ms = now;
                    self.catalog_mut().apply(&CatalogOp {
                        record: old.clone(),
                        event: CatalogEventKind::Alias,
                        detail: json!({ "path": new_path }),
                        at_ms: now,
                    })?;
                    return Ok(ImportResult {
                        asset_id: old.asset_id.clone(),
                        outcome: ImportOutcome::Aliased,
                        record: old,
                        commit: None,
                    });
                }
                // offline/modificado e o conteúdo novo é idêntico (mesmo hash) ⇒ relink automático
                let from = old.location.path.clone();
                push_alias(&mut old.known_paths, &from);
                old.location = rec.location.clone();
                old.status = Availability::Online;
                old.status_checked_ms = now;
                self.catalog_mut().apply(&CatalogOp {
                    record: old.clone(),
                    event: CatalogEventKind::Relink,
                    detail: json!({ "from": from, "to": new_path, "auto": true }),
                    at_ms: now,
                })?;
                Ok(ImportResult {
                    asset_id: old.asset_id.clone(),
                    outcome: ImportOutcome::Relinked,
                    record: old,
                    commit: None,
                })
            }
        }
    }

    /// `register_asset` no engine **com** o efeito do catálogo na mesma transação SQLite.
    fn register_with_catalog(
        &mut self,
        actor: &Actor,
        rec: &AssetRecord,
        op: CatalogOp,
    ) -> Result<CommitResult, ProjectError> {
        let revision = self.document().revision;
        let tx = Transaction {
            transaction_id: None,
            label: format!("import {}", rec.display_name),
            base_revision: None,
            commands: vec![CommandEnvelope {
                operation_id: format!("import:{}:r{revision}", rec.asset_id),
                reference: None,
                command: Command::RegisterAsset {
                    asset: doc_asset(rec),
                },
            }],
            max_ops: None,
        };
        self.queue_catalog(op)?;
        let result = self.execute(actor, tx);
        // a fila nunca sobrevive à chamada: sucesso (drenada pelo journal) ou falha (descartada)
        self.clear_catalog_queue();
        Ok(result?)
    }

    /// Documento ∪ catálogo, com a disponibilidade calculada agora (só `metadata`, sem hash).
    pub fn assets(&self) -> Result<Vec<AssetView>, ProjectError> {
        let dir = self.project_dir();
        let mut views: std::collections::BTreeMap<AssetId, AssetView> = self
            .document()
            .assets()
            .map(|a| {
                (
                    a.id.clone(),
                    AssetView {
                        asset_id: a.id.clone(),
                        in_document: true,
                        document: Some(a.clone()),
                        catalog: None,
                        resolved_path: None,
                    },
                )
            })
            .collect();
        for mut rec in self.catalog().list()? {
            let (status, found) = quick_status(&rec, dir.as_deref());
            rec.status = status;
            let view = views
                .entry(rec.asset_id.clone())
                .or_insert_with(|| AssetView {
                    asset_id: rec.asset_id.clone(),
                    in_document: false,
                    document: None,
                    catalog: None,
                    resolved_path: None,
                });
            view.resolved_path = found.map(|p| p.display().to_string());
            view.catalog = Some(rec);
        }
        Ok(views.into_values().collect())
    }

    /// Um asset: O(1) (leitura direta da linha do catálogo + `metadata` do arquivo), sem montar a
    /// lista inteira.
    pub fn asset(&self, id: &AssetId) -> Result<AssetView, ProjectError> {
        let document = self.document().asset(id).cloned();
        let mut catalog = self.catalog().get(id)?;
        if document.is_none() && catalog.is_none() {
            return Err(ProjectError::invalid(
                "ASSET_NOT_FOUND",
                format!("asset {id} does not exist"),
            ));
        }
        let mut resolved_path = None;
        if let Some(rec) = catalog.as_mut() {
            let (status, found) = quick_status(rec, self.project_dir().as_deref());
            rec.status = status;
            resolved_path = found.map(|p| p.display().to_string());
        }
        Ok(AssetView {
            asset_id: id.clone(),
            in_document: document.is_some(),
            document,
            catalog,
            resolved_path,
        })
    }

    /// Procura pelo **conteúdo** (hash) no catálogo; `None` se o projeto nunca viu esse conteúdo.
    pub fn find_asset_by_hash(
        &self,
        hash: &ContentHash,
    ) -> Result<Option<AssetView>, ProjectError> {
        match self.catalog().find_by_hash(hash)? {
            Some(rec) => Ok(Some(self.asset(&rec.asset_id)?)),
            None => Ok(None),
        }
    }

    pub(crate) fn managed(&self, id: &AssetId) -> Result<AssetRecord, ProjectError> {
        match self.catalog().get(id)? {
            Some(r) => Ok(r),
            None if self.document().asset(id).is_some() => Err(ProjectError::invalid(
                "ASSET_NOT_MANAGED",
                format!("asset {id} has no media file in the catalog (logical asset)"),
            )),
            None => Err(ProjectError::invalid(
                "ASSET_NOT_FOUND",
                format!("asset {id} does not exist"),
            )),
        }
    }

    /// Verificação **completa** (recalcula o hash) e atualiza o estado conhecido no catálogo.
    pub fn verify_asset(&mut self, id: &AssetId) -> Result<VerifyResult, ProjectError> {
        let mut rec = self.managed(id)?;
        let dir = self.project_dir();
        let report = verify_content(&rec, dir.as_deref())?;
        let now = now_ms();
        let actual = report.actual.as_ref().map(|d| d.hash.clone());
        let result = VerifyResult {
            asset_id: id.clone(),
            status: report.status,
            expected_hash: rec.content_hash.clone(),
            actual_hash: actual.clone(),
            path: report.path.as_ref().map(|p| p.display().to_string()),
        };
        rec.status = report.status;
        rec.status_checked_ms = now;
        self.catalog_mut().apply(&CatalogOp {
            record: rec,
            event: CatalogEventKind::Verify,
            detail: json!({
                "status": report.status.as_str(),
                "actual_hash": actual.as_ref().map(ContentHash::as_str),
            }),
            at_ms: now,
        })?;
        Ok(result)
    }

    /// Relink por conteúdo (ADR-048 §5): só aceita arquivo com o **mesmo hash**; senão
    /// `ASSET_HASH_MISMATCH` estruturado. Não é um comando do engine (não entra no undo).
    pub fn relink_asset(
        &mut self,
        id: &AssetId,
        new_file: &Path,
    ) -> Result<RelinkResult, ProjectError> {
        let rec = self.managed(id)?;
        let check = check_relink(&rec, new_file)?;
        self.apply_relink_unchecked(id, &check.path, "manual")
    }

    /// Grava o relink (localização nova, online, alias do caminho antigo) **sem** reconferir o
    /// conteúdo: o chamador já provou o hash (relink manual, em lote ou import).
    pub(crate) fn apply_relink_unchecked(
        &mut self,
        id: &AssetId,
        new_abs: &Path,
        mode: &str,
    ) -> Result<RelinkResult, ProjectError> {
        let mut rec = self.managed(id)?;
        let dir = self.project_dir();
        let from = rec.location.path.clone();
        let location = AssetLocation::from_path(new_abs, dir.as_deref());
        let to = location.path.clone();
        push_alias(&mut rec.known_paths, &from);
        rec.location = location;
        rec.status = Availability::Online;
        rec.status_checked_ms = now_ms();
        self.catalog_mut().apply(&CatalogOp {
            record: rec.clone(),
            event: CatalogEventKind::Relink,
            detail: json!({ "from": from, "to": to, "auto": false, "mode": mode }),
            at_ms: rec.status_checked_ms,
        })?;
        Ok(RelinkResult {
            asset_id: id.clone(),
            from,
            to,
            record: rec,
        })
    }

    /// Histórico de eventos de catálogo do asset (import, alias, relink, verify…).
    pub fn asset_events(
        &self,
        id: &AssetId,
    ) -> Result<Vec<capia_store::CatalogEvent>, ProjectError> {
        Ok(self.catalog().events(id)?)
    }

    /// Miniatura de um frame (ADR-049): vai para o cache e é regenerável.
    pub fn generate_thumbnail(
        &self,
        id: &AssetId,
        at: Ticks,
        max_dim: u32,
        toolchain: &MediaToolchain,
    ) -> Result<PathBuf, ProjectError> {
        let rec = self.managed(id)?;
        let dir = self.project_dir();
        let (status, found) = quick_status(&rec, dir.as_deref());
        let Some(file) = found.filter(|_| status == Availability::Online) else {
            return Err(ProjectError::Asset(capia_assets::AssetError::new(
                capia_assets::AssetErrorCode::AssetOffline,
                format!("asset {id} is {}: no file to read", status.as_str()),
            )));
        };
        Ok(ensure_thumbnail(
            toolchain,
            &rec,
            &file,
            at,
            max_dim,
            &self.cache_dir(),
        )?)
    }
}

/// Atalho útil aos adaptadores: id que o import atribuirá a um conteúdo.
pub fn expected_asset_id(hash: &ContentHash) -> AssetId {
    asset_id_for(hash)
}
