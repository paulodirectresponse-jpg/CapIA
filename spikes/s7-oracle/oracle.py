#!/usr/bin/env python3
"""S7 THROWAWAY reference oracle. Purpose: prove the acceptance scenarios in tests/acceptance/timeline/*.json are
self-consistent and unambiguous. Expected values in the scenarios were derived BY HAND from the CapIA spec, not by this file.
This is NOT the CapIA engine. Times are frames (Fraction); ticks asserted separately."""
import json, sys, glob, os
from fractions import Fraction as F

class E(Exception):
    def __init__(s, code): s.code = code

TPS = 705_600_000
RANGES = {"opacity": (0.0, 1.0), "scale": (0.0, 10.0)}
SPEED_MIN, SPEED_MAX = F(1, 100), F(5)

def fr(x): return F(x) if isinstance(x, str) else F(x)
def rhu(x):  # round half up
    return int((x + F(1, 2)).__floor__())

def mk_clip(d, track):
    c = {"id": d["id"], "start": fr(d.get("start", 0)), "dur": fr(d["dur"]), "src_in": fr(d.get("src_in", 0)),
         "src_dur": None if d.get("src_dur") is None else fr(d["src_dur"]), "speed": fr(d.get("speed", 1)),
         "reversed": d.get("reversed", False), "content": d.get("content", "media"), "kind": d.get("kind", track["kind"]),
         "kf": {}, "static": {"opacity": 1.0}}
    for prop, lst in d.get("kf", {}).items():
        c["kf"][prop] = [{"t": fr(t), "v": float(v), "i": i} for t, v, *r in lst for i in [r[0] if r else "linear"]]
    return c

def load(given):
    tracks = {}; order = []
    for t in given["tracks"]:
        tr = {"id": t["id"], "kind": t["kind"], "magnetic": t.get("magnetic", False), "locked": t.get("locked", False), "clips": []}
        tr["clips"] = [mk_clip(c, tr) for c in t.get("clips", [])]
        tracks[tr["id"]] = tr; order.append(tr["id"])
    return tracks, order

def find(tracks, cid):
    for t in tracks.values():
        for c in t["clips"]:
            if c["id"] == cid: return t, c
    raise E("NOT_FOUND")

def end(c): return c["start"] + c["dur"]
def overlaps(tr, s, e, excl=()):
    return any(c["id"] not in excl and s < end(c) and e > c["start"] for c in tr["clips"])
def sort(tr): tr["clips"].sort(key=lambda c: c["start"])

# ---------- keyframes ----------
def content_t(c, t):
    return c["src_in"] + (t - c["start"]) * c["speed"] if c["content"] == "media" else t - c["start"]

def bez(x1, y1, x2, y2, u):
    lo, hi = 0.0, 1.0
    for _ in range(50):
        s = (lo + hi) / 2; x = 3 * (1 - s) ** 2 * s * x1 + 3 * (1 - s) * s * s * x2 + s ** 3
        if x < u: lo = s
        else: hi = s
    s = (lo + hi) / 2
    return 3 * (1 - s) ** 2 * s * y1 + 3 * (1 - s) * s * s * y2 + s ** 3

def eval_kf(kfs, ct, static, prop):
    if not kfs: v = static
    elif ct <= kfs[0]["t"]: v = kfs[0]["v"]
    elif ct >= kfs[-1]["t"]: v = kfs[-1]["v"]
    else:
        for a, b in zip(kfs, kfs[1:]):
            if a["t"] <= ct < b["t"]:
                i = a["i"]; u = float((ct - a["t"]) / (b["t"] - a["t"]))
                if i == "hold": v = a["v"]
                elif i == "linear": v = a["v"] + (b["v"] - a["v"]) * u
                else: v = a["v"] + (b["v"] - a["v"]) * bez(*i["bezier"], u)
                break
    lo, hi = RANGES.get(prop, (-1e18, 1e18))
    return min(max(v, lo), hi)

