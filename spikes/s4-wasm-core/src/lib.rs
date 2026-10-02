//! IO-free toy core: Ticks (i64), per-track ordered clip index, global edge index, ghost move with snapping.
use std::collections::BTreeMap;
use std::sync::Mutex;

pub const TICKS_PER_SECOND: i64 = 705_600_000;
pub const MAX_TICKS: i64 = 24 * 3600 * TICKS_PER_SECOND; // 24 h cap (also keeps values < 2^53 for JS Number)

#[derive(Clone, Copy)]
pub struct Clip { pub track: u32, pub start: i64, pub dur: i64 }

#[derive(Default)]
pub struct Timeline {
    pub clips: Vec<Clip>,
    pub tracks: Vec<BTreeMap<i64, usize>>, // start -> clip index
    pub edges: BTreeMap<i64, u32>,         // edge tick -> refcount (snap targets)
}

fn xorshift(s: &mut u64) -> u64 { *s ^= *s << 13; *s ^= *s >> 7; *s ^= *s << 17; *s }

impl Timeline {
    fn add_edge(&mut self, t: i64) { *self.edges.entry(t).or_insert(0) += 1; }
    fn del_edge(&mut self, t: i64) { if let Some(c) = self.edges.get_mut(&t) { *c -= 1; if *c == 0 { self.edges.remove(&t); } } }

    pub fn build(n_tracks: u32, per_track: u32, seed: u64) -> Self {
        let mut tl = Timeline::default();
        let mut s = seed | 1;
        let frame = TICKS_PER_SECOND * 1001 / 30000; // 29.97 frame duration (exact: 23_543_520)
        for tr in 0..n_tracks {
            let mut map = BTreeMap::new();
            let mut cursor = (xorshift(&mut s) % 40) as i64 * frame;
            for _ in 0..per_track {
                let dur = (15 + xorshift(&mut s) % 200) as i64 * frame;
                let gap = (xorshift(&mut s) % 30) as i64 * frame;
                let idx = tl.clips.len();
                tl.clips.push(Clip { track: tr, start: cursor, dur });
                map.insert(cursor, idx);
                tl.add_edge(cursor); tl.add_edge(cursor + dur);
                cursor += dur + gap;
            }
            tl.tracks.push(map);
        }
        tl
    }

    fn nearest_edge(&self, t: i64, thresh: i64, skip: &[i64]) -> Option<i64> {
        let mut best: Option<i64> = None;
        for (&e, _) in self.edges.range(t - thresh..=t + thresh) {
            if skip.contains(&e) { continue; }
            if best.map_or(true, |b| (e - t).abs() < (b - t).abs()) { best = Some(e); }
        }
        best
    }

    /// Returns snapped start, or -1 if the (snapped) placement overlaps a neighbour or leaves [0, MAX].
    pub fn ghost_move(&self, idx: usize, new_start: i64, thresh: i64) -> i64 {
        let c = self.clips[idx];
        let own = [c.start, c.start + c.dur];
        let mut start = new_start;
        let a = self.nearest_edge(new_start, thresh, &own);
        let b = self.nearest_edge(new_start + c.dur, thresh, &own);
        match (a, b) {
            (Some(x), Some(y)) => { if (x - new_start).abs() <= (y - (new_start + c.dur)).abs() { start = x } else { start = y - c.dur } }
            (Some(x), None) => start = x,
            (None, Some(y)) => start = y - c.dur,
            _ => {}
        }
        if start < 0 || start + c.dur > MAX_TICKS { return -1; }
        let map = &self.tracks[c.track as usize];
        if let Some((&ps, &pi)) = map.range(..=start).filter(|(_, &i)| i != idx).next_back() { if ps + self.clips[pi].dur > start { return -1; } }
        if let Some((&ns, _)) = map.range(start..).filter(|(_, &i)| i != idx).next() { if ns < start + c.dur { return -1; } }
        start
    }

    pub fn commit_move(&mut self, idx: usize, start: i64) {
        let c = self.clips[idx];
        self.tracks[c.track as usize].remove(&c.start);
        self.del_edge(c.start); self.del_edge(c.start + c.dur);
        self.clips[idx].start = start;
        self.tracks[c.track as usize].insert(start, idx);
        self.add_edge(start); self.add_edge(start + c.dur);
    }
}

/// Deterministic workload; returns an FNV-style hash of every result => parity probe.
pub fn run_ops(tl: &mut Timeline, seed: u64, n: u32) -> (u64, u32) {
    let mut s = seed | 1; let mut h: u64 = 0xcbf29ce484222325; let mut commits = 0;
    let frame = TICKS_PER_SECOND * 1001 / 30000;
    for k in 0..n {
        let i = (xorshift(&mut s) % tl.clips.len() as u64) as usize;
        let jitter = (xorshift(&mut s) % 4001) as i64 - 2000;
        let target = (tl.clips[i].start + jitter * frame / 7).max(0);
        let r = tl.ghost_move(i, target, 6 * frame);
        h = (h ^ (r as u64)).wrapping_mul(0x100000001b3);
        if r >= 0 && k % 3 == 0 { tl.commit_move(i, r); commits += 1; }
    }
    (h, commits)
}

// ---- C ABI for WASM (f64 ticks at the boundary: i64 would force BigInt in JS) ----
static TL: Mutex<Option<Timeline>> = Mutex::new(None);

#[no_mangle] pub extern "C" fn init(tracks: u32, per_track: u32, seed: u32) -> u32 {
    let tl = Timeline::build(tracks, per_track, seed as u64); let n = tl.clips.len() as u32; *TL.lock().unwrap() = Some(tl); n
}
#[no_mangle] pub extern "C" fn ghost_move(idx: u32, new_start: f64, thresh: f64) -> f64 {
    let g = TL.lock().unwrap(); g.as_ref().unwrap().ghost_move(idx as usize, new_start as i64, thresh as i64) as f64
}
#[no_mangle] pub extern "C" fn commit_move(idx: u32, start: f64) { TL.lock().unwrap().as_mut().unwrap().commit_move(idx as usize, start as i64) }
#[no_mangle] pub extern "C" fn clip_start(idx: u32) -> f64 { TL.lock().unwrap().as_ref().unwrap().clips[idx as usize].start as f64 }
#[no_mangle] pub extern "C" fn run_ops_hash(seed: u32, n: u32) -> u64 {
    let mut g = TL.lock().unwrap(); run_ops(g.as_mut().unwrap(), seed as u64, n).0
}
