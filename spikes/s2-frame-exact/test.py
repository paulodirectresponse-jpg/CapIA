#!/usr/bin/env python3
"""S2 spike harness (throwaway). Ground truth comes from how the clips were GENERATED, not from ffprobe/ffmpeg timestamps.
frame_at(t) = last frame with pts <= t  (sample-and-hold, ADR-007)."""
import subprocess, random, json, sys, os
from fractions import Fraction
from concurrent.futures import ThreadPoolExecutor
D = "/tmp/claude-0/s2"; TPS = 705_600_000; W, H = 640, 360
random.seed(7)

def run(cmd, binary=False):
    r = subprocess.run(cmd, shell=True, capture_output=True); return r.stdout if binary else r.stdout.decode()

def decode_id(raw):  # raw = gray W*H bytes ; 16 blocks of 40px at y=32
    v = 0
    for i in range(16):
        px = raw[32 * W + 20 + 40 * i]
        if px > 128: v |= (1 << i)
    return v

def ground_truth(name):
    """returns list of (original_index N, pts Fraction seconds) for surviving frames, from the generation recipe"""
    if name == "cfr2997": return [(n, Fraction(n * 1001, 30000)) for n in range(600)]
    if name == "cfr24":   return [(n, Fraction(n, 24)) for n in range(480)]
    if name == "vfr":     return [(n, Fraction(n, 30)) for n in range(600) if (n * 7919) % 100 < 65 or n == 0]
    if name == "vfr_bursty": return [(n, Fraction(n, 30)) for n in range(600) if ((n % 150) < 20 if True else 0) or (n % 150) == 75 or n == 0]

def expected_at(gt, t):  # sample-and-hold on ground truth
    best = None
    for n, p in gt:
        if p <= t: best = n
        else: break
    return best

def probe_index(path):
    out = run(f"ffprobe -v error -select_streams v:0 -show_entries packet=pts,duration -show_entries stream=time_base -of json {path}")
    j = json.loads(out); tb = j["streams"][0]["time_base"]; num, den = map(int, tb.split("/"))
    pts = sorted(int(p["pts"]) for p in j["packets"])
    return Fraction(num, den), pts

def frame_by_ordinal(path, k):
    raw = run(f"ffmpeg -v error -i {path} -vf select='eq(n\\,{k})' -frames:v 1 -fps_mode passthrough -f rawvideo -pix_fmt gray -", True)
    return decode_id(raw) if len(raw) >= W * H else None

def frame_naive_ss(path, t):
    raw = run(f"ffmpeg -v error -ss {float(t):.6f} -i {path} -frames:v 1 -f rawvideo -pix_fmt gray -", True)
    return decode_id(raw) if len(raw) >= W * H else None

report = {}
for name in ["cfr2997", "cfr24", "vfr", "vfr_bursty"]:
    path = f"{D}/{name}.mp4"; gt = ground_truth(name); res = {}
    # A. pts -> ticks exactness (rational, no floats) and agreement with ground truth
    tb, pts = probe_index(path); first = pts[0]
    ticks = [Fraction(p - first) * tb * TPS for p in pts]
    res["index_frames"] = len(pts); res["gt_frames"] = len(gt)
    res["ticks_all_integer"] = all(t.denominator == 1 for t in ticks)
    gtticks = [p * TPS for _, p in gt]
    res["index_matches_ground_truth"] = (len(ticks) == len(gtticks)) and all(a == b for a, b in zip(ticks, gtticks))
    res["max_err_ticks"] = int(max((abs(a - b) for a, b in zip(ticks, gtticks)), default=0)) if len(ticks) == len(gtticks) else None
    res["time_base"] = str(tb)
    # B. random times: index-based (ours) vs naive -ss
    dur = gt[-1][1] + Fraction(1, 30)
    ts = [Fraction(random.randrange(0, int(dur * 1000)), 1000) for _ in range(60)]
    def ours(t):
        exp = expected_at(gt, t)
        # index lookup: ordinal = number of frames with pts <= t, minus 1
        k = sum(1 for x in ticks if x <= t * TPS) - 1
        return exp, frame_by_ordinal(path, k)
    def naive(t): return expected_at(gt, t), frame_naive_ss(path, t)
    with ThreadPoolExecutor(4) as ex:
        o = list(ex.map(ours, ts)); n = list(ex.map(naive, ts))
    res["by_index_exact"] = f"{sum(1 for e, a in o if e == a)}/{len(o)}"
    res["naive_ss_exact"] = f"{sum(1 for e, a in n if e == a)}/{len(n)}"
    res["naive_ss_examples"] = [(float(t), e, a) for t, (e, a) in zip(ts, n) if e != a][:3]
    report[name] = res

# C. conforming to a 29.97 timeline from VFR source: our rule vs ffmpeg fps filter
gt = ground_truth("vfr"); outn = 240
ours_ids = [expected_at(gt, Fraction(n * 1001, 30000)) for n in range(outn)]
raw = run(f"ffmpeg -v error -i {D}/vfr.mp4 -vf fps=30000/1001 -frames:v {outn} -f rawvideo -pix_fmt gray -", True)
ff_ids = [decode_id(raw[i * W * H:(i + 1) * W * H]) for i in range(len(raw) // (W * H))]
mism = sum(1 for a, b in zip(ours_ids, ff_ids) if a != b)
report["conform_vfr_to_2997"] = {"frames": len(ff_ids), "ffmpeg_fps_filter_differs_from_sample_and_hold": f"{mism}/{len(ff_ids)}"}

# D. audio sync: AAC priming / edit list. burst starts at exactly 1.000 s => sample 48000
run(f"ffmpeg -v error -y -f lavfi -i \"aevalsrc='if(between(t,1,1.02),sin(2*PI*1000*t),0)':s=48000:d=3\" -i {D}/cfr24.mp4 -t 3 -c:v copy -c:a aac -b:a 128k -shortest {D}/av.mp4")
def first_onset(extra):
    raw = run(f"ffmpeg -v error {extra} -i {D}/av.mp4 -vn -f s16le -ac 1 -ar 48000 -", True)
    import array; a = array.array("h"); a.frombytes(raw[: len(raw) // 2 * 2])
    for i, s in enumerate(a):
        if abs(s) > 3000: return i
audio = {"expected_sample": 48000, "libav_default(edit list honored)": first_onset(""), "ignore_editlist": first_onset("-ignore_editlist 1")}
audio["error_samples_default"] = audio["libav_default(edit list honored)"] - 48000
audio["error_ms_default"] = round(audio["error_samples_default"] / 48, 3)
audio["error_ms_if_edit_list_ignored"] = round((audio["ignore_editlist"] - 48000) / 48, 3)
report["audio_sync"] = audio

# E. rotation metadata is NOT applied by libavcodec; engine must apply display matrix
run(f"ffmpeg -v error -y -display_rotation:v:0 90 -i {D}/cfr24.mp4 -c copy {D}/rot.mp4")
report["rotation_probe"] = run(f"ffprobe -v error -select_streams v:0 -show_entries stream_side_data=rotation -of csv=p=0 {D}/rot.mp4").strip()

print(json.dumps(report, indent=2, default=str))
json.dump(report, open(f"{D}/report.json", "w"), indent=2, default=str)