def eval_clip(c, prop, t):
    return eval_kf(c["kf"].get(prop, []), content_t(c, t), c["static"].get(prop, 1.0), prop)

def split_kfs(c, prop, cs, shift_right):
    kfs = c["kf"].get(prop, [])
    if not kfs: return [], []
    left = [dict(k) for k in kfs if k["t"] <= cs]; right = [dict(k) for k in kfs if k["t"] >= cs]
    if kfs[0]["t"] < cs < kfs[-1]["t"] and not any(k["t"] == cs for k in kfs):
        seg = [k for k in kfs if k["t"] < cs][-1]
        b = {"t": cs, "v": eval_kf(kfs, cs, 0, prop), "i": seg["i"]}
        left.append(dict(b)); right.insert(0, dict(b))
    if shift_right:
        for k in right: k["t"] -= shift_right
    return left, right

# ---------- commands ----------
def cmd_insert_clip(tracks, a):
    tr = tracks[a["track"]]; d = a["clip"]
    if tr["locked"]: raise E("TRACK_LOCKED")
    if d.get("kind", tr["kind"]) != tr["kind"]: raise E("WRONG_TRACK_KIND")
    c = mk_clip({**d, "start": 0}, tr); start = fr(a["start"])
    if start < 0: raise E("INVALID_ARGUMENT")
    if tr["magnetic"]:
        sort(tr); tot = max([end(x) for x in tr["clips"]], default=F(0))
        start = min(start, tot)
        inside = [x for x in tr["clips"] if x["start"] < start < end(x)]
        if inside:
            if not a.get("split_at_insert"): raise E("NOT_ON_BOUNDARY")
            do_split(tracks, tr, inside[0], start, a["split_new_id"])
        for x in tr["clips"]:
            if x["start"] >= start: x["start"] += c["dur"]
        c["start"] = start; tr["clips"].append(c); sort(tr)
    else:
        if overlaps(tr, start, start + c["dur"]): raise E("OVERLAP")
        c["start"] = start; tr["clips"].append(c); sort(tr)

def cmd_delete_clip(tracks, a):
    tr, c = find(tracks, a["clip"])
    if tr["locked"]: raise E("TRACK_LOCKED")
    ripple = a.get("ripple", tr["magnetic"]); scope = a.get("scope", "track")
    if tr["magnetic"] and not ripple: raise E("INVALID_ARGUMENT")
    s, e = c["start"], end(c); dur = c["dur"]
    if ripple and scope == "sequence":
        for t in tracks.values():
            if t is tr or t["locked"]: continue
            if any(x["start"] < e and end(x) > s for x in t["clips"]): raise E("RIPPLE_CONFLICT")
    tr["clips"].remove(c)
    if ripple:
        for t in tracks.values():
            if t is not tr and (scope != "sequence" or t["locked"]): continue
            for x in t["clips"]:
                if x["start"] >= e: x["start"] -= dur

def check_handles(c, new_dur, new_src_in=None):
    src_in = c["src_in"] if new_src_in is None else new_src_in
    if src_in < 0: raise E("INSUFFICIENT_HANDLES")
    if c["src_dur"] is not None and src_in + new_dur * c["speed"] > c["src_dur"]: raise E("INSUFFICIENT_HANDLES")

