//! Vocabulário de comandos (docs/COMMAND_SYSTEM.md §2): estável, versionado, serializável.
//! UI, IA (tools), CLI e API usam exatamente estes tipos.

use capia_model::{
    Asset, AssetId, ClipContent, ClipId, Interp, MarkerId, PropertySet, SequenceId, TrackId,
    TrackKind, TrackRole,
};
use capia_time::{FrameRate, Rational, Ticks};
use serde::{Deserialize, Serialize};

/// Versão do contrato de comandos (upcasters de comandos antigos, COMMAND_SYSTEM.md §10).
pub const COMMAND_SCHEMA_VERSION: u32 = 1;

fn one() -> Rational {
    Rational::ONE
}

/// Borda de um clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    In,
    Out,
}

/// Quais tracks participam de um ripple (D-S7-3). Só tracks do escopo são deslocadas; tracks
/// travadas ou fora do escopo ficam intactas; se as invariantes não puderem ser preservadas,
/// `RIPPLE_CONFLICT` estruturado.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RippleScope {
    /// Só a track do clip (padrão).
    #[default]
    Track,
    /// Tracks explícitas, além da track do clip.
    Tracks { tracks: Vec<TrackId> },
    /// Tracks com o mesmo rótulo de grupo que a track do clip (respeita `sync_lock`).
    Group,
    /// Todas as tracks com `sync_lock` ligado.
    Sequence,
}

/// Dados de um clip a inserir.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewClip {
    /// Se ausente, o id é derivado deterministicamente do `operation_id`.
    #[serde(default)]
    pub id: Option<ClipId>,
    #[serde(default)]
    pub name: String,
    pub duration: Ticks,
    pub content: ClipContent,
    #[serde(default)]
    pub source_in: Ticks,
    #[serde(default = "one")]
    pub speed: Rational,
    #[serde(default)]
    pub reversed: bool,
    #[serde(default)]
    pub properties: PropertySet,
}

/// Uma variante de `generate_variants`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantSpec {
    #[serde(default)]
    pub sequence: Option<SequenceId>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deep: bool,
    /// Os `clip` das trocas são ids do **template**.
    #[serde(default)]
    pub swaps: Vec<VariantSwap>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VariantSwap {
    SetNested { clip: ClipId, sequence: SequenceId },
    ReplaceMedia { clip: ClipId, asset: AssetId },
}

