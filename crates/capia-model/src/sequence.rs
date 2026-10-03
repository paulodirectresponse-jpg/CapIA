use crate::clip::{Clip, ClipContent};
use crate::error::ErrorCode;
use crate::ids::{ClipId, MarkerId, SequenceId, TrackId};
use capia_time::{FrameRate, Ticks};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Família de uma track (semântica de render distinta; ADR-008).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Visual,
    Audio,
}

/// Papel de uma track: dica para UX/IA, não restrição dura.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackRole {
    Main,
    #[default]
    Overlay,
    Text,
    Captions,
    Voice,
    Music,
    Sfx,
    Custom(String),
}

fn default_true() -> bool {
    true
}

/// Track. A ordem na sequence é a ordem de empilhamento do documento (topo → base na UI).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    #[serde(default)]
    pub name: String,
    pub kind: TrackKind,
    #[serde(default)]
    pub role: TrackRole,
    /// Sem gaps; inserir/apagar/recortar faz ripple na própria track.
    #[serde(default)]
    pub magnetic: bool,
    /// Rejeita qualquer comando que a altere (`TRACK_LOCKED`), inclusive de IA.
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    /// Participa de ripple de escopo mais amplo que a própria track (D-S7-3). Padrão: `true`.
    #[serde(default = "default_true")]
    pub sync_lock: bool,
    /// Grupo de tracks (rótulo livre) usado por `RippleScope::Group`.
    #[serde(default)]
    pub group: Option<String>,
}

impl Track {
    pub fn new(id: impl Into<TrackId>, kind: TrackKind) -> Self {
        Self {
            id: id.into(),
            name: String::new(),
            kind,
            role: TrackRole::default(),
            magnetic: false,
            locked: false,
            hidden: false,
            muted: false,
            solo: false,
            sync_lock: true,
            group: None,
        }
    }
}

/// Marcador na timeline (alvo de snap).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub id: MarkerId,
    pub time: Ticks,
    #[serde(default)]
    pub label: String,
}

/// Cabeçalho de uma sequence (tudo que não é coleção).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceHeader {
    pub name: String,
    pub frame_rate: FrameRate,
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
}

fn default_sample_rate() -> u32 {
    48_000
}

/// Ordenação de clips numa track: `(start, id)`. Permite starts repetidos *transitoriamente* durante
/// a aplicação de ops; a invariante de não-sobreposição é checada na validação.
type TrackIndex = BTreeSet<(Ticks, ClipId)>;

/// Referência de um clip `Nested` (índice derivado; não é serializado).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NestedRef {
    pub target: SequenceId,
    pub follow_length: bool,
}

fn nested_ref_of(clip: &Clip) -> Option<NestedRef> {
    match &clip.content {
        ClipContent::Nested {
            sequence,
            follow_length,
        } => Some(NestedRef {
            target: sequence.clone(),
            follow_length: *follow_length,
        }),
        _ => None,
    }
}

/// Sequence: tracks, clips e marcadores. Coleções são privadas para manter o índice por track
/// consistente; toda mutação passa pelas ops primitivas (`Document::apply_op`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sequence {
    pub header: SequenceHeader,
    tracks: Vec<Track>,
    clips: BTreeMap<ClipId, Clip>,
    markers: BTreeMap<MarkerId, Marker>,
    #[serde(skip)]
    by_track: BTreeMap<TrackId, TrackIndex>,
    #[serde(skip)]
    nested_index: BTreeMap<ClipId, NestedRef>,
}

#[derive(Deserialize)]
struct SequenceRaw {
    header: SequenceHeader,
    #[serde(default)]
    tracks: Vec<Track>,
    #[serde(default)]
    clips: BTreeMap<ClipId, Clip>,
    #[serde(default)]
    markers: BTreeMap<MarkerId, Marker>,
}

impl<'de> Deserialize<'de> for Sequence {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = SequenceRaw::deserialize(deserializer)?;
        Sequence::from_parts(raw.header, raw.tracks, raw.clips, raw.markers)
            .map_err(serde::de::Error::custom)
    }
}

impl Sequence {
    pub fn new(header: SequenceHeader) -> Self {
        Self {
            header,
            tracks: Vec::new(),
            clips: BTreeMap::new(),
            markers: BTreeMap::new(),
            by_track: BTreeMap::new(),
            nested_index: BTreeMap::new(),
        }
    }