def cmd_trim_clip(tracks, a):
    tr, c = find(tracks, a["clip"])
    if tr["locked"]: raise E("TRACK_LOCKED")
    ripple = a.get("ripple", tr["magnetic"]); to = fr(a["to"])
    if a["edge"] == "out":
        nd = to - c["start"]
        if nd < 1: raise E("INVALID_ARGUMENT")
        check_handles(c, nd); delta = nd - c["dur"]
        if not tr["magnetic"] and delta > 0 and overlaps(tr, c["start"], c["start"] + nd, {c["id"]}): raise E("OVERLAP")
        old_end = end(c); c["dur"] = nd
        if ripple:
            for x in tr["clips"]:
                if x is not c and x["start"] >= old_end: x["start"] += delta
    else:
        shift = to - c["start"]; nd = c["dur"] - shift
        if nd < 1: raise E("INVALID_ARGUMENT")
        nsi = c["src_in"] + shift * c["speed"]; check_handles(c, nd, nsi)
        if not tr["magnetic"] and shift < 0 and overlaps(tr, to, to + nd, {c["id"]}): raise E("OVERLAP")
        old_end = end(c); c["src_in"] = nsi; c["dur"] = nd
        if tr["magnetic"]:
            for x in tr["clips"]:
                if x is not c and x["start"] >= old_end: x["start"] -= shift
        else: c["start"] = to

def do_split(tracks, tr, c, at, new_id):
    if not (c["start"] < at < end(c)): raise E("INVALID_SPLIT_POINT")
    off = at - c["start"]; r = dict(c); r["id"] = new_id; r["kf"] = {}; r["static"] = dict(c["static"])
    if c["content"] == "media":
        cs = c["src_in"] + off * c["speed"]
        if c["reversed"]: r["src_in"] = c["src_in"]; c["src_in"] = c["src_in"] + (c["dur"] - off) * c["speed"]
        else: r["src_in"] = cs
    else: cs = off
    for prop in c["kf"]:
        L, R = split_kfs(c, prop, cs, off if c["content"] != "media" else 0)
        if not L: c["static"][prop] = c["kf"][prop][0]["v"]
        if not R: r["static"][prop] = c["kf"][prop][-1]["v"]
        c["kf"][prop] = L
        if R: r["kf"][prop] = R
    r["start"] = at; r["dur"] = end(c) - at; c["dur"] = off
    tr["clips"].append(r); sort(tr)

def cmd_split_clip(tracks, a):
    tr, c = find(tracks, a["clip"])
    if tr["locked"]: raise E("TRACK_LOCKED")
    do_split(tracks, tr, c, fr(a["at"]), a["new_id"])

def cmd_set_clip_speed(tracks, a):
    tr, c = find(tracks, a["clip"]); sp = fr(a["speed"])
    if tr["locked"]: raise E("TRACK_LOCKED")
    if not (SPEED_MIN <= sp <= SPEED_MAX): raise E("OUT_OF_RANGE")
    nd = max(1, rhu(c["dur"] * c["speed"] / sp)); delta = nd - c["dur"]
    if not tr["magnetic"] and delta > 0 and overlaps(tr, c["start"], c["start"] + nd, {c["id"]}): raise E("OVERLAP")
    old_end = end(c); c["dur"] = F(nd); c["speed"] = sp
    if tr["magnetic"]:
        for x in tr["clips"]:
            if x is not c and x["start"] >= old_end: x["start"] += delta

def cmd_add_keyframe(tracks, a):
    tr, c = find(tracks, a["clip"]); at = fr(a["at"]); prop = a["prop"]; v = float(a["value"])
    if at.denominator != 1: raise E("NOT_FRAME_ALIGNED")
    if not (c["start"] <= at <= end(c)): raise E("OUT_OF_CLIP_RANGE")
    lo, hi = RANGES.get(prop, (-1e18, 1e18))
    if not (lo <= v <= hi): raise E("OUT_OF_RANGE")
    ct = content_t(c, at); kfs = c["kf"].setdefault(prop, [])
    kfs[:] = [k for k in kfs if k["t"] != ct]
    kfs.append({"t": ct, "v": v, "i": a.get("interp", "linear")}); kfs.sort(key=lambda k: k["t"])

def cmd_move_keyframe(tracks, a):
    tr, c = find(tracks, a["clip"]); kfs = c["kf"][a["prop"]]; ct = content_t(c, fr(a["to"]))
    if any(k["t"] == ct for k in kfs): raise E("KEYFRAME_TIME_CONFLICT")
    k = [k for k in kfs if k["t"] == content_t(c, fr(a["from"]))][0]; k["t"] = ct; kfs.sort(key=lambda k: k["t"])

