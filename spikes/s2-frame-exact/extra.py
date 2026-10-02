#!/usr/bin/env python3
"""S2 extras: (1) conform VFR source to other output rates: ffmpeg fps filter vs sample-and-hold; (2) index build cost for a 10 min clip."""
import subprocess, time, json
from fractions import Fraction
D="/tmp/claude-0/s2"; W,H=640,360
def run(c,b=False):
    r=subprocess.run(c,shell=True,capture_output=True); return r.stdout if b else r.stdout.decode()
def decode_id(raw):
    v=0
    for i in range(16):
        if raw[32*W+20+40*i]>128: v|=1<<i
    return v
gt=[(n,Fraction(n,30)) for n in range(600) if (n*7919)%100<65 or n==0]
def exp(t):
    b=None
    for n,p in gt:
        if p<=t: b=n
        else: break
    return b
out={}
for label,(num,den) in {"23.976":(24000,1001),"24":(24,1),"25":(25,1),"29.97":(30000,1001),"50":(50,1),"59.94":(60000,1001)}.items():
    N=300
    raw=run(f"ffmpeg -v error -i {D}/vfr.mp4 -vf fps={num}/{den} -frames:v {N} -f rawvideo -pix_fmt gray -",True)
    ids=[decode_id(raw[i*W*H:(i+1)*W*H]) for i in range(len(raw)//(W*H))]
    mine=[exp(Fraction(n*den,num)) for n in range(len(ids))]
    out[label]={"frames":len(ids),"fps_filter_differs":sum(1 for a,b in zip(ids,mine) if a!=b)}
print(json.dumps(out)) 
# index cost
run(f"ffmpeg -v error -y -f lavfi -i testsrc2=size=640x360:rate=30000/1001:duration=600 -c:v libx264 -preset ultrafast -g 60 -bf 2 -crf 28 {D}/long10min.mp4")
t=time.time(); o=run(f"ffprobe -v error -select_streams v:0 -show_entries packet=pts -of csv=p=0 {D}/long10min.mp4"); dt=time.time()-t
import os
print(json.dumps({"packets":len(o.split()),"index_scan_seconds":round(dt,2),"file_MB":round(os.path.getsize(D+'/long10min.mp4')/1e6,1)}))