/// Um movimento de clip (estrito: sem clamp; ver `resolve_group_move` para o cálculo de UX).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipMove {
    pub clip: ClipId,
    /// Track de destino (omitido = mesma track).
    #[serde(default)]
    pub track: Option<TrackId>,
    pub start: Ticks,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    // --- projeto / sequence / track / marcador / asset -------------------------------------
    RegisterAsset {
        asset: Asset,
    },
    /// Remove o asset **lógico** do documento. `IN_USE` se algum clip o referencia (ADR-048 §7). O
    /// registro do catálogo de mídia permanece (biblioteca): reimportar o mesmo conteúdo o reaproveita.
    DeleteAsset {
        asset: AssetId,
    },
    /// Atualiza os metadados **lógicos** de um asset existente (duração, vídeo/áudio, nome) — o
    /// documento-lado do *force relink* (ADR-058). Valida **todos** os clips dependentes: o trecho
    /// de fonte que consomem tem de caber na nova duração e as faixas que usam têm de existir.
    /// Nenhum trim silencioso: ou tudo continua válido ou `CONFLICT` com a lista de clips.
    UpdateAsset {
        asset: Asset,
    },
    CreateSequence {
        #[serde(default)]
        id: Option<SequenceId>,
        name: String,
        frame_rate: FrameRate,
        #[serde(default)]
        sample_rate: Option<u32>,
    },
    AddTrack {
        sequence: SequenceId,
        #[serde(default)]
        id: Option<TrackId>,
        kind: TrackKind,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        role: Option<TrackRole>,
        #[serde(default)]
        magnetic: bool,
        /// Posição na pilha (omitido = no fim).
        #[serde(default)]
        index: Option<usize>,
    },
    SetTrackFlags {
        track: TrackId,
        #[serde(default)]
        locked: Option<bool>,
        #[serde(default)]
        hidden: Option<bool>,
        #[serde(default)]
        muted: Option<bool>,
        #[serde(default)]
        solo: Option<bool>,
        #[serde(default)]
        magnetic: Option<bool>,
        #[serde(default)]
        sync_lock: Option<bool>,
        #[serde(default)]
        group: Option<String>,
        /// Remove a track de qualquer grupo (ignorado se `group` foi dado).
        #[serde(default)]
        clear_group: bool,
        /// Ao ligar `magnetic` numa track com gaps: fecha os gaps em vez de rejeitar.
        #[serde(default)]
        compact: bool,
    },
    DeleteTrack {
        track: TrackId,
    },
    /// Apaga a sequence (e seu conteúdo). Recusa com `IN_USE` se outra sequence a usa como nested.
    DeleteSequence {
        sequence: SequenceId,
    },
    RenameSequence {
        sequence: SequenceId,
        name: String,
    },
    AddMarker {
        sequence: SequenceId,
        #[serde(default)]
        id: Option<MarkerId>,
        time: Ticks,
        #[serde(default)]
        label: String,
    },
    MoveMarker {
        sequence: SequenceId,
        marker: MarkerId,
        time: Ticks,
    },
    DeleteMarker {
        sequence: SequenceId,
        marker: MarkerId,
    },

    // --- clips ----------------------------------------------------------------------------
    InsertClip {
        track: TrackId,
        start: Ticks,
        clip: NewClip,
        /// Track magnética: inserir no interior de um clip divide-o (D-S2: senão `NOT_ON_BOUNDARY`).
        #[serde(default)]
        split_at_insert: bool,
        #[serde(default)]
        split_new_id: Option<ClipId>,
    },
    /// Insere uma sequence dentro de outra. `duration` omitida ⇒ duração da filha menos `source_in`
    /// (alinhada ao frame do pai, mínimo 1 frame). Ciclo/profundidade/alvo são validados antes.
    InsertNested {
        track: TrackId,
        start: Ticks,
        sequence: SequenceId,
        #[serde(default)]
        id: Option<ClipId>,
        #[serde(default)]
        name: String,
        #[serde(default)]
        duration: Option<Ticks>,
        #[serde(default)]
        source_in: Ticks,
        #[serde(default)]
        follow_length: bool,
        #[serde(default)]
        split_at_insert: bool,
        #[serde(default)]
        split_new_id: Option<ClipId>,
    },
    /// Aponta um clip nested para outra sequence (mesmas validações da inserção).
    SetNestedTarget {
        clip: ClipId,
        sequence: SequenceId,
    },
    /// Liga/desliga o acompanhamento da duração da sequence filha (ADR-045).
    SetFollowLength {
        clip: ClipId,
        follow_length: bool,
    },
    /// Copia uma sequence (tracks, clips, marcadores) com ids **determinísticos** (`<nova>.<id>`).
    /// Os nested internos continuam compartilhando as filhas, salvo `deep` (copia a subárvore).
    DuplicateSequence {
        source: SequenceId,
        #[serde(default)]
        new_sequence: Option<SequenceId>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        deep: bool,
    },
    /// Dá ao clip nested uma cópia **independente** da sequence filha (e, com `deep`, de toda a
    /// subárvore) e o aponta para ela. Nenhum id é reaproveitado.
    MakeUnique {
        clip: ClipId,
        #[serde(default)]
        new_sequence: Option<SequenceId>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        deep: bool,
    },
    /// Substitui o clip nested pelos clips da filha (recortados à janela visível), em novas tracks
    /// livres. Falha em vez de perder informação (retime, propriedades próprias, track magnética).
    FlattenNested {
        clip: ClipId,
        /// Prefixo dos ids criados (padrão: o id do clip).
        #[serde(default)]
        prefix: Option<String>,
    },
    /// Move os clips selecionados (mesmos `ClipId`) para uma nova sequence e os substitui por um
    /// clip nested que ocupa o mesmo trecho, preservando o tempo relativo.
    CreateNestedFromSelection {
        clips: Vec<ClipId>,
        #[serde(default)]
        new_sequence: Option<SequenceId>,
        #[serde(default)]
        name: Option<String>,
        /// Id do clip nested criado.
        #[serde(default)]
        clip_id: Option<ClipId>,
        /// Track que recebe o nested (padrão: a de cima entre as dos clips selecionados).
        #[serde(default)]
        track: Option<TrackId>,
        #[serde(default)]
        follow_length: bool,
    },
    /// Variantes de um template: cada variante é uma cópia do template com trocas estruturais de
    /// nested alvo ou de mídia. **Não** usa IA.
    GenerateVariants {
        template: SequenceId,
        variants: Vec<VariantSpec>,
    },
    MoveClips {
        moves: Vec<ClipMove>,
    },
    DeleteClip {
        clip: ClipId,
        /// Padrão: `true` em track magnética, `false` nas demais.
        #[serde(default)]
        ripple: Option<bool>,
        #[serde(default)]
        scope: RippleScope,
    },
    TrimClip {
        clip: ClipId,
        edge: Edge,
        /// Novo instante da borda, no tempo da sequence.
        to: Ticks,
        #[serde(default)]
        ripple: Option<bool>,
        #[serde(default)]
        scope: RippleScope,
    },
    SplitClip {
        clip: ClipId,
        at: Ticks,
        #[serde(default)]
        new_id: Option<ClipId>,
    },
    SetClipSpeed {
        clip: ClipId,
        speed: Rational,
        #[serde(default)]
        ripple: Option<bool>,
        #[serde(default)]
        scope: RippleScope,
    },

    // --- propriedades e keyframes ---------------------------------------------------------
    SetProperty {
        clip: ClipId,
        prop: String,
        value: f64,
    },
    AddKeyframe {
        clip: ClipId,
        prop: String,
        /// Instante no tempo da **sequence**; o keyframe é guardado em tempo de conteúdo.
        at: Ticks,
        value: f64,
        #[serde(default)]
        interp: Option<Interp>,
    },
    MoveKeyframe {
        clip: ClipId,
        prop: String,
        from: Ticks,
        to: Ticks,
    },
    DeleteKeyframe {
        clip: ClipId,
        prop: String,
        at: Ticks,
    },
    SetKeyframeInterp {
        clip: ClipId,
        prop: String,
        at: Ticks,
        interp: Interp,
    },
}