# ---------- pure resolvers ----------
def targets_for(tracks, exclude_ids, a):
    T = []
    if a.get("playhead") is not None: T.append((fr(a["playhead"]), "playhead", 0))
    for m in a.get("markers", []): T.append((fr(m), "marker", 1))
    T.append((F(0), "sequence_start", 2))
    for t in tracks.values():
        for c in t["clips"]:
            if c["id"] in exclude_ids: continue
            T.append((c["start"], "clip_start", 2)); T.append((end(c), "clip_end", 2))
    return T

def res_snap(tracks, order, a):
    tr, c = find(tracks, a["clip"]); prop = fr(a["proposed_start"]); thr = fr(a["threshold_frames"])
    if not a.get("enabled", True): return {"start": max(F(0), prop), "snapped_to": None}
    cands = []
    for (t, typ, pri) in targets_for(tracks, {c["id"]}, a):
        for edge_i, off in enumerate((0, c["dur"])):
            d = abs(prop + off - t)
            if d <= thr:
                s = t - off
                if s < 0 or overlaps(tr, s, s + c["dur"], {c["id"]}): continue
                cands.append((d, pri, t, edge_i, s, typ))
    if not cands: return {"start": max(F(0), prop), "snapped_to": None}
    d, pri, t, ei, s, typ = sorted(cands)[0]
    return {"start": s, "snapped_to": {"type": typ, "t": t}}

