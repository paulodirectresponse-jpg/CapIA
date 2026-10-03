//! Consultas somente leitura sobre o documento.

use crate::error::{CommandError, Result};
use capia_model::{Animatable, ClipId, Document, property_spec};
use capia_time::Ticks;

/// Valor de uma propriedade no instante `at` (tempo da **sequence**), com a avaliação oficial
/// (a mesma de preview e export). Propriedade ausente ⇒ valor padrão.
pub fn eval_property(doc: &Document, clip: &ClipId, prop: &str, at: Ticks) -> Result<f64> {
    let (_, _, c) = doc
        .find_clip(clip)
        .ok_or_else(|| CommandError::not_found("clip", clip))?;
    let spec = property_spec(prop)
        .ok_or_else(|| CommandError::invalid(format!("unknown property {prop}")))?;
    let anim = c
        .properties
        .get(prop)
        .cloned()
        .unwrap_or(Animatable::Static(spec.default));
    Ok(anim.eval(c.content_time(at)?, spec))
}