    /// Reconstrói o índice. Falha se houver track repetida, clip em track inexistente ou chave
    /// de clip que não bate com o `id` do valor.
    pub fn from_parts(
        header: SequenceHeader,
        tracks: Vec<Track>,
        clips: BTreeMap<ClipId, Clip>,
        markers: BTreeMap<MarkerId, Marker>,
    ) -> Result<Self, String> {
        let mut by_track: BTreeMap<TrackId, TrackIndex> = BTreeMap::new();
        for t in &tracks {
            if by_track.insert(t.id.clone(), TrackIndex::new()).is_some() {
                return Err(format!("duplicate track id {}", t.id));
            }
        }
        for (key, clip) in &clips {
            if key != &clip.id {
                return Err(format!("clip key {key} does not match its id {}", clip.id));
            }
            let Some(index) = by_track.get_mut(&clip.track) else {
                return Err(format!(
                    "clip {} references missing track {}",
                    clip.id, clip.track
                ));
            };
            index.insert((clip.start, clip.id.clone()));
        }
        let nested_index = clips
            .values()
            .filter_map(|c| nested_ref_of(c).map(|n| (c.id.clone(), n)))
            .collect();
        Ok(Self {
            header,
            tracks,
            clips,
            markers,
            by_track,
            nested_index,
        })
    }

    pub fn frame_rate(&self) -> FrameRate {
        self.header.frame_rate
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn track(&self, id: &TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| &t.id == id)
    }