def res_threshold(a):
    return {"frames": int(F(a["px"]) / F(a["pps"]) * F(a["fps"]) // 1)}

def res_group_move(tracks, order, a):
    members = set(a["members"]); req = fr(a["delta_time"]); reqk = int(a.get("delta_tracks", 0))
    mem = [(find(tracks, m)) for m in a["members"]]
    for tr, c in mem:
        if tr["locked"]: raise E("TRACK_LOCKED")
    by_kind = {}
    for tid in order: by_kind.setdefault(tracks[tid]["kind"], []).append(tid)
    def target(tr, k):
        lst = by_kind[tr["kind"]]; i = lst.index(tr["id"]) + k
        if not (0 <= i < len(lst)): return None
        t = tracks[lst[i]]
        return None if t["locked"] else t
    def limits(k):
        lo, hi = F(-10**9), F(10**9)
        for tr, c in mem:
            tt = target(tr, k)
            if tt is None: return None
            obst = [x for x in tt["clips"] if x["id"] not in members]
            if any(c["start"] < end(x) and end(c) > x["start"] for x in obst): return None
            lo = max(lo, -c["start"])
            for x in obst:
                if x["start"] >= end(c): hi = min(hi, x["start"] - end(c))
                if end(x) <= c["start"]: lo = max(lo, -(c["start"] - end(x)))
        return lo, hi
    sign = 1 if reqk >= 0 else -1; k = reqk; lim = None
    while True:
        lim = limits(k)
        if lim is not None or k == 0: break
        k -= sign
    lo, hi = lim if lim else (F(0), F(0))
    delta = min(max(req, lo), hi)
    sn = a.get("snap")
    if sn and sn.get("enabled", True):
        thr = fr(sn["threshold_frames"]); T = targets_for(tracks, members, sn); best = None
        for (t, typ, pri) in T:
            for tr, c in mem:
                for off in (0, c["dur"]):
                    cur = c["start"] + off + delta; d = abs(cur - t)
                    nd = delta + (t - cur)
                    if d <= thr and lo <= nd <= hi:
                        key = (d, pri, t)
                        if best is None or key < best[0]: best = (key, nd)
        if best: delta = best[1]
    return {"delta_time": delta, "delta_tracks": k}

def res_placement(tracks, order, a):
    st = a["strategy"]; kind = a["kind"]; spans = [(fr(s), fr(d)) for s, d in a["spans"]]
    same = [tracks[t] for t in order if tracks[t]["kind"] == kind]
    top = same[0]["id"] if same else None
    def free(tr): return not tr["locked"] and all(not overlaps(tr, s, s + d) for s, d in spans)
    def newtrack():
        if kind == "visual": return {"kind": "new_track", "track_kind": "visual", "insert": "above", "relative_to": top}
        return {"kind": "new_track", "track_kind": "audio", "insert": "below", "relative_to": same[-1]["id"] if same else None}
    if st["type"] == "explicit":
        if st["track"] not in tracks: raise E("NOT_FOUND")
        tr = tracks[st["track"]]
        if tr["kind"] != kind: raise E("WRONG_TRACK_KIND")
        return {"kind": "existing", "track": tr["id"]}
    if st["type"] == "first_available":
        for tr in same:
            if not tr["magnetic"] and free(tr): return {"kind": "existing", "track": tr["id"]}
        return newtrack()
    if st["type"] == "prefer":
        tr = tracks.get(st["track"])
        if tr and tr["kind"] == kind and not tr["magnetic"] and free(tr): return {"kind": "existing", "track": tr["id"]}
        if tr and tr["kind"] == kind:
            return {"kind": "new_track", "track_kind": kind, "insert": "above" if kind == "visual" else "below", "relative_to": tr["id"]}
        return newtrack()
    if st["type"] == "always_new": return newtrack()
    raise E("INVALID_ARGUMENT")

CMDS = {"insert_clip": cmd_insert_clip, "delete_clip": cmd_delete_clip, "trim_clip": cmd_trim_clip, "split_clip": cmd_split_clip,
        "set_clip_speed": cmd_set_clip_speed, "add_keyframe": cmd_add_keyframe, "move_keyframe": cmd_move_keyframe}
PURE = {"resolve_snap": res_snap, "resolve_group_move": res_group_move, "resolve_placement": res_placement}

def snapshot(tracks):
    return {tid: [(c["id"], c["start"], c["dur"], c["src_in"], c["speed"], c["reversed"],
                   {p: [(k["t"], k["v"]) for k in kk] for p, kk in c["kf"].items()}) for c in sorted(t["clips"], key=lambda c: c["start"])]
            for tid, t in tracks.items()}

def approx(a, b): return abs(float(a) - float(b)) < 1e-9

def run(sc):
    tracks, order = load(sc["given"]); before = snapshot(tracks); fps = F(sc.get("fps", "30")); then = sc["then"]
    steps = sc["when"] if isinstance(sc["when"], list) else [sc["when"]]
    err = None; result = None
    try:
        for st in steps:
            if st["cmd"] in CMDS: CMDS[st["cmd"]](tracks, st["args"])
            elif st["cmd"] == "threshold_frames": result = res_threshold(st["args"])
            elif st["cmd"] == "eval_keyframes":
                tr, c = find(tracks, st["args"]["clip"]); result = {"values": [eval_clip(c, st["args"]["prop"], fr(t)) for t in st["args"]["at"]]}
            elif st["cmd"] in PURE: result = PURE[st["cmd"]](tracks, order, st["args"])
            else: raise AssertionError("unknown cmd " + st["cmd"])
    except E as e: err = e.code
    probs = []
    if "error" in then:
        if err != then["error"]: probs.append(f"expected error {then['error']} got {err}")
        if snapshot(tracks) != before and sc["area"] != "keyframes_stepped": probs.append("state changed despite error (atomicity)")
        return probs
    if err: return [f"unexpected error {err}"]
    exp_t = then.get("tracks", {})
    for tid, lst in exp_t.items():
        got = [[c["id"], c["start"], c["dur"]] for c in sorted(tracks[tid]["clips"], key=lambda c: c["start"])]
        want = [[i, fr(s), fr(d)] for i, s, d in lst]
        if got != want: probs.append(f"track {tid}: got {[[g[0], str(g[1]), str(g[2])] for g in got]} want {[[w[0], str(w[1]), str(w[2])] for w in want]}")
    after, bef = snapshot(tracks), before
    timing = lambda snap: {t: [(c[0], c[1], c[2]) for c in v] for t, v in snap.items()}
    ta, tb = timing(after), timing(bef)
    for tid in tracks:
        if tid not in exp_t and ta[tid] != tb[tid]: probs.append(f"track {tid} timing changed but not expected")
    for cid, f in then.get("clips", {}).items():
        tr, c = find(tracks, cid)
        for k, v in f.items():
            if k == "kf_count": g = sum(len(x) for x in c["kf"].values())
            elif k in ("src_in", "speed"): g = c[k]; v = fr(v)
            else: g = c[k]
            if g != v: probs.append(f"clip {cid}.{k}: got {g} want {v}")
    for cid, props in then.get("kfs", {}).items():
        tr, c = find(tracks, cid)
        for prop, lst in props.items():
            got = [(k["t"], k["v"]) for k in c["kf"].get(prop, [])]
            if len(got) != len(lst) or any(g[0] != fr(w[0]) or not approx(g[1], w[1]) for g, w in zip(got, lst)): probs.append(f"kfs {cid}.{prop}: got {[(str(a), b) for a, b in got]} want {lst}")
            else:
                for k, w in zip(c["kf"][prop], lst):
                    if len(w) > 2 and k["i"] != w[2]: probs.append(f"kf interp {k['i']} != {w[2]}")
    for cid, v in then.get("static", {}).items():
        tr, c = find(tracks, cid)
        for prop, w in v.items():
            if not approx(c["static"][prop], w): probs.append(f"static {cid}.{prop}: {c['static'][prop]} != {w}")
    for cid, (s, d) in then.get("ticks", {}).items():
        tr, c = find(tracks, cid); ft = TPS * 1001 / 30000 if sc.get("fps") == "30000/1001" else TPS / fps
        if c["start"] * ft != s or c["dur"] * ft != d: probs.append(f"ticks {cid}: {c['start']*ft},{c['dur']*ft} want {s},{d}")
    if "result" in then:
        r = result
        for k, w in then["result"].items():
            g = r.get(k)
            if isinstance(g, F): g = g; w = fr(w) if not isinstance(w, (dict, list, type(None))) else w
            if isinstance(g, dict) and isinstance(w, dict):
                g = {a: (fr(b) if isinstance(b, F) else b) for a, b in g.items()}; w = {a: (fr(b) if isinstance(b, (int, str)) and a == "t" else b) for a, b in w.items()}
            if g != w and not (isinstance(g, (int, float)) and isinstance(w, (int, float)) and approx(g, w)): probs.append(f"result.{k}: got {g} want {w}")
    if "all_within" in then:
        lo, hi = then["all_within"]["range"]
        if not all(lo <= v <= hi for v in result["values"]): probs.append("value outside property range")
    if "values" in then:
        got = result["values"]; want = then["values"]
        if len(got) != len(want) or not all(approx(g, w) for g, w in zip(got, want)): probs.append(f"values got {got} want {want}")
    return probs

def main():
    d = sys.argv[1] if len(sys.argv) > 1 else "../../tests/acceptance/timeline"
    total = bad = 0; seen = set(); by_area = {}
    for f in sorted(glob.glob(os.path.join(d, "*.json"))):
        for sc in json.load(open(f)):
            total += 1
            if sc["id"] in seen: print("DUPLICATE ID", sc["id"]); bad += 1
            seen.add(sc["id"]); by_area[sc["area"]] = by_area.get(sc["area"], 0) + 1
            p = run(sc)
            if p: bad += 1; print(f"FAIL {sc['id']} {sc['title']}\n   " + "\n   ".join(p))
    print(f"\n{total - bad}/{total} scenarios consistent with oracle  | by area: {by_area}")
    sys.exit(1 if bad else 0)
main()
