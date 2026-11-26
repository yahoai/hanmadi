// Records the promo video: app/ui in Chrome with mock.js as the backend, scripted clicks, captions.
// usage: node demo/video/record.mjs  ->  demo/video/out/demo.mp4 (1920x1080, 30 fps, no audio)
// needs Google Chrome and playwright-core (npm i playwright-core in ~/.cache/hanmadi-video), plus ffmpeg
import { createRequire } from "node:module";
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(join(process.env.HOME, ".cache/hanmadi-video/"));
const { chromium } = require("playwright-core");
const HERE = dirname(fileURLToPath(import.meta.url)), ROOT = join(HERE, "../.."), OUT = join(HERE, "out"), FRAMES = join(OUT, "frames");
const PORT = 8771, wait = ms => new Promise(r => setTimeout(r, ms));

rmSync(FRAMES, { recursive: true, force: true }); mkdirSync(FRAMES, { recursive: true });
const server = spawn("python3", ["-m", "http.server", String(PORT), "--bind", "127.0.0.1"], { cwd: ROOT, stdio: "ignore" });
await wait(700);

let browser;
const cleanup = () => { browser?.close().catch(() => {}); server.kill(); };
const fail = async e => { console.error(String(e).split("\n")[0]); await globalThis.page?.screenshot({ path: join(OUT, "error.png") }).catch(() => {}); cleanup(); process.exit(1); };
process.on("uncaughtException", fail); process.on("unhandledRejection", fail);
browser = await chromium.launch({ channel: "chrome", args: ["--autoplay-policy=no-user-gesture-required", "--hide-scrollbars"] });
const ctx = await browser.newContext({ viewport: { width: 1440, height: 810 }, deviceScaleFactor: 4 / 3 });
await ctx.addInitScript({ path: join(HERE, "mock.js") });
const page = globalThis.page = await ctx.newPage();
await page.goto(`http://127.0.0.1:${PORT}/demo/video/stage.html`);
await page.waitForFunction(() => document.querySelector("#app").contentDocument?.querySelector("#list .item"));
const app = page.frame({ url: /\/app\/ui\// });
await app.evaluate(() => { document.querySelector('[data-go="new"]').click(); return document.fonts.ready; });
await page.evaluate(() => stage.card("Hanmadi", "Meeting transcripts for teams that switch languages"));
await wait(400);

// capture every painted frame with its timestamp, then hold each one until the next
const shots = [], cdp = await ctx.newCDPSession(page);
cdp.on("Page.screencastFrame", async f => {
  const file = join(FRAMES, `${String(shots.length).padStart(5, "0")}.jpg`);
  writeFileSync(file, Buffer.from(f.data, "base64")); shots.push({ file, t: f.metadata.timestamp });
  await cdp.send("Page.screencastFrameAck", { sessionId: f.sessionId }).catch(() => {});
});
await cdp.send("Page.startScreencast", { format: "jpeg", quality: 92, maxWidth: 1920, maxHeight: 1080 });

const S = (fn, ...a) => page.evaluate(([fn, a]) => stage[fn](...a), [fn, a]);
async function point(sel, { click = true, dy = 0 } = {}) {
  const el = app.locator(sel).first(), b = await el.boundingBox();
  await S("cursor", b.x + b.width / 2, b.y + b.height / 2 + dy); await wait(600);
  if (click) { await S("press", true); await wait(90); await el.click(); await S("press", false); }
}

await wait(2600);
await S("card", null); await wait(700);

// 1. drop a recording
await S("caption", "Drop in a meeting recording");
const drop = await app.locator("#drop").boundingBox();
await S("file", 1500, 760, true); await S("cursor", 1560, 800); await wait(300);
await S("file", drop.x + drop.width / 2 - 110, drop.y + drop.height / 2 - 24, true);
await S("cursor", drop.x + drop.width / 2, drop.y + drop.height / 2); await wait(1000);
await app.evaluate(() => __demo.emit("tauri://drag-enter", {})); await wait(500);
await S("file", drop.x + drop.width / 2 - 110, drop.y + drop.height / 2 - 24, false);
await app.evaluate(() => __demo.emit("tauri://drag-drop", { paths: ["/demo/Nimbus launch sync.m4a"] }));
await wait(1200);

// 2. who is in the meeting
await S("caption", "Say who was there and which languages they speak");
for (let n = 0; n < 4; n++) { await point("#past [data-p]"); await wait(350); }
await wait(900);

// 3. transcribe, all on this machine
await S("caption", "It all runs on your Mac. Nothing is uploaded");
await point("#start"); await page.evaluate(() => 0);
await app.waitForSelector("#view.on", { timeout: 20000 });
await wait(900);

// 4. the transcript
await S("zoom", 1.32, 860, 330); await wait(1200);
await S("caption", "Every line keeps the language it was spoken in");
await S("cursor", 1500, 900);
for (let i = 0; i < 40; i++) { await app.evaluate(() => document.querySelector("#scroller").scrollBy(0, 9)); await wait(45); }
await wait(1500);
await S("caption", "With a translation underneath");
await point('#mode [data-v="tr"]'); await wait(1600);
await point('#mode [data-v="orig"]'); await wait(1400);
await point('#mode [data-v="both"]'); await wait(1200);

// 5. play a line
await S("caption", "Click any line to hear it");
await app.evaluate(() => document.querySelector("#scroller").scrollTo({ top: 0, behavior: "smooth" })); await wait(700);
await point('[data-clip="1"]'); await wait(4200);
await S("caption", null); await S("zoom", 1); await wait(1200);

// 6. end card
await S("card", "Hanmadi", "Free and open source. Runs on your Mac.", "github.com/yahoai/hanmadi");
await wait(3200);

await cdp.send("Page.stopScreencast");
await browser.close(); server.kill();

// hold each frame until the next one arrived, then encode at a constant 30 fps
const list = shots.map((s, i) => `file '${s.file}'\nduration ${Math.max(1 / 60, (shots[i + 1]?.t ?? s.t + 0.5) - s.t).toFixed(4)}`).join("\n") + `\nfile '${shots.at(-1).file}'\n`;
writeFileSync(join(OUT, "frames.txt"), list);
execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", join(OUT, "frames.txt"),
  "-vf", "scale=1920:1080:flags=lanczos,fps=30,format=yuv420p", "-c:v", "libx264", "-crf", "18", "-preset", "slow", "-movflags", "+faststart", join(OUT, "demo.mp4")]);
console.log(join(OUT, "demo.mp4"), shots.length, "frames");
