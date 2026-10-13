import {boot,openProject,importAll,rec,results,PPS,X0} from "./lib.mjs";
import fs from "fs";
const Ns=process.env.NS?process.env.NS.split(",").map(Number):[100,500,1000,5000,10000];
const out=[];
for (const N of Ns) {
  const {page,b,logs}=await boot();
  await openProject(page);
  const [fc]=await Promise.all([page.waitForEvent("filechooser"),page.getByRole("button",{name:/^import/i}).first().click()]);
  await fc.setFiles(["/tmp/oc/media/main.webm"]);
  await page.waitForFunction(()=>window.__capia?.api.inspect_project().assets.length>=1,null,{timeout:30000});
  await page.waitForTimeout(800);
  const row={N};
  try {
  const r=await page.evaluate(async(N)=>{
    const ed=window.__capia.editor; const mediaId=ed.media.getAssets()[0].id; const sc=ed.scenes.getActiveScene();
    const TICKS=120000; const per=50; const nTracks=Math.max(1,Math.ceil(N/per));
    const mk=(ti,ci)=>({id:`e-${ti}-${ci}`,type:"video",mediaId,name:`c${ti}-${ci}`,duration:Math.round(0.5*TICKS),startTime:Math.round(ci*0.6*TICKS),trimStart:0,trimEnd:0,sourceDuration:20*TICKS,params:{"transform.positionX":0,"transform.positionY":0,"transform.scaleX":1,"transform.scaleY":1,"transform.rotate":0,"opacity":1,"blendMode":"normal","volume":0,"muted":false},isSourceAudioEnabled:true,hidden:false});
    const overlay=[]; let left=N; for(let t=0;t<nTracks;t++){ const n=Math.min(per,left); left-=n; overlay.push({id:`t-${t}`,name:`T${t}`,type:"video",muted:false,hidden:false,elements:Array.from({length:n},(_,i)=>mk(t,i))}); }
    const tracks={...sc.tracks,overlay};
    const lt=[]; const po=new PerformanceObserver(l=>l.getEntries().forEach(e=>lt.push(e.duration))); try{po.observe({entryTypes:["longtask"]})}catch{}
    const t0=performance.now(); ed.timeline.updateTracks(tracks); await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))); await new Promise(r=>setTimeout(r,500));
    const loadMs=performance.now()-t0;
    return {loadMs:Math.round(loadMs), dom:document.querySelectorAll("*").length, heapMB: performance.memory?Math.round(performance.memory.usedJSHeapSize/1e6):null, longTaskMaxMs:Math.round(Math.max(0,...lt)), longTasks:lt.length, nTracks};
  },N);
  Object.assign(row,r);
  // scroll/zoom frames
  const fr=await page.evaluate(async()=>{ const ds=[]; let last=performance.now(); let n=0; return await new Promise(res=>{ function f(t){ ds.push(t-last); last=t; if(++n<40) requestAnimationFrame(f); else {ds.shift(); ds.sort((a,b)=>a-b); res({p50:Math.round(ds[Math.floor(ds.length/2)]),p95:Math.round(ds[Math.floor(ds.length*0.95)]),max:Math.round(ds[ds.length-1])});} } requestAnimationFrame(f); }); });
  const wheel=async()=>{ const p=page.evaluate(async()=>{ const ds=[]; let last=performance.now(); let n=0; return await new Promise(res=>{ function f(t){ ds.push(t-last); last=t; if(++n<60) requestAnimationFrame(f); else {ds.shift(); ds.sort((a,b)=>a-b); res({p50:Math.round(ds[Math.floor(ds.length/2)]),p95:Math.round(ds[Math.floor(ds.length*0.95)]),max:Math.round(ds[ds.length-1])});} } requestAnimationFrame(f); }); }); for(let i=0;i<12;i++){ await page.mouse.move(800,700); await page.mouse.wheel(300,0); await page.waitForTimeout(40);} return await p; };
  row.idleFrame=fr; row.scrollFrame=await wheel();
  // zoom
  row.zoomMs=await page.evaluate(async()=>{ const t0=performance.now(); document.querySelector('button svg')?.closest("button"); return null; });
  const z=Date.now(); await page.mouse.click(1517,520); await page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))); row.zoomOutMs=Date.now()-z;
  // command timings (same API UI uses) : split / move / trim / delete + undo
  const cmd=await page.evaluate(async()=>{ const ed=window.__capia.editor; const tr=ed.scenes.getActiveScene().tracks.overlay[0]; const el=tr.elements[0]; const T=120000; const m=async(f)=>{const t0=performance.now(); f(); await new Promise(r=>requestAnimationFrame(r)); return Math.round((performance.now()-t0)*10)/10;};
    const split=await m(()=>ed.timeline.splitElements({elements:[{trackId:tr.id,elementId:el.id}],splitTime:Math.round(0.25*T)}));
    const move=await m(()=>ed.timeline.moveElements({moves:[{sourceTrackId:tr.id,targetTrackId:tr.id,elementId:tr.elements[1].id,newStartTime:Math.round(500*T)}]}));
    const trim=await m(()=>ed.timeline.updateElementTrim({elementId:tr.elements[2].id,trimStart:Math.round(0.1*T),trimEnd:0,startTime:Math.round(tr.elements[2].startTime+0.1*T),duration:Math.round(0.4*T)}));
    const del=await m(()=>ed.timeline.deleteElements({elements:[{trackId:tr.id,elementId:tr.elements[3].id}]}));
    const undo=await m(()=>ed.command.undo());
    const t0=performance.now(); const ser=JSON.stringify(ed.scenes.getScenes()); const serMs=Math.round(performance.now()-t0);
    return {split,move,trim,del,undo,serializeMs:serMs,jsonMB:+(ser.length/1e6).toFixed(2)}; });
  row.cmd=cmd;
  // save to IndexedDB
  row.saveMs=await page.evaluate(async()=>{ const t0=performance.now(); await window.__capia.editor.project.saveCurrentProject(); return Math.round(performance.now()-t0); });
  } catch(e){ row.error=String(e).slice(0,200); }
  console.log(JSON.stringify(row)); out.push(row); fs.writeFileSync("perf-results.json",JSON.stringify(out,null,1));
  await b.close();
}