impl Command {
    /// Nome estável do tipo de comando.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::RegisterAsset { .. } => "register_asset",
            Self::DeleteAsset { .. } => "delete_asset",
            Self::UpdateAsset { .. } => "update_asset",
            Self::DuplicateSequence { .. } => "duplicate_sequence",
            Self::MakeUnique { .. } => "make_unique",
            Self::FlattenNested { .. } => "flatten_nested",
            Self::CreateNestedFromSelection { .. } => "create_nested_from_selection",
            Self::GenerateVariants { .. } => "generate_variants",
            Self::CreateSequence { .. } => "create_sequence",
            Self::AddTrack { .. } => "add_track",
            Self::SetTrackFlags { .. } => "set_track_flags",
            Self::DeleteTrack { .. } => "delete_track",
            Self::DeleteSequence { .. } => "delete_sequence",
            Self::RenameSequence { .. } => "rename_sequence",
            Self::InsertNested { .. } => "insert_nested",
            Self::SetNestedTarget { .. } => "set_nested_target",
            Self::SetFollowLength { .. } => "set_follow_length",
            Self::AddMarker { .. } => "add_marker",
            Self::MoveMarker { .. } => "move_marker",
            Self::DeleteMarker { .. } => "delete_marker",
            Self::InsertClip { .. } => "insert_clip",
            Self::MoveClips { .. } => "move_clips",
            Self::DeleteClip { .. } => "delete_clip",
            Self::TrimClip { .. } => "trim_clip",
            Self::SplitClip { .. } => "split_clip",
            Self::SetClipSpeed { .. } => "set_clip_speed",
            Self::SetProperty { .. } => "set_property",
            Self::AddKeyframe { .. } => "add_keyframe",
            Self::MoveKeyframe { .. } => "move_keyframe",
            Self::DeleteKeyframe { .. } => "delete_keyframe",
            Self::SetKeyframeInterp { .. } => "set_keyframe_interp",
        }
    }

    /// Rótulo humano curto para o histórico.
    pub fn label(&self) -> String {
        match self {
            Self::InsertClip { track, .. } => format!("Insert clip on {track}"),
            Self::DeleteClip { clip, .. } => format!("Delete clip {clip}"),
            Self::TrimClip { clip, edge, .. } => format!("Trim {edge:?} of clip {clip}"),
            Self::SplitClip { clip, .. } => format!("Split clip {clip}"),
            Self::SetClipSpeed { clip, speed, .. } => format!("Set speed of {clip} to {speed}x"),
            other => other.type_name().replace('_', " "),
        }
    }
}

/// Comando com sua identidade de submissão (ADR-029).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandEnvelope {
    /// ≤ 128 caracteres; único na transação e no projeto.
    pub operation_id: String,
    /// Referência simbólica (`$nome`) para o id principal criado por este comando.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(flatten)]
    pub command: Command,
}

/// Transação: uma única `HistoryEntry`, uma única mudança de revisão; tudo ou nada.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    #[serde(default)]
    pub transaction_id: Option<String>,
    pub label: String,
    /// Revisão sobre a qual o autor planejou. Se o documento já avançou, tenta-se rebase.
    #[serde(default)]
    pub base_revision: Option<u64>,
    pub commands: Vec<CommandEnvelope>,
    /// Teto de ops primitivas (padrão 10.000).
    #[serde(default)]
    pub max_ops: Option<usize>,
}
