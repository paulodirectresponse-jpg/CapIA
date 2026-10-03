//! Modelo do documento do CapIA (docs/DATA_MODEL.md, docs/TIMELINE_ENGINE.md).
//!
//! * Entidades: [`Document`], [`Sequence`], [`Track`], [`Clip`], [`Marker`], [`Asset`].
//! * Escrita **somente** por [`PrimitiveOp`] (com inversa), aplicada por [`Document::apply_op`].
//! * Estado imutável com compartilhamento estrutural (`Arc<Sequence>`): `clone()` é barato.
//! * Invariantes em [`validate`]; keyframes e avaliação de propriedades em [`property`].
//!
//! Depende só de `capia-time` (+ `serde`), não faz IO e compila para WASM (ADR-016).

pub mod clip;
pub mod document;
pub mod error;
pub mod ids;
pub mod ops;
pub mod property;
pub mod sequence;
pub mod validate;

pub use capia_time::Ticks;
pub use clip::{Clip, ClipContent, speed_in_range};
pub use document::{Asset, DOCUMENT_SCHEMA_VERSION, Document, OpError};
pub use error::ErrorCode;
pub use ids::{AssetId, ClipId, EntityKind, EntityRef, MarkerId, SequenceId, TrackId};
pub use ops::{PrimitiveOp, TrackSlot};
pub use property::{
    Animatable, Interp, Keyframe, PropertySet, PropertySpec, property_spec, property_specs,
};
pub use sequence::{Marker, NestedRef, Sequence, SequenceHeader, Track, TrackKind, TrackRole};
pub use validate::{
    MAX_CLIPS_PER_SEQUENCE, MAX_NESTING_DEPTH, MAX_TRACKS_PER_SEQUENCE, Violation,
    check_nested_edge, content_fits_track, nested_depth_above, nested_depth_below, nested_path,
    validate_clip, validate_document, validate_nested_graph, validate_sequence,
};
