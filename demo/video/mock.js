// Stands in for the Tauri backend so app/ui runs in a plain browser with the demo meeting.
// Loaded before app.js by record.mjs (Playwright addInitScript); only acts inside the app frame.
(() => {
  if (!location.pathname.includes("/app/ui/")) return;
  const DOC_URL = "/demo/video/transcript.json";
  const AUDIO = "/demo/nimbus-launch-sync.m4a";
  localStorage.lang = "en";
  localStorage.theme = "light";
  localStorage.me = JSON.stringify({ name: "Sam", langs: ["en"] });
  localStorage.memory = JSON.stringify({ vocab: ["Nimbus"], speakers: [
    { name: "Mina", langs: ["ko", "zh"] }, { name: "Wei", langs: ["zh", "en"] },
    { name: "Yuki", langs: ["ja", "en"] }, { name: "Claire", langs: ["fr", "en"] }] });

  const on = {};
  let doc = null, state = window.__showDemo ? "done" : "idle", live = null;
  const past = [
    { id: "p1", title: "Weekly design review", duration: 1834, languages: ["en", "ja"], recorded: "2026-11-21T15:00:00+09:00", added: "2026-11-21T16:00:00+09:00", status: "done" },
    { id: "p2", title: "Shanghai partner call", duration: 2711, languages: ["zh", "ko", "en"], recorded: "2026-11-18T11:00:00+09:00", added: "2026-11-18T12:00:00+09:00", status: "done" },
  ];
  const load = async () => doc ??= await (await fetch(DOC_URL)).json();
  const summary = d => ({ id: d.id, title: d.title, duration: d.duration, languages: d.languages, recorded: d.recorded, added: d.added, status: d.status, progress: d.progress });
  const emit = (name, payload) => (on[name] || []).forEach(f => f({ payload }));

  // the progress screen, compressed: each step reports what a real run of this file took
  const TOOK = { prepare: 0.6, vad: 1.4, diarize: 18.2, transcribe: 41.5, name: 0.1, translate: 24.8 };
  async function fakeJob(d) {
    const steps = ["prepare", "vad", "diarize", "transcribe", "name", "translate"], took = {};
    for (const s of steps) {
      for (const frac of [0.2, 0.6, 1]) {
        live = { ...d, status: "running", progress: { step: s, stepEl: TOOK[s] * frac, est: TOOK, took: { ...took }, frac, pct: (steps.indexOf(s) + frac) / steps.length } };
        emit("job-progress", live);
        await new Promise(r => setTimeout(r, window.__stepMs ?? 350));
      }
      took[s] = TOOK[s];
    }
    state = "done";
    emit("job-progress", { ...d, status: "done" });
  }

  const cmds = {
    list_transcripts: async () => { const d = await load(); return [...(state === "done" ? [summary(d)] : state === "running" && live ? [live] : []), ...past]; },
    get_transcript: async ({ id }) => { const d = await load(); return id === d.id ? (state === "done" ? d : live || { ...summary(d), translateTo: "en", status: "running", progress: { step: "prepare", stepEl: 0, est: TOOK, took: {} } }) : { ...d, ...past.find(p => p.id === id), turns: d.turns.slice(0, 4) }; },
    probe_source: async () => ({ recorded: "2026-11-24T10:00:00+09:00", duration: 121.6 }),
    estimate: async () => ({ transcribe: 62, translate: 25 }),
    start_job: async () => { const d = await load(); state = "running"; setTimeout(() => fakeJob({ ...summary(d), translateTo: "en", status: "running" }), 50); return d.id; },
    list_voice_memos: async () => [],
    suggest_terms: async () => [],
    save_transcript: async () => null,
    retitle: async () => doc?.title,
  };
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args = {}) => (cmds[cmd] ? cmds[cmd](args) : null),
      convertFileSrc: () => AUDIO,
    },
    event: { listen: async (name, f) => { (on[name] ||= []).push(f); return () => {}; } },
    menu: { Menu: { new: async () => ({ popup() {} }) } },
  };
  window.__demo = { emit, load };
})();
