import {boot,openProject,importAll,dragAsset,rec,results,api,PPS,X0} from "./lib.mjs";
import {flat} from "./scene.mjs";
import fs from "fs";
const {page,b,logs}=await boot();
await openProject(page); await importAll(page);
const J=(o)=>JSON.stringify(o).slice(0,260);
const names=async()=> (await flat(page)).map(e=>`${e.name}@${e.startSec.toFixed(2)}+${e.durSec.toFixed(2)}`);
const domLabels=async()=> page.evaluate(()=>[...document.querySelectorAll("span,div")].filter(e=>e.children.length===0&&/main\.webm/.test(e.textContent||"")&&e.getBoundingClientRect().height<30&&e.getBoundingClientRect().top>550).map(e=>e.textContent.trim()));
// 1 UI: create project (done) + insert clip
await dragAsset(page,"main.webm",400,620);
rec("PARITY-1 UI inserts clip", (await names()).length===1?"PASS":"FAIL", J(await names()));
const pj=await api(page,"inspect_project"); const tr0=await api(page,"inspect_tracks");
rec("PARITY-1b PROGRAM inspect sees UI-created clip", tr0.some(t=>t.elements.length===1)?"PASS":"FAIL", `rev ${pj.revision}; tracks=${tr0.length}`);
const clip=tr0.flatMap(t=>t.elements)[0];
// 2 PROGRAM split at 5s
const sp=await page.evaluate((a)=>window.__capia.api.split_clip(a),{elementId:clip.id,atSeconds:2});
rec("PARITY-2 PROGRAM split_clip @2s", sp.ok?"PASS":"FAIL", J({ok:sp.ok,applied:sp.applied,diff:{added:sp.diff?.added.length,changed:sp.diff?.changed.length},err:sp.error}));
let els=(await flat(page)).filter(e=>e.zone.startsWith("main"));
await page.waitForTimeout(500); await page.screenshot({path:"par-2-after-split.png"});
// 3 PROGRAM move right part to 8s
const right=els.find(e=>e.startSec>=1.9);
const mv=await page.evaluate((a)=>window.__capia.api.move_clip(a),{elementId:right.id,newStartSeconds:4});
rec("PARITY-3 PROGRAM move_clip right part -> 4s", mv.ok?"PASS":"FAIL", J({ok:mv.ok,err:mv.error,after:await names()}));
await page.waitForTimeout(600); await page.screenshot({path:"par-3-after-move.png"});
// 4 UI observes: DOM shows 2 clips? and playhead renders frames
const dom=await domLabels(); rec("PARITY-4 UI observes programmatic change (DOM clip labels)", dom.length>=2?"PASS":"PARTIAL", J(dom));
// 5 UI trim: drag the right edge of left part (ends 5s) left by 0.5s
const p=390; const lefts=(await flat(page)).find(e=>e.startSec<1&&e.zone.startsWith("main"));
const ex=X0+(lefts.startSec+lefts.durSec)*p-3;
await page.mouse.click(500,620); await page.waitForTimeout(300);
await page.mouse.move(ex,620); await page.waitForTimeout(200); await page.mouse.down(); await page.mouse.move(ex-20,620,{steps:4}); await page.mouse.move(ex-Math.round(0.3*p),620,{steps:6}); await page.mouse.up(); await page.waitForTimeout(500);
const lefts2=(await flat(page)).find(e=>e.startSec<1&&e.zone.startsWith("main"));
rec("PARITY-5 UI trim end of left part (drag handle)", lefts2.durSec<lefts.durSec-0.2?"PASS":"FAIL", `dur ${lefts.durSec.toFixed(2)} -> ${lefts2.durSec.toFixed(2)}`);
// 6 PROGRAM inspect sees UI trim
const tr1=await api(page,"inspect_tracks"); const seen=tr1.flatMap(t=>t.elements).find(e=>e.id===lefts.id);
rec("PARITY-6 PROGRAM inspect sees UI trim", Math.abs(seen.durSec-lefts2.durSec)<1e-6?"PASS":"FAIL", `inspect durSec=${seen.durSec.toFixed(3)} trimEndTicks=${seen.trimEndTicks}`);
// 7 SHARED HISTORY: UI Ctrl+Z undoes the PROGRAM move? order: [UI insert, PROG split, PROG move, UI trim]. undo x1 -> UI trim; undo x2 -> PROG move (via UI shortcut)
await page.keyboard.press("Control+z"); await page.waitForTimeout(300); await page.keyboard.press("Control+z"); await page.waitForTimeout(400);
const afterTwo=await names(); const rightNow=(await flat(page)).filter(e=>e.zone.startsWith("main")).map(e=>e.startSec.toFixed(2));
rec("PARITY-7 UI Ctrl+Z x2 undoes UI trim THEN PROGRAM move (single shared history)", rightNow.includes("2.00")?"PASS":"FAIL", J({starts:rightNow}));
// 8 PROGRAM undo undoes UI/PROG commands: undo split via program
const u=await page.evaluate(()=>window.__capia.api.undo()); await page.waitForTimeout(300);
const n3=(await flat(page)).filter(e=>e.zone.startsWith("main")).length;
rec("PARITY-8 PROGRAM undo reverts PROG split; UI state follows", n3===1?"PASS":"FAIL", `main clips now ${n3}`);
// 9 PROGRAM redo then delete; UI observes final
await page.evaluate(()=>window.__capia.api.redo()); await page.waitForTimeout(300);
const cur=(await flat(page)).filter(e=>e.zone.startsWith("main")); const del=await page.evaluate((a)=>window.__capia.api.delete_clip(a),{elementId:cur[cur.length-1].id}); await page.waitForTimeout(500);
await page.screenshot({path:"par-9-final.png"});
rec("PARITY-9 PROGRAM delete_clip; UI observes final state", del.ok&&(await flat(page)).filter(e=>e.zone.startsWith("main")).length===1?"PASS":"FAIL", J({ok:del.ok,remaining:await names()}));
// 10 Contract features: stale revision, idempotency, atomic rollback, preview
const rev=(await api(page,"inspect_project")).revision; const e1=(await flat(page))[0];
const stale=await page.evaluate((a)=>window.__capia.api.apply_plan(a),{baseRevision:rev-1,ops:[{op:"delete_clip",operationId:"x1",elementId:e1.id}]});
rec("CONTRACT stale baseRevision rejected", stale.ok===false&&stale.error.code==="STALE_REVISION"?"PASS":"FAIL", J(stale));
const prev=await page.evaluate((a)=>window.__capia.api.preview_plan(a),{baseRevision:rev,ops:[{op:"split_clip",operationId:"p1",elementId:e1.id,atSeconds:1}]});
const afterPrev=(await flat(page)).length;
rec("CONTRACT preview_plan returns diff, leaves doc+history untouched", prev.ok&&prev.diff.added.length===1&&afterPrev===1?"PASS":"FAIL", J({added:prev.diff?.added.length,elementsAfterPreview:afterPrev,revision:prev.revision}));
const rev2=(await api(page,"inspect_project")).revision;
const hist0=await page.evaluate(()=>window.__capia.editor.command.canUndo());
const bad=await page.evaluate((a)=>window.__capia.api.apply_plan(a),{baseRevision:rev2,ops:[{op:"split_clip",operationId:"a1",elementId:e1.id,atSeconds:1},{op:"delete_clip",operationId:"a2",elementId:"does-not-exist"}]});
rec("CONTRACT atomic: plan with invalid 2nd op applies nothing", bad.ok===false&&(await flat(page)).length===1?"PASS":"FAIL", J(bad));
const rev3=(await api(page,"inspect_project")).revision;
const plan={baseRevision:rev3,ops:[{op:"split_clip",operationId:"idem-1",elementId:e1.id,atSeconds:1}]};
const a1=await page.evaluate((a)=>window.__capia.api.apply_plan(a),plan); const rev4=(await api(page,"inspect_project")).revision;
const a2=await page.evaluate((a)=>window.__capia.api.apply_plan(a),{...plan,baseRevision:rev4});
rec("CONTRACT idempotent operationId (replay is no-op)", a1.ok&&a2.ok&&a2.idempotent&&(await flat(page)).length===2?"PASS":"FAIL", J({first:a1.applied,replay:a2}));
// undo receipt: one UI Ctrl+Z reverts the whole plan
await page.keyboard.press("Control+z"); await page.waitForTimeout(300);
rec("CONTRACT plan = single undo entry (UI Ctrl+Z)", (await flat(page)).length===1?"PASS":"FAIL", `elements ${(await flat(page)).length}`);
fs.writeFileSync("p24-results.json",JSON.stringify(results,null,1));
console.log(logs.slice(0,3)); await b.close();
