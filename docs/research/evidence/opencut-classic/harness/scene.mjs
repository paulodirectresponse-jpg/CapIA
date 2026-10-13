import {boot,openProject,S,importAll,dragAsset,rec,results,PPS,X0,api} from "./lib.mjs";
export async function setup(){
  const c=await boot(); const {page}=c;
  await openProject(page); await importAll(page);
  await dragAsset(page,"main.webm",400,620);
  await dragAsset(page,"broll.webm",600,570);
  await dragAsset(page,"music.ogg",300,720);
  await dragAsset(page,"img.png",900,540);
  return c;
}
export const st=async(page)=>JSON.parse(await S(page));
export const flat=async(page)=>(await page.evaluate(()=>window.__capia.api.inspect_tracks())).flatMap(t=>t.elements.map(e=>({...e,zone:t.zone+t.index,trackId:t.trackId})));
export const seek=async(page,t)=>{ await page.mouse.click(X0+t*PPS,550); await page.waitForTimeout(250); };
