//! Consulta pura de intervalo da timeline: o que a UI virtualizada pede a cada rolagem/zoom
//! ("quais clips intersectam `[start, end)` em cada track"). Usa só o índice por track do modelo
//! (`Sequence::clips_in`, O(log n + k)) — nenhum IPC, nenhum comando.

use capia_model::{Clip, Sequence};
use capia_time::Ticks;

/// Clips de **todas** as tracks que intersectam `[start, end)`, em ordem de track e de início.
pub fn clips_in_range(seq: &Sequence, start: Ticks, end: Ticks) -> Vec<&Clip> {
    let mut out = Vec::new();
    for track in seq.tracks() {
        out.extend(seq.clips_in(&track.id, start, end));
    }
    out
}

/// Referência ingênua (varre todos os clips): oráculo dos testes.
pub fn clips_in_range_naive(seq: &Sequence, start: Ticks, end: Ticks) -> Vec<&Clip> {
    let mut out: Vec<&Clip> = seq
        .clips()
        .filter(|c| c.start < end && c.end() > start)
        .collect();
    out.sort_by(|a, b| {
        let ta = seq.track_position(&a.track);
        let tb = seq.track_position(&b.track);
        (ta, a.start, &a.id).cmp(&(tb, b.start, &b.id))
    });
    out
}
