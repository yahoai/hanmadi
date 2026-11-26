const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const GLYPH = {ko:"한",zh:"中",en:"EN",ja:"日",fr:"FR",de:"DE",es:"ES"};
const $ = s => document.querySelector(s);
const esc = s => String(s ?? "").replace(/[&<>"]/g,c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c]));
const fmt = s => { s=Math.max(0,s); const h=Math.floor(s/3600), m=Math.floor(s%3600/60), x=Math.floor(s%60); return (h?`${h}:${String(m).padStart(2,"0")}`:String(m).padStart(2,"0"))+":"+String(x).padStart(2,"0"); };
const mins = s => T("mins", Math.round(s/60));
const dur = s => T("dur", Math.floor(s/60), Math.round(s%60));
const dfmt = s => `${Math.floor(s/60)}:${String(Math.round(s%60)).padStart(2,"0")}`;
const ICON = {play:'<svg viewBox="0 0 16 16"><path d="M5 3.2v9.6L12.8 8z"/></svg>',pause:'<svg viewBox="0 0 16 16"><path d="M4.5 3h2.4v10H4.5zM9.1 3h2.4v10H9.1z"/></svg>',check:'<svg viewBox="0 0 16 16"><path d="M3.5 8.5 6.5 11.5 12.5 4.5"/></svg>',trash:'<svg viewBox="0 0 16 16"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5"/></svg>'};

let L = I18N[localStorage.lang] ? localStorage.lang : "en";
const T = (k,...a) => { const v=I18N[L][k] ?? I18N.en[k]; return typeof v==="function" ? v(...a) : v; };
const langName = c => new Intl.DisplayNames([L],{type:"language"}).of(c);
const dateFmt = (d,opt) => new Intl.DateTimeFormat(L,opt).format(d);
function toast(m){ const e=$("#toast"); e.textContent=m; e.classList.add("on"); clearTimeout(toast.h); toast.h=setTimeout(()=>e.classList.remove("on"),2400); }

/* ---------- navigation + list ---------- */
let list = [], doc = null, draft = null, active = null;
function go(id){ document.querySelectorAll(".screen").forEach(s=>s.classList.toggle("on",s.id===id)); if(id!=="view") setPlay(false); $("#pv").pause(); }
async function loadList(){
  list = await invoke("list_transcripts");
  const now = Date.now(), week = 7*864e5;
  const added = d => new Date(d.added || d.recorded);
  const groups = [["gProg",d=>d.status!=="done"],["gWeek",d=>d.status==="done" && now-added(d)<week],["gEarlier",d=>d.status==="done" && now-added(d)>=week]];
  $("#list").innerHTML = list.length ? groups.map(([g,f])=>{ const ds=list.filter(f); return ds.length ? `<div class="group">${T(g)}</div>`+ds.map(item).join("") : ""; }).join("")
    : `<p class="group">${T("empty")}</p>`;
}
// stopped (the app quit mid-job) and failed jobs stay under In progress, frozen, with a word instead of a moving bar
const HALT = {stopped:"stopped", error:"failed"};
function item(d){
  const run = d.status!=="done", halt = HALT[d.status];
  return `<button class="item ${d.id===active?"on":""}" data-open="${esc(d.id)}"><b>${esc(d.title)}</b><span class="dur num">${fmt(d.duration||0)}</span><span class="rm" role="button" data-rm="${esc(d.id)}" title="${T("del")}" aria-label="${T("del")}">${ICON.trash}</span>`
    + (run ? `<span class="bar ${halt?"halt":""}"><i style="width:${Math.round((d.progress?.pct||0)*100)}%"></i></span>` + (halt ? `<span class="marks"><span class="state">${T(halt)}</span></span>` : "")
           : `<span class="marks">${(d.languages||[]).map(l=>`<span class="glyph">${GLYPH[l]||l}</span>`).join("")}<span class="when">${d.recorded?dateFmt(new Date(d.recorded),{month:"short",day:"numeric"}):""}</span></span>`) + `</button>`;
}
$("#list").addEventListener("click",e=>{ const r=e.target.closest("[data-rm]"); if(r) return remove(r.dataset.rm); const b=e.target.closest("[data-open]"); if(b) open(b.dataset.open); });
// native context menu on a sidebar item
$("#list").addEventListener("contextmenu",async e=>{ const b=e.target.closest("[data-open]"); if(!b) return; e.preventDefault();
  const id=b.dataset.open, done=list.find(x=>x.id===id)?.status==="done";
  const menu = await window.__TAURI__.menu.Menu.new({items:[
    {id:"open", text:T("openT"), action:()=>open(id)},
    {id:"export", text:T("export"), enabled:done, action:()=>exportMd(id)},
    {item:"Separator"},
    {id:"del", text:`${T("del")}…`, action:()=>remove(id)},
  ]});
  menu.popup(); });
document.addEventListener("click",e=>{ const g=e.target.closest("[data-go]"); if(g){ if(g.dataset.go==="new") { active=null; loadList(); memos(); } if(g.closest(".railfoot")) { active=null; loadList(); renderPrefs(); } go(g.dataset.go); } });

async function open(id){
  active = id; loadList();
  const d = await invoke("get_transcript",{id});
  if(d.status!=="done"){ showProgress(d); return go("proc"); }
  doc = d; renderView(); go("view");
}

// deletes the app's copy (transcript + playback audio); the source recording is never touched
async function remove(id){
  const d = list.find(x=>x.id===id); if(!d) return;
  const ok = await window.__TAURI__.dialog.ask(T("delAsk",d.title),{title:T("delTitle"),kind:"warning",okLabel:T("del"),cancelLabel:T("cancel")});
  if(!ok) return;
  const order = [...document.querySelectorAll("#list [data-open]")].map(b=>b.dataset.open), at = order.indexOf(id);
  try { await invoke("delete_transcript",{id}); } catch(err){ return toast(err==="busy" ? T("delBusy") : String(err)); }
  const wasActive = active===id;
  if(wasActive){ setPlay(false); AU.removeAttribute("src"); AU.load(); delete AU.dataset.src; doc=null; active=null; }
  await loadList();
  if(wasActive){ const next = order[at+1] ?? order[at-1]; next ? open(next) : go("new"); }
  toast(T("deleted"));
}

/* ---------- static text ---------- */
function texts(){
  document.documentElement.lang = L;
  document.querySelectorAll("[data-t]").forEach(e=>{ const v=T(e.dataset.t); if(typeof v==="string") e.textContent=v; });
  document.querySelectorAll("[data-ph]").forEach(e=>e.placeholder=T(e.dataset.ph));
  loadList(); if(doc) renderView(); if(draft) renderSetup(); memos(); renderPrefs();
}

/* ---------- new: voice memos, drop, file ---------- */
$("#wave").innerHTML = Array.from({length:28},(_,i)=>`<i style="height:${8+Math.round(30*Math.abs(Math.sin(i*1.7)*Math.cos(i*.45)))}px"></i>`).join("");
let memoRows = [];
async function memos(){
  try { memoRows = await invoke("list_voice_memos"); } catch { memoRows = []; }
  const q = $("#memoq").value.trim().toLowerCase();
  const rows = memoRows.filter(r=>!q || (r.title||"").toLowerCase().includes(q));
  $("#memos").innerHTML = rows.length ? rows.map((r,i)=>`<tr class="row" data-memo="${memoRows.indexOf(r)}"><td class="pvc"><button class="quiet pvb" data-pv="${memoRows.indexOf(r)}" title="${T("listen")}" aria-label="${T("listen")}">${ICON.play}</button></td><td><b>${esc(r.title)}</b></td><td class="d">${dateFmt(new Date(r.ts*1000),{dateStyle:"medium",timeStyle:"short"})}</td><td class="d num">${fmt(r.duration)}</td><td>${list.some(d=>d.source?.path===r.path)?`<span class="state">${T("transcribed")}</span>`:""}</td></tr>`).join("")
    : `<tr><td class="d">${T("noMemos")}</td></tr>`;
  pvPaint();
}
$("#memoq").addEventListener("input",memos);
$("#memos").addEventListener("click",e=>{ const p=e.target.closest("[data-pv]"); if(p) return preview(memoRows[+p.dataset.pv].path); const r=e.target.closest("[data-memo]"); if(!r) return; const m=memoRows[+r.dataset.memo];
  startDraft({path:m.path, title:m.title, kind:"voice-memo", recorded:new Date(m.ts*1000).toISOString(), duration:m.duration}); });
$("#choose").onclick = async ()=>{ const p = await window.__TAURI__.dialog.open({multiple:false,filters:[{name:"Audio",extensions:["m4a","mp3","wav","mp4","mov","qta","aac","flac","ogg","webm"]}]}); if(p) startDraft(await fromPath(p)); };
// recording date comes from the file (creation_time tag, else the file's creation date), never "now"
async function fromPath(p){ const m = await invoke("probe_source",{path:p});
  return {path:p, title:p.split(/[\\/]/).pop().replace(/\.[^.]+$/,""), kind:"file", recorded:m.recorded || new Date().toISOString(), duration:m.duration}; }
listen("tauri://drag-enter",()=>$("#drop").classList.add("over"));
listen("tauri://drag-leave",()=>$("#drop").classList.remove("over"));
listen("tauri://drag-drop",e=>{ $("#drop").classList.remove("over"); const p=e.payload?.paths?.[0]; if(p) fromPath(p).then(startDraft); });

/* ---------- preview: listen to a recording before transcribing (#pv, separate from the transcript player) ---------- */
const PV = $("#pv");
let pvPath = null, pvFixed = false; // source loaded in #pv; whether it already went through the remux
// WebKit can't open .qta (and may choke on other containers): play a remuxed copy of the first audio track
async function pvRemux(path){ pvFixed = true; try { return await invoke("preview_audio",{path}); } catch(err){ toast(String(err)); } }
async function pvLoad(path){
  PV.pause(); PV.removeAttribute("src"); pvPath = path; pvFixed = false;
  const src = /\.qta$/i.test(path) ? await pvRemux(path) : path;
  if(src && pvPath===path) PV.src = convertFileSrc(src);
}
async function preview(path){
  if(pvPath===path && PV.src) return PV.paused ? PV.play().catch(()=>{}) : PV.pause();
  await pvLoad(path); if(pvPath===path && PV.src) PV.play().catch(()=>{});
}
function pvPaint(){
  const mine = draft && pvPath===draft.source.path, d = (mine && isFinite(PV.duration) ? PV.duration : draft?.source.duration) || 0, now = mine ? PV.currentTime : 0;
  $("#pvplay").innerHTML = mine && !PV.paused ? ICON.pause : ICON.play; $("#pvplay").ariaLabel = T("listen");
  $("#pvtime").textContent = `${fmt(now)} / ${fmt(d)}`;
  Object.assign($("#pvseek"), {max:d, value:now});
  document.querySelectorAll("#memos [data-pv]").forEach(b=>{ const on = !PV.paused && memoRows[+b.dataset.pv]?.path===pvPath;
    b.innerHTML = on ? ICON.pause : ICON.play; b.closest("tr").classList.toggle("playing",on); });
}
["play","pause","timeupdate","loadedmetadata","ended","emptied"].forEach(ev=>PV.addEventListener(ev,pvPaint));
PV.addEventListener("error", async ()=>{ if(!pvPath || pvFixed) return; const p = pvPath, src = await pvRemux(p);
  if(src && pvPath===p){ PV.src = convertFileSrc(src); PV.play().catch(()=>{}); } });
// one thing plays at a time
PV.addEventListener("play",()=>setPlay(false)); $("#au").addEventListener("play",()=>PV.pause());
$("#pvplay").onclick = ()=>preview(draft.source.path);
$("#pvseek").oninput = async e=>{ const v = +e.target.value; if(pvPath!==draft.source.path) await pvLoad(draft.source.path); PV.currentTime = v; };

/* ---------- setup ---------- */
let memory = JSON.parse(localStorage.memory || '{"vocab":[],"speakers":[]}');
// recommended target: the app language. Lines already in it are never translated, so this costs nothing
// when nobody speaks another language (participants' language hints are often incomplete)
const recTr = ()=>GLYPH[L] ? L : "en";
// whisper's prompt holds about 224 tokens and drops the front when longer; more words also invite hallucinated vocabulary
const VOCAB_MAX = 40;
function startDraft(src){
  const m = me();
  draft = {source:src, title:src.title, titleEdited:false, speakers: m.name ? [{name:m.name, langs:[...m.langs]}] : [], vocab:memory.vocab.slice(-VOCAB_MAX), trPicked:false};
  renderSetup(); go("setup");
}
function renderSetup(){
  $("#stitle").textContent = draft.title;
  const s = draft.source;
  $("#srcline").textContent = [s.kind==="voice-memo"?T("fromMemo"):s.kind==="recording"?T("fromRec"):T("fromFile"), dateFmt(new Date(s.recorded),{dateStyle:"long"}), s.duration?fmt(s.duration):""].filter(Boolean).join(", ");
  pvPaint(); recPaint();
  $("#people").innerHTML = draft.speakers.map((p,i)=>`<div class="person"><span class="dot" style="background:var(--sp${i%6})"></span>
    <input value="${esc(p.name)}" placeholder="${T("name")}" data-i="${i}">
    <div class="langs">${Object.entries(GLYPH).map(([k,g])=>`<button class="lg ${p.langs.includes(k)?"on":""}" data-i="${i}" data-l="${k}" title="${langName(k)}">${g}</button>`).join("")}</div>
    <button class="del" data-del="${i}" aria-label="Remove"><svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8"/></svg></button></div>`).join("");
  $("#terms").innerHTML = draft.vocab.map((w,i)=>`<button class="term" data-w="${i}" title="Remove">${esc(w)}</button>`).join("") + `<input id="wordin" placeholder="${T("addWord")}">`;
  const names = new Set([me().name, ...draft.speakers.map(p=>p.name.trim())]);
  const past = memory.speakers.map((p,i)=>[p,i]).filter(([p])=>!names.has(p.name));
  $("#past").innerHTML = past.length ? `<span>${T("pastPeople")}</span>` + past.map(([p,i])=>`<button class="term" data-p="${i}">${esc(p.name)}</button>`).join("") : "";
  pastTerms();
  $("#vnote").textContent = draft.vocab.length>VOCAB_MAX ? T("vocabOver",VOCAB_MAX) : memory.vocab.length>VOCAB_MAX ? T("vocabMore",memory.vocab.length,VOCAB_MAX) : "";
  const rec = recTr();
  if(!draft.trPicked) draft.translateTo = rec || null;
  $("#trsel").innerHTML = [rec, ...[...Object.keys(GLYPH),""].filter(c=>c!==rec)].map(c=>`<option value="${c}" ${c===(draft.translateTo||"")?"selected":""}>${c?`${T("trInto")}: ${langName(c)}`:T("trOff")}${c===rec?` (${T("recommended")})`:""}</option>`).join("");
  $("#eta").textContent = "";
  if(s.duration) invoke("estimate",{duration:s.duration, translate:!!draft.translateTo})
    .then(e=>$("#eta").textContent = T("eta", mins(e.transcribe), draft.translateTo ? mins(e.translate) : null));
}
// words from done meetings with the same people (not me, I'm in all of them); else the 3 latest. Ranked by meetings containing it
function pastTerms(){
  const lc = s=>String(s).trim().toLowerCase(), mine = lc(me().name), have = new Set(draft.vocab.map(lc)), n = new Map();
  const want = new Set(draft.speakers.map(p=>lc(p.name)).filter(x=>x && x!==mine)), done = list.filter(d=>d.status==="done");
  (want.size ? done.filter(d=>(d.names||[]).some(x=>want.has(lc(x)))) : done.slice(0,3)).forEach(d=>{ const seen = new Set();
    [...(d.vocab||[]), ...(d.terms||[])].forEach(w=>{ const k=lc(w); if(!k || have.has(k) || seen.has(k)) return; seen.add(k); const e=n.get(k)||{w,c:0}; e.c++; n.set(k,e); }); });
  const top = [...n.values()].sort((a,b)=>b.c-a.c).slice(0,12);
  $("#pastterms").innerHTML = top.length ? `<span>${T("pastTerms")}</span>` + top.map(e=>`<button class="term" data-v="${esc(e.w)}">${esc(e.w)}</button>`).join("") : "";
}
$("#pastterms").addEventListener("click",e=>{ const b=e.target.closest("[data-v]"); if(b){ draft.vocab.push(b.dataset.v); renderSetup(); } });
$("#stitle").addEventListener("input",e=>{ draft.title=e.target.textContent.trim(); draft.titleEdited=true; });
$("#people").addEventListener("click",e=>{
  const c=e.target.closest(".lg"), d=e.target.closest("[data-del]");
  if(c){ const p=draft.speakers[c.dataset.i], l=c.dataset.l; p.langs=p.langs.includes(l)?p.langs.filter(x=>x!==l):[...p.langs,l]; renderSetup(); }
  if(d){ draft.speakers.splice(+d.dataset.del,1); renderSetup(); }
});
$("#people").addEventListener("input",e=>{ draft.speakers[e.target.dataset.i].name=e.target.value; });
$("#people").addEventListener("change",pastTerms);
$("#addp").onclick=()=>{ draft.speakers.push({name:"",langs:[]}); renderSetup(); };
$("#terms").addEventListener("click",e=>{ const w=e.target.closest("[data-w]"); if(w){ draft.vocab.splice(+w.dataset.w,1); renderSetup(); } });
$("#terms").addEventListener("keydown",e=>{ if(e.key==="Enter" && e.target.value.trim()){ draft.vocab.push(e.target.value.trim()); renderSetup(); $("#wordin").focus(); } });
$("#trsel").onchange=e=>{ draft.translateTo=e.target.value||null; draft.trPicked=true; renderSetup(); };
$("#past").addEventListener("click",e=>{ const b=e.target.closest("[data-p]"); if(b){ const {name,langs}=memory.speakers[b.dataset.p]; draft.speakers.push({name,langs:[...langs]}); renderSetup(); } });
$("#start").onclick = async ()=>{
  if(draft.recording) await recStop();
  const speakers = draft.speakers.filter(p=>p.name.trim() || p.langs.length);
  // merge: this meeting first (latest langs win), then earlier ones; dedupe by name, skip me
  const seen = new Set([me().name]), past = [...speakers.map(p=>({name:p.name.trim(),langs:p.langs})).filter(p=>p.name), ...memory.speakers].filter(p=>!seen.has(p.name) && seen.add(p.name)).slice(0,12);
  memory = {...memory, vocab:[...memory.vocab.filter(w=>!draft.vocab.includes(w)), ...draft.vocab], speakers:past}; localStorage.memory = JSON.stringify(memory);
  const req = {...draft, vocab:draft.vocab.slice(-VOCAB_MAX), speakers, autoTitle:!draft.titleEdited, titleLang:L};
  // a stopped transcript with no stored request: run that id again with this setup
  try { const id = draft.resumeId ? (await invoke("resume_job",{id:draft.resumeId, fresh:!!draft.fresh, req}), draft.resumeId) : await invoke("start_job",{req}); draft=null; await open(id); }
  catch(err){ toast(String(err)); }
};

/* ---------- progress ---------- */
const STEP_ORDER = ["prepare","vad","diarize","transcribe","name","title","translate"];
const PHASES = [["phTrans",["prepare","vad","diarize","transcribe","name","title"]],["phTransl",["translate"]]];
let prog = null; // last progress event and when it arrived, so the clock keeps ticking between events
function showProgress(d){ prog = {d, at: performance.now()}; renderProgress(); }
setInterval(()=>{ if(prog?.d.status==="running" && $("#proc").classList.contains("on")) renderProgress(); }, 1000);
function stepInfo(d){
  const p = d.progress || {}, est = p.est || {}, took = p.took || {};
  const live = (p.stepEl||0) + (d.status==="running" ? (performance.now()-prog.at)/1000 : 0);
  const steps = STEP_ORDER.filter(s=>(s!=="translate" || d.translateTo) && (s!=="title" || est.title>0 || p.step==="title")), cur = steps.indexOf(p.step);
  return steps.map((s,i)=>{
    if(d.status==="queued" || i>cur) return {s, st:"wait", el:0, left:est[s]||0};
    if(i<cur) return {s, st:"ok", el:took[s]||0, left:0};
    // transcribe and translate report real progress (files, lines), so extrapolate from it
    const byFrac = (s==="transcribe"||s==="translate") && p.frac>0.03 ? live/p.frac-live : null;
    return {s, st:HALT[d.status]?"stop":"run", el:live, left:Math.max(0, byFrac ?? (est[s]||0)-live), detail:p.detail};
  });
}
function renderProgress(){
  const d = prog.d, info = stepInfo(d);
  $("#ptitle").textContent = d.title;
  $("#pline").textContent = [dateFmt(new Date(d.recorded),{dateStyle:"long"}), d.duration?fmt(d.duration):""].filter(Boolean).join(", ");
  $("#phases").innerHTML = PHASES.filter(([,ss])=>info.some(x=>ss.includes(x.s))).map(([k,ss])=>{
    const g = info.filter(x=>ss.includes(x.s)), el = g.reduce((a,x)=>a+x.el,0), left = g.reduce((a,x)=>a+x.left,0);
    const state = g.every(x=>x.st==="ok") ? "done" : g.some(x=>x.st==="stop") ? "stop" : g.some(x=>x.st!=="wait") ? "run" : "wait";
    const right = state==="done" ? T("tookIn",dur(el)) : state==="stop" ? T(HALT[d.status]) : state==="run" ? `${T("elapsed",fmt(el))}<i></i>${left>20?T("leftAbout",mins(left)):T("almost")}` : (left ? T("expected",mins(left)) : T("waiting"));
    const pct = state==="done" ? 100 : state==="run"||state==="stop" ? Math.round(100*el/(el+left||1)) : 0;
    return `<div class="phase ${state}"><div class="phead"><b>${T(k)}</b><span class="num">${right}</span></div><div class="track"><i style="width:${pct}%"></i></div></div>`;
  }).join("") + (d.status==="queued" ? `<p class="aside">${T("queuedNote")}</p>` : "");
  $("#steps").innerHTML = info.map(x=>`<div class="step ${x.st}"><span class="ic">${x.st==="ok"?ICON.check:x.st==="stop"?ICON.pause:""}</span>${T("steps")[x.s]}<em class="num">${
      x.st==="ok" ? dur(x.el) : x.st==="stop" ? (x.detail?`<span class="dt">${x.detail}</span>`:"") : x.st==="run" ? (x.detail?`<span class="dt">${x.detail}</span>`:"")+fmt(x.el) : x.left>1 ? T("expected",mins(x.left)) : ""}</em></div>`).join("")
    + (HALT[d.status] ? `<p class="aside">${d.status==="error" ? esc(d.error) : T("stoppedNote")}</p><div class="pacts"><button class="btn" data-again="resume">${T("resume")}</button><button class="quiet" data-again="fresh">${T("startOver")}</button><button class="quiet" data-again="del">${T("del")}</button></div>` : "");
  $("#proc [data-t=closeNote]").parentElement.hidden = !!HALT[d.status];
}
// Resume reuses what the stopped run left in work/, Start over throws it away. Old transcripts never stored
// their request, so those go through the setup screen again (prefilled) and Transcribe resumes this id
$("#steps").addEventListener("click", async e=>{ const b=e.target.closest("[data-again]"); if(!b) return; const d=prog.d, fresh=b.dataset.again==="fresh";
  if(b.dataset.again==="del") return remove(d.id);
  try { await invoke("resume_job",{id:d.id, fresh}); open(d.id); }
  catch(err){ if(err!=="needs-setup") return toast(String(err));
    startDraft({path:d.source?.path, title:d.title, kind:d.source?.kind, recorded:d.recorded, duration:d.duration});
    Object.assign(draft, {vocab:d.vocab||draft.vocab, translateTo:d.translateTo||null, trPicked:true, resumeId:d.id, fresh}); renderSetup(); } });
listen("job-progress", async e=>{
  const d = e.payload; loadList();
  if(active!==d.id) return;
  if(d.status==="done") return open(d.id);
  showProgress(d);
});

/* ---------- transcript view ---------- */
let hidden = new Set(), q = "", cur = 0, t = 0, stopAt = null, lineI = null, editing = null;
const AU = $("#au");
const nameOf = i => doc.speakers[i]?.name || `${T("speakerN")} ${i+1}`;
function renderView(){
  $("#vtitle").textContent = doc.title;
  $("#vdate").textContent = `${T("recordedOn")} ${dateFmt(new Date(doc.recorded),{dateStyle:"medium",timeStyle:"short"})}`;
  $("#vadded").textContent = doc.added ? `${T("addedOn")} ${dateFmt(new Date(doc.added),{dateStyle:"medium"})}` : "";
  $("#vdur").textContent = fmt(doc.duration); $("#ttot").textContent = fmt(doc.duration);
  $("#vlangs").innerHTML = (doc.languages||[]).map(l=>`<span class="glyph">${GLYPH[l]||l}</span>`).join("");
  $("#vtr").textContent = doc.translateTo && doc.turns.some(x=>x.tr) ? T("trNote",langName(doc.translateTo)) : "";
  const tm = doc.timing || {}, tTrans = ["prepare","vad","diarize","transcribe","name"].reduce((a,k)=>a+(tm[k]||0),0);
  $("#vtime").textContent = [tTrans ? `${T("phTrans")} ${dur(tTrans)}` : "", tm.translate ? `${T("phTransl")} ${dur(tm.translate)}` : ""].filter(Boolean).join(", ");
  const src = convertFileSrc(doc.audioPath); if(AU.dataset.src!==src){ AU.src=src; AU.dataset.src=src; t=0; cur=0; }
  tools(); turns(); map(); place(); found();
}
$("#vfound").addEventListener("keydown",e=>{ const i=e.target.closest(".termedit"); if(!i) return;
  if(e.key==="Escape") found();
  if(e.key==="Enter" && i.value.trim()) fixTerm(i.dataset.old, i.value.trim()); });
$("#vfound").addEventListener("focusout",e=>{ if(e.target.closest(".termedit")) setTimeout(()=>{ if(!document.activeElement?.closest?.("#vfound")) found(); }); });
/* names and jargon the local model found in this meeting, one click to add to the vocabulary */
const askedTerms = new Set();
let foundOpen = false;
function found(){
  const have = new Set(memory.vocab.map(w=>w.toLowerCase())), fresh = (doc.terms||[]).filter(w=>!have.has(w.toLowerCase()));
  $("#vfound").innerHTML = !fresh.length ? "" : `<button class="fold ${foundOpen?"open":""}" data-fold>${T("foundTerms")} <span class="num">${fresh.length}</span><svg viewBox="0 0 16 16"><path d="M6 4l4 4-4 4"/></svg></button>` + (!foundOpen ? "" : fresh.map(w=>`<span class="fterm"><button class="term" data-v="${esc(w)}">${esc(w)}</button><button class="fix" data-fix="${esc(w)}" title="${T("fixTerm")}" aria-label="${T("fixTerm")}"><svg viewBox="0 0 16 16"><path d="M3 13l1-3.5L10.5 3 13 5.5 6.5 12z"/></svg></button></span>`).join(""));
  if(doc.terms || doc.status!=="done" || askedTerms.has(doc.id)) return;
  const id = doc.id; askedTerms.add(id);
  invoke("suggest_terms",{id}).then(t=>{ if(doc.id===id){ doc.terms=t; found(); } }).catch(()=>{});
}
// a misheard term ("Face 4"): fix it everywhere in this transcript and keep the right spelling
function fixTerm(old, neu){
  const re = new RegExp(`(?<![A-Za-z0-9])${old.replace(/[.*+?^${}()|[\]\\]/g,"\\$&")}(?![A-Za-z0-9])`,"gi");
  let n = 0;
  doc.turns.forEach(x=>["x","tr"].forEach(k=>{ if(x[k]) x[k]=x[k].replace(re,()=>{ n++; return neu; }); }));
  doc.terms = doc.terms.map(w=>w===old ? neu : w);
  if(!memory.vocab.some(w=>w.toLowerCase()===neu.toLowerCase())) memory.vocab.push(neu);
  localStorage.memory = JSON.stringify(memory); save(); turns(); found(); toast(T("termFixed",n));
}
$("#vfound").addEventListener("click",e=>{ if(e.target.closest("[data-fold]")){ foundOpen=!foundOpen; found(); return; }
  const f=e.target.closest("[data-fix]"); if(f){ const w=f.dataset.fix;
    f.parentElement.outerHTML = `<input class="termedit" value="${esc(w)}" data-old="${esc(w)}">`; const i=$(".termedit"); i.focus(); i.select(); return; }
  const b=e.target.closest("[data-v]"); if(b){ memory.vocab.push(b.dataset.v); localStorage.memory=JSON.stringify(memory); found(); toast(T("termAdded")); } });
function share(){ const s=[]; doc.diar.forEach(([a,b,k])=>s[k]=(s[k]||0)+b-a); const tot=s.reduce((x,y)=>x+(y||0),0)||1; return s.map(v=>Math.round(100*(v||0)/tot)); }
function nSpk(){ return Math.max(doc.speakers.length, ...doc.diar.map(d=>d[2]+1)); }
let minorOpen = false;
function tools(){
  const sh = share(), all = Array.from({length:nSpk()},(_,i)=>i);
  // ponytail: under 2% is mostly diarization splitting one voice; fold them unless there is just one
  let minor = all.filter(i=>(sh[i]||0)<2 && !doc.speakers[i]?.name && editing!==`spk${i}`); if(minor.length<2) minor = [];
  const shown = minorOpen ? all : all.filter(i=>!minor.includes(i));
  $("#tools").innerHTML = shown.map(i=> editing===`spk${i}`
      ? `<input class="spkedit" data-spkin="${i}" value="${esc(nameOf(i))}">`
      : `<button class="spk ${hidden.has(i)?"off":""}" data-spk="${i}" title="Double-click to rename"><span class="dot" style="background:var(--sp${i%6})"></span>${esc(nameOf(i))}<small class="num">${sh[i]||0}%</small></button>`).join("")
    + (minor.length ? `<button class="spk more" data-minor>${minorOpen ? T("less") : `+${minor.length}`}</button>` : "")
    ;
  const inp = $(".spkedit"); if(inp){ inp.focus(); inp.select(); }
}
$("#q").oninput=e=>{ q=e.target.value; turns(); };
$("#tools").addEventListener("click",e=>{ if(e.target.closest("[data-minor]")){ minorOpen=!minorOpen; tools(); return; }
  const s=e.target.closest("[data-spk]"); if(!s) return; const i=+s.dataset.spk; hidden.has(i)?hidden.delete(i):hidden.add(i); tools(); turns(); });
$("#tools").addEventListener("dblclick",e=>{ const s=e.target.closest("[data-spk]"); if(!s) return; hidden.delete(+s.dataset.spk); editing=`spk${s.dataset.spk}`; tools(); });
$("#tools").addEventListener("keydown",e=>{ const i=e.target.dataset.spkin; if(i===undefined) return; if(e.key==="Enter") e.target.blur(); if(e.key==="Escape"){ editing=null; tools(); } });
$("#tools").addEventListener("focusout",e=>{ const i=e.target.dataset.spkin; if(i===undefined||editing===null) return; editing=null;
  while(doc.speakers.length<=+i) doc.speakers.push({name:"",langs:[]});
  doc.speakers[+i].name=e.target.value.trim(); save(); tools(); turns(); map(); });
function hl(s){ return q ? esc(s).split(esc(q)).join(`<mark>${esc(q)}</mark>`) : esc(s); }
function turns(){
  $("#turns").innerHTML = doc.turns.map((x,i)=>{
    if(hidden.has(x.s) || (q && !x.x.includes(q) && !(x.tr||"").includes(q))) return "";
    return `<div class="turn ${i===cur?"now":""} ${x.tr?"has-tr":""}" data-i="${i}">
      <button class="ts num" data-seek="${i}">${fmt(x.t)}</button>
      <div><div class="who"><span class="dot" style="background:var(--sp${x.s%6})"></span><b>${esc(nameOf(x.s))}</b><span class="glyph">${GLYPH[x.l]||x.l}</span>
        <button class="clip" data-clip="${i}" aria-label="Play this line">${ICON.play.replace("<svg",'<svg class="pl"')}${ICON.pause.replace("<svg",'<svg class="pz"')}<span class="bars">${(x.w||[]).map(v=>`<i style="height:${3+v*1.6}px"></i>`).join("")}</span><span class="cd num">${dfmt(x.e-x.t)}</span></button>
        <span class="acts"><button data-chg="${i}">${T("change")}</button><button data-edit="${i}">${T("edit")}</button></span></div>
        <p class="orig" lang="${x.l}" data-orig="${i}">${hl(x.x)}</p>${x.tr?`<p class="tr" lang="${doc.translateTo||""}">${hl(x.tr)}</p>`:""}</div></div>`;
  }).join("");
}
function map(){
  const n = nSpk(), D = doc.duration;
  $("#map").innerHTML = Array.from({length:n},(_,k)=>`<span class="nm" style="grid-row:${k+1}">${esc(nameOf(k))}</span><div class="lane" style="grid-row:${k+1}">${doc.diar.filter(d=>d[2]===k).map(([a,b])=>`<i style="left:${a/D*100}%;width:${Math.max(.12,(b-a)/D*100)}%;background:var(--sp${k%6})"></i>`).join("")}</div>`).join("")
    + `<div id="lanes" style="grid-column:2;grid-row:1 / ${n+1};position:relative;align-self:stretch"><div class="head" id="head"></div><div class="hover num" id="hov"></div></div>`;
  place();
}
function place(){ const h=$("#head"); if(h&&doc) h.style.left=`${t/doc.duration*100}%`; $("#tnow").textContent=fmt(t); }
function seekTime(s, scroll=true){
  t=Math.max(0,Math.min(doc.duration,s)); let i=0; doc.turns.forEach((x,k)=>{ if(x.t<=t+.05) i=k; });
  const changed=i!==cur; cur=i; place();
  document.querySelectorAll(".turn").forEach(el=>el.classList.toggle("now",+el.dataset.i===cur));
  if(changed && scroll) document.querySelector(`.turn[data-i="${cur}"]`)?.scrollIntoView({block:"center",behavior:matchMedia("(prefers-reduced-motion:reduce)").matches?"auto":"smooth"});
}
function setPlay(on, from){
  if(!doc) return;
  if(on){ AU.currentTime = from ?? t; AU.play(); requestAnimationFrame(tick); }
  else { AU.pause(); stopAt=null; lineI=null; paintClip(); }
  $("#play").innerHTML = on?ICON.pause:ICON.play;
}
function tick(){
  if(AU.paused) return;
  const now = AU.currentTime;
  if(stopAt!==null && now>=stopAt){ const end=stopAt; setPlay(false); seekTime(end-.05,false); return; }
  seekTime(now); paintClip(now); requestAnimationFrame(tick);
}
function paintClip(now){
  document.querySelectorAll(".turn").forEach(el=>{
    const i=+el.dataset.i, on=i===lineI, x=doc.turns[i], bars=el.querySelectorAll(".bars i"), p=on?(now-x.t)/(x.e-x.t):-1;
    el.classList.toggle("playing",on); bars.forEach((b,k)=>b.classList.toggle("on",on && k/bars.length<p));
  });
}
function jump(s){ seekTime(s); if(!AU.paused){ stopAt=null; lineI=null; paintClip(); AU.currentTime=t; } }
AU.addEventListener("ended",()=>setPlay(false));
$("#play").onclick=()=>{ stopAt=null; lineI=null; setPlay(AU.paused); };
$("#rw").onclick=()=>jump(t-5); $("#ff").onclick=()=>jump(t+5);
const RATES=[1,1.25,1.5,2,.75]; $("#rate").onclick=e=>{ const r=RATES[(RATES.indexOf(AU.playbackRate)+1)%RATES.length]; AU.playbackRate=r; e.currentTarget.textContent=`${r}×`; };
$("#turns").addEventListener("click",e=>{
  const c=e.target.closest("[data-clip]");
  if(c){ const i=+c.dataset.clip; if(lineI===i && !AU.paused){ setPlay(false); return; } AU.pause(); lineI=i; stopAt=doc.turns[i].e; seekTime(doc.turns[i].t,false); setPlay(true,doc.turns[i].t); return; }
  const s=e.target.closest("[data-seek]"); if(s) return jump(doc.turns[+s.dataset.seek].t);
  const ch=e.target.closest("[data-chg]"); if(ch){ const x=doc.turns[+ch.dataset.chg]; x.s=(x.s+1)%nSpk(); save(); turns(); return; }
  const ed=e.target.closest("[data-edit]"); if(ed){ const p=document.querySelector(`[data-orig="${ed.dataset.edit}"]`); p.contentEditable="true"; p.textContent=doc.turns[+ed.dataset.edit].x; p.focus(); }
});
$("#turns").addEventListener("focusout",e=>{ const i=e.target.dataset.orig; if(i===undefined||e.target.contentEditable!=="true") return; e.target.contentEditable="false"; doc.turns[+i].x=e.target.textContent.trim(); save(); });
$("#map").addEventListener("click",e=>{ const r=$("#lanes").getBoundingClientRect(); jump((e.clientX-r.left)/r.width*doc.duration); });
$("#map").addEventListener("mousemove",e=>{ const r=$("#lanes")?.getBoundingClientRect(); if(!r) return; const x=Math.max(0,Math.min(r.width,e.clientX-r.left)); $("#hov").style.left=x+"px"; $("#hov").textContent=fmt(x/r.width*doc.duration); });
document.addEventListener("keydown",e=>{
  if(!$("#view").classList.contains("on") || e.target.matches("input,select,[contenteditable=true]")) return;
  if(e.code==="Space"){ e.preventDefault(); stopAt=null; lineI=null; setPlay(AU.paused); }
  if(e.key==="ArrowLeft") jump(t-5); if(e.key==="ArrowRight") jump(t+5);
  if(e.key==="ArrowDown"&&cur<doc.turns.length-1){ e.preventDefault(); jump(doc.turns[cur+1].t); }
  if(e.key==="ArrowUp"&&cur>0){ e.preventDefault(); jump(doc.turns[cur-1].t); }
  if((e.metaKey||e.ctrlKey) && e.key==="f"){ e.preventDefault(); $("#q").focus(); }
  if((e.metaKey||e.ctrlKey) && e.key==="Backspace"){ e.preventDefault(); remove(doc.id); }
});
let mode = "both";
$("#mode").addEventListener("click",e=>{ const b=e.target.closest("button"); if(!b) return; mode=b.dataset.v; document.body.dataset.mode=mode; $("#mode").querySelectorAll("button").forEach(x=>x.classList.toggle("on",x===b)); });
function save(){ invoke("save_transcript",{doc}).catch(err=>toast(String(err))); }
$("#copy").onclick=async()=>{
  const txt = doc.turns.map(x=>{ const tr = x.tr && mode!=="orig" ? (mode==="tr" ? null : x.tr) : null;
    const body = mode==="tr" && x.tr ? x.tr : x.x; return `[${fmt(x.t)}] ${nameOf(x.s)} (${x.l}): ${body}${tr?`\n  > ${tr}`:""}`; }).join("\n\n");
  await navigator.clipboard.writeText(`# ${doc.title}\n\n${txt}\n`); toast(T("copied"));
};
$("#del").onclick=()=>remove(doc.id);
async function exportMd(id){ try { toast(T("exported", await invoke("export_markdown",{id, mode}))); } catch(err){ toast(String(err)); } }
$("#export").onclick=()=>exportMd(doc.id);

/* ---------- toggles + boot ---------- */
function mini(id,val,fn){ const el=$(id); el.querySelectorAll("button").forEach(b=>b.classList.toggle("on",b.dataset.v===val)); el.onclick=e=>{ const b=e.target.closest("button"); if(b){ fn(b.dataset.v); mini(id,b.dataset.v,fn); } }; }
const theme = localStorage.theme || (matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light");
document.documentElement.dataset.theme = theme;
$("#uiLang").innerHTML = UI_LANGS.map(([c,n])=>`<option value="${c}">${n}</option>`).join(""); $("#uiLang").value = L;
$("#uiLang").onchange = e=>{ L=localStorage.lang=e.target.value; texts(); };
mini("#theme",theme,v=>{ document.documentElement.dataset.theme=localStorage.theme=v; });
$("#play").innerHTML = ICON.play;
window.addEventListener("focus", loadList);
texts();
(async()=>{ await loadList(); const first = list.find(d=>d.status==="done"); first ? open(first.id) : go("new"); })();

/* ---------- title: edit by hand, or let the local model suggest one ---------- */
$("#vtitle").addEventListener("keydown",e=>{ if(e.key==="Enter"){ e.preventDefault(); e.target.blur(); } });
$("#vtitle").addEventListener("blur",e=>{ const v=e.target.textContent.trim(); if(v && v!==doc.title){ doc.title=v; save(); loadList(); } else e.target.textContent=doc.title; });
$("#retitle").onclick = async e=>{
  const b=e.currentTarget, id=doc.id; b.disabled=true; b.querySelector("span").textContent=T("titling");
  try { const t = await invoke("retitle",{id, lang:L}); if(doc.id===id){ doc.title=t; $("#vtitle").textContent=t; } loadList(); }
  catch(err){ toast(String(err)); }
  finally { b.disabled=false; b.querySelector("span").textContent=T("autoTitle"); }
};

/* ---------- vocabulary + settings (function declarations: texts() runs before this point) ---------- */
function me(){ return JSON.parse(localStorage.me || '{"name":"","langs":[]}'); }
function renderPrefs(){
  $("#vlist").innerHTML = memory.vocab.map((w,i)=>`<button class="term" data-w="${i}" title="Remove">${esc(w)}</button>`).join("") + `<input placeholder="${T("addWord")}">`;
  const m = me(); $("#myname").value = m.name;
  $("#mylangs").innerHTML = Object.entries(GLYPH).map(([k,g])=>`<button class="lg ${m.langs.includes(k)?"on":""}" data-l="${k}" title="${langName(k)}">${g}</button>`).join("");
}
const saveVocab = () => { localStorage.memory = JSON.stringify(memory); renderPrefs(); $("#vlist input").focus(); };
$("#vlist").addEventListener("click",e=>{ const w=e.target.closest("[data-w]"); if(w){ memory.vocab.splice(+w.dataset.w,1); saveVocab(); } });
$("#vlist").addEventListener("keydown",e=>{ const v=e.target.value?.trim(); if(e.key==="Enter" && v){ if(!memory.vocab.includes(v)) memory.vocab.push(v); saveVocab(); } });
const saveMe = m => { localStorage.me = JSON.stringify(m); };
$("#myname").addEventListener("input",e=>saveMe({...me(), name:e.target.value.trim()}));
$("#mylangs").addEventListener("click",e=>{ const b=e.target.closest(".lg"); if(!b) return; const m=me(), l=b.dataset.l;
  m.langs = m.langs.includes(l) ? m.langs.filter(x=>x!==l) : [...m.langs,l]; saveMe(m); b.classList.toggle("on"); });

/* ---------- record a meeting here; stopping it starts the transcript ---------- */
let recDraft = null, recQuiet = false; // the draft being recorded, kept while the user looks elsewhere
// local time for the file name ("2026-11-26 15-04-05"); Rust has no local clock
const stamp = d => new Date(d-d.getTimezoneOffset()*6e4).toISOString().slice(0,19).replace("T"," ").replaceAll(":","-");
$("#record").onclick = async ()=>{
  const now = new Date();
  try { const path = await invoke("start_recording",{stamp:stamp(now)});
    startDraft({path, kind:"recording", title:T("recTitle",dateFmt(now,{dateStyle:"medium",timeStyle:"short"})), recorded:now.toISOString(), duration:0});
    recDraft = Object.assign(draft,{recording:true, recAt:Date.now()}); recQuiet = false; renderSetup(); }
  catch(err){ toast(String(err)); }
};
function recEnd(){ if(recDraft) recDraft.recording = false; recDraft = null; recPaint(); }
async function recStop(){ const d = recDraft; recEnd(); await invoke("stop_recording").catch(()=>{});
  d.source.duration = (await invoke("probe_source",{path:d.source.path})).duration || 0; }
function recPaint(){
  const on = !!draft?.recording, el = recDraft ? fmt((Date.now()-recDraft.recAt)/1000) : "";
  $("#setup .preview").style.display = on ? "none" : "";
  $("#recbar").classList.toggle("on",on); $("#rectime").textContent = el;
  $("#reclvl").textContent = T(recQuiet?"recQuiet":"recSound"); $("#reclvl").classList.toggle("q",recQuiet);
  const b = $("#start"); if(on) b.textContent = T("recStop"); else if(b.textContent===T("recStop")) b.textContent = T("transcribe");
  // away from the recording's setup screen, a pill in the sidebar leads back to it
  $("#recpill").classList.toggle("on", !!recDraft && !(draft===recDraft && $("#setup").classList.contains("on"))); $("#recpt").textContent = el;
}
setInterval(()=>{ if(recDraft) recPaint(); }, 1000);
$("#recpill").onclick = ()=>{ draft = recDraft; renderSetup(); go("setup"); };
// Cancel while recording stops it and keeps the file without transcribing (Back keeps recording)
$("#setup .bottom [data-go=new]").addEventListener("click", async ()=>{ if(!draft?.recording) return; const p = draft.source.path; recEnd();
  await invoke("stop_recording").catch(()=>{}); toast(T("recSaved",p)); });
listen("rec-state", e=>{ recQuiet = e.payload.quiet; if(recDraft) recPaint(); });
// stopped by 5 quiet minutes or by sleep: transcribe what was recorded
listen("rec-stopped", async e=>{ const {path, reason, duration} = e.payload, d = recDraft;
  if(!d || d.source.path!==path || reason==="user") return;
  recEnd(); d.source.duration = duration || 0;
  if(reason==="error"){ toast(T("recFailed")); if(draft===d) renderSetup(); return; }
  toast(T(reason==="sleep"?"recSlept":"recSilence")); draft = d; await $("#start").onclick(); });