    pub fn track_position(&self, id: &TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| &t.id == id)
    }

    pub fn clip(&self, id: &ClipId) -> Option<&Clip> {
        self.clips.get(id)
    }

    pub fn clip_count(&self) -> usize {
        self.clips.len()
    }

    /// Todos os clips, em ordem de id.
    pub fn clips(&self) -> impl Iterator<Item = &Clip> {
        self.clips.values()
    }

    pub fn markers(&self) -> impl Iterator<Item = &Marker> {
        self.markers.values()
    }

    pub fn marker(&self, id: &MarkerId) -> Option<&Marker> {
        self.markers.get(id)
    }

    /// Clips `Nested` desta sequence e para onde apontam (índice derivado, O(log n)).
    pub fn nested_refs(&self) -> impl Iterator<Item = (&ClipId, &NestedRef)> {
        self.nested_index.iter()
    }

    pub fn marker_count(&self) -> usize {
        self.markers.len()
    }

    /// Clips da track em ordem de início.
    pub fn track_clips<'a>(&'a self, track: &TrackId) -> impl Iterator<Item = &'a Clip> + 'a {
        self.by_track
            .get(track)
            .into_iter()
            .flat_map(|idx| idx.iter())
            .filter_map(|(_, id)| self.clips.get(id))
    }

    pub fn track_clip_count(&self, track: &TrackId) -> usize {
        self.by_track.get(track).map_or(0, BTreeSet::len)
    }

    /// Clip que contém o instante `t` (`start ≤ t < end`) na track. O(log n).
    pub fn clip_at(&self, track: &TrackId, t: Ticks) -> Option<&Clip> {
        let idx = self.by_track.get(track)?;
        let bound = (Ticks(t.0.saturating_add(1)), ClipId::new(""));
        let (_, id) = idx.range(..bound).next_back()?;
        self.clips.get(id).filter(|c| c.end() > t)
    }

    /// Clips da track que intersectam `[start, end)`, em ordem. O(log n + k).
    pub fn clips_in(&self, track: &TrackId, start: Ticks, end: Ticks) -> Vec<&Clip> {
        let Some(idx) = self.by_track.get(track) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // Como clips válidos não se sobrepõem, só o último com start < `start` pode alcançar `start`.
        let lower = (start, ClipId::new(""));
        if let Some((_, id)) = idx.range(..lower.clone()).next_back()
            && let Some(c) = self.clips.get(id).filter(|c| c.end() > start)
        {
            out.push(c);
        }
        let upper = (end, ClipId::new(""));
        for (_, id) in idx.range(lower..upper) {
            if let Some(c) = self.clips.get(id) {
                out.push(c);
            }
        }
        out
    }

    /// Fim do último clip da track (0 se vazia).
    pub fn track_end(&self, track: &TrackId) -> Ticks {
        self.track_clips(track)
            .map(Clip::end)
            .max()
            .unwrap_or(Ticks::ZERO)
    }

    /// Duração da sequence: fim do último clip em qualquer track.
    pub fn duration(&self) -> Ticks {
        self.clips
            .values()
            .map(Clip::end)
            .max()
            .unwrap_or(Ticks::ZERO)
    }

    // ---- mutação de baixo nível (só `ops`) ------------------------------------------------

    pub(crate) fn raw_insert_track(
        &mut self,
        index: usize,
        track: Track,
    ) -> Result<(), (ErrorCode, String)> {
        if self.by_track.contains_key(&track.id) {
            return Err((
                ErrorCode::OpMismatch,
                format!("track {} already exists", track.id),
            ));
        }
        if index > self.tracks.len() {
            return Err((
                ErrorCode::OpMismatch,
                format!("track index {index} out of bounds"),
            ));
        }
        self.by_track.insert(track.id.clone(), TrackIndex::new());
        self.tracks.insert(index, track);
        Ok(())
    }

    pub(crate) fn raw_remove_track(
        &mut self,
        id: &TrackId,
    ) -> Result<(usize, Track), (ErrorCode, String)> {
        let Some(pos) = self.track_position(id) else {
            return Err((ErrorCode::OpMismatch, format!("track {id} does not exist")));
        };
        if self.by_track.get(id).is_some_and(|i| !i.is_empty()) {
            return Err((ErrorCode::OpMismatch, format!("track {id} still has clips")));
        }
        self.by_track.remove(id);
        Ok((pos, self.tracks.remove(pos)))
    }

    pub(crate) fn raw_replace_track(
        &mut self,
        id: &TrackId,
        new: Track,
    ) -> Result<Track, (ErrorCode, String)> {
        let Some(pos) = self.track_position(id) else {
            return Err((ErrorCode::OpMismatch, format!("track {id} does not exist")));
        };
        if new.id != *id {
            return Err((ErrorCode::OpMismatch, "track id cannot change".to_owned()));
        }
        Ok(core::mem::replace(&mut self.tracks[pos], new))
    }

    /// Reposiciona uma track (reordenar) e atualiza suas propriedades, preservando o índice de clips.
    pub(crate) fn raw_reposition_track(
        &mut self,
        id: &TrackId,
        new_index: usize,
        new: Track,
    ) -> Result<(), (ErrorCode, String)> {
        let Some(pos) = self.track_position(id) else {
            return Err((ErrorCode::OpMismatch, format!("track {id} does not exist")));
        };
        if new.id != *id || new_index >= self.tracks.len() {
            return Err((ErrorCode::OpMismatch, "invalid track reposition".to_owned()));
        }
        self.tracks.remove(pos);
        self.tracks.insert(new_index, new);
        Ok(())
    }

    pub(crate) fn raw_insert_clip(&mut self, clip: Clip) -> Result<(), (ErrorCode, String)> {
        let Some(index) = self.by_track.get_mut(&clip.track) else {
            return Err((
                ErrorCode::NotFound,
                format!("track {} does not exist", clip.track),
            ));
        };
        if self.clips.contains_key(&clip.id) {
            return Err((
                ErrorCode::OpMismatch,
                format!("clip {} already exists", clip.id),
            ));
        }
        index.insert((clip.start, clip.id.clone()));
        if let Some(n) = nested_ref_of(&clip) {
            self.nested_index.insert(clip.id.clone(), n);
        }
        self.clips.insert(clip.id.clone(), clip);
        Ok(())
    }

    pub(crate) fn raw_remove_clip(&mut self, id: &ClipId) -> Result<Clip, (ErrorCode, String)> {
        let Some(clip) = self.clips.remove(id) else {
            return Err((ErrorCode::OpMismatch, format!("clip {id} does not exist")));
        };
        if let Some(index) = self.by_track.get_mut(&clip.track) {
            index.remove(&(clip.start, clip.id.clone()));
        }
        self.nested_index.remove(id);
        Ok(clip)
    }

    pub(crate) fn raw_replace_clip(&mut self, new: Clip) -> Result<Clip, (ErrorCode, String)> {
        let old = self.raw_remove_clip(&new.id)?;
        if let Err(e) = self.raw_insert_clip(new) {
            // restaura para não deixar a sequence inconsistente
            let _ = self.raw_insert_clip(old);
            return Err(e);
        }
        Ok(old)
    }

    pub(crate) fn raw_put_marker(
        &mut self,
        marker: Option<Marker>,
        id: &MarkerId,
    ) -> Option<Marker> {
        match marker {
            Some(m) => self.markers.insert(id.clone(), m),
            None => self.markers.remove(id),
        }
    }

    /// Sem tracks, clips nem marcadores (pode ser apagada).
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.clips.is_empty() && self.markers.is_empty()
    }

    /// Verifica que o índice por track bate com os clips (usado em testes e na validação).
    pub fn index_is_consistent(&self) -> bool {
        let total: usize = self.by_track.values().map(BTreeSet::len).sum();
        let nested_ok = self.nested_index.len()
            == self
                .clips
                .values()
                .filter(|c| nested_ref_of(c).is_some())
                .count()
            && self
                .clips
                .values()
                .all(|c| self.nested_index.get(&c.id) == nested_ref_of(c).as_ref());
        nested_ok
            && total == self.clips.len()
            && self.tracks.len() == self.by_track.len()
            && self.clips.values().all(|c| {
                self.by_track
                    .get(&c.track)
                    .is_some_and(|i| i.contains(&(c.start, c.id.clone())))
            })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::clip::ClipContent;

    fn header() -> SequenceHeader {
        SequenceHeader {
            name: "s".into(),
            frame_rate: FrameRate::FPS_30,
            sample_rate: 48_000,
        }
    }

    fn solid(id: &str, track: &str, start: i64, dur: i64) -> Clip {
        Clip {
            id: id.into(),
            track: track.into(),
            start: Ticks(start),
            duration: Ticks(dur),
            name: String::new(),
            enabled: true,
            content: ClipContent::Solid {
                color: "#000".into(),
            },
            source_in: Ticks(0),
            speed: capia_time::Rational::ONE,
            reversed: false,
            properties: Default::default(),
        }
    }

    fn seq() -> Sequence {
        let mut s = Sequence::new(header());
        s.raw_insert_track(0, Track::new("V1", TrackKind::Visual))
            .unwrap();
        for (id, start, dur) in [("a", 0, 10), ("b", 10, 10), ("c", 30, 5)] {
            s.raw_insert_clip(solid(id, "V1", start, dur)).unwrap();
        }
        s
    }

    #[test]
    fn clip_at_and_clips_in_use_half_open_ranges() {
        let s = seq();
        let v1 = TrackId::from("V1");
        assert_eq!(s.clip_at(&v1, Ticks(0)).unwrap().id.as_str(), "a");
        assert_eq!(s.clip_at(&v1, Ticks(9)).unwrap().id.as_str(), "a");
        assert_eq!(s.clip_at(&v1, Ticks(10)).unwrap().id.as_str(), "b");
        assert!(s.clip_at(&v1, Ticks(20)).is_none(), "gap");
        assert!(s.clip_at(&v1, Ticks(35)).is_none(), "end is exclusive");
        let ids = |v: Vec<&Clip>| v.iter().map(|c| c.id.0.clone()).collect::<Vec<_>>();
        assert_eq!(ids(s.clips_in(&v1, Ticks(5), Ticks(12))), ["a", "b"]);
        assert_eq!(ids(s.clips_in(&v1, Ticks(10), Ticks(30))), ["b"]);
        assert_eq!(
            ids(s.clips_in(&v1, Ticks(20), Ticks(30))),
            Vec::<String>::new()
        );
        assert_eq!(ids(s.clips_in(&v1, Ticks(0), Ticks(100))), ["a", "b", "c"]);
        assert_eq!(s.track_end(&v1), Ticks(35));
        assert_eq!(s.duration(), Ticks(35));
        assert!(s.index_is_consistent());
    }

    #[test]
    fn replace_clip_updates_the_track_index() {
        let mut s = seq();
        let old = s.raw_replace_clip(solid("a", "V1", 50, 10)).unwrap();
        assert_eq!(old.start, Ticks(0));
        let v1 = TrackId::from("V1");
        assert!(s.clip_at(&v1, Ticks(0)).is_none());
        assert_eq!(s.clip_at(&v1, Ticks(55)).unwrap().id.as_str(), "a");
        assert!(s.index_is_consistent());
    }

    #[test]
    fn track_with_clips_cannot_be_removed() {
        let mut s = seq();
        assert!(s.raw_remove_track(&"V1".into()).is_err());
        for id in ["a", "b", "c"] {
            s.raw_remove_clip(&id.into()).unwrap();
        }
        assert!(s.raw_remove_track(&"V1".into()).is_ok());
        assert!(s.is_empty());
    }

    #[test]
    fn json_round_trip_rebuilds_the_index() {
        let s = seq();
        let json = serde_json::to_string(&s).unwrap();
        let back: Sequence = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert!(back.index_is_consistent());
        assert_eq!(
            back.clip_at(&"V1".into(), Ticks(31)).unwrap().id.as_str(),
            "c"
        );
    }

    #[test]
    fn deserialize_rejects_dangling_clip_tracks() {
        let mut s = Sequence::new(header());
        s.raw_insert_track(0, Track::new("V1", TrackKind::Visual))
            .unwrap();
        s.raw_insert_clip(solid("a", "V1", 0, 10)).unwrap();
        let json = serde_json::to_string(&s)
            .unwrap()
            .replace("\"track\":\"V1\"", "\"track\":\"nope\"");
        assert!(serde_json::from_str::<Sequence>(&json).is_err());
    }
}
