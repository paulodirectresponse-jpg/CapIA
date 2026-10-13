import { chromium } from "playwright-core";
import fs from "fs";
export const CH = fs.readdirSync("/opt/pw-browsers").filter(d=>d.startsWith("chromium-")).map(d=>`/opt/pw-browsers/${d}/chrome-linux/chrome`)[0];
export async function boot({storage}={}) {
  const b = await chromium.launch({ executablePath: CH, args:(process.env.PWARGS?process.env.PWARGS.split(" "):["--use-gl=angle","--use-angle=swiftshader","--enable-unsafe-swiftshader","--autoplay-policy=no-user-gesture-required"]) });
  const ctx = await b.newContext({ viewport:{width:1700,height:950}, acceptDownloads:true, storageState: storage });
  const page = await ctx.newPage();
  const logs=[]; page.on("console",m=>{ if(["error"].includes(m.type())) logs.push(m.text().slice(0,240)); }); page.on("pageerror",e=>logs.push("PAGEERROR "+String(e).slice(0,400)));
  return {b,ctx,page,logs};
}
export const BASE="http://localhost:3100";
export async function openProject(page){
  await page.goto(BASE+"/projects",{waitUntil:"networkidle"});
  await page.getByRole("button",{name:/new project|create your first project/i}).first().click();
  await page.waitForURL(/editor\//);
  await page.waitForTimeout(2500);
  const x = page.locator('button:has(svg.lucide-x), [aria-label=Close]').first();
  for (let i=0;i<3;i++){ const c=page.getByRole("button",{name:/^close$/i}); if(await c.count()){ await c.first().click().catch(()=>{}); await page.waitForTimeout(400);} }
  await page.keyboard.press("Escape");
}
export const S = (page)=>page.evaluate(()=>JSON.stringify(window.__capia.api.inspect_tracks().map(t=>({z:t.zone+t.index,id:t.trackId.slice(0,4),t:t.type,els:t.elements.map(e=>`${e.name.slice(0,8)}|${e.id.slice(0,4)}|${e.startSec.toFixed(2)}+${e.durSec.toFixed(2)}`)}))));
export async function importAll(page){
  const [fc] = await Promise.all([page.waitForEvent("filechooser"), page.getByRole("button",{name:/^import/i}).first().click()]);
  await fc.setFiles(["/tmp/oc/media/main.webm","/tmp/oc/media/broll.webm","/tmp/oc/media/img.png","/tmp/oc/media/music.ogg"]);
  await page.waitForFunction(()=>window.__capia?.api.inspect_project().assets.length>=4,null,{timeout:30000});
  await page.waitForTimeout(1500);
}
export const asset = (page,name)=>page.locator('[class*="cursor"], div').filter({hasText:new RegExp("^"+name)}).last();
export const PPS=390, X0=126;
export async function dragAsset(page,name,x,y){
  const o=page.getByRole("option").filter({hasText:new RegExp(name.replace(".","\\."))}).first(); const bb=await o.boundingBox();
  await page.mouse.move(bb.x+bb.width/2,bb.y+bb.height/2); await page.mouse.down();
  await page.mouse.move(bb.x+bb.width/2+30,bb.y+bb.height/2+30,{steps:4}); await page.mouse.move(x,y,{steps:14}); await page.waitForTimeout(250); await page.mouse.up(); await page.waitForTimeout(1000);
}
export const J=(x)=>JSON.stringify(x);
export const results=[]; export function rec(n,s,d=""){ results.push({n,s,d:String(d).slice(0,400)}); console.log(`[${s}] ${n} :: ${String(d).slice(0,200)}`); }
export const api=(page,fn,arg)=>page.evaluate(([f,a])=>window.__capia.api[f](a),[fn,arg]);
export const el=(page,namePrefix)=>page.evaluate((n)=>{const t=window.__capia.editor.scenes.getActiveScene().tracks;const all=[...t.overlay,t.main,...t.audio].flatMap(x=>x.elements);const e=all.find(e=>e.name.startsWith(n)||e.type===n);return e?JSON.parse(JSON.stringify(e,(k,v)=>k==="buffer"?undefined:v)):null;},namePrefix);
export async function bootPersistent(dir){
  const ctx = await chromium.launchPersistentContext(dir,{ executablePath: CH, viewport:{width:1700,height:950}, acceptDownloads:true, args:["--use-gl=angle","--use-angle=swiftshader","--enable-unsafe-swiftshader","--autoplay-policy=no-user-gesture-required"] });
  const page = ctx.pages()[0] ?? await ctx.newPage();
  const logs=[]; page.on("console",m=>{ if(m.type()==="error") logs.push(m.text().slice(0,240)); }); page.on("pageerror",e=>logs.push("PAGEERROR "+String(e).slice(0,400)));
  return {b:{close:()=>ctx.close()},ctx,page,logs};
}
