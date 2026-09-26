// CPU-profile the GPUI wasm startup (navigation -> first frame) in a fresh Chrome profile.
// Prints the top self-time functions and the time spent inside WebGL/WebGPU API calls.
import { chromium } from 'playwright-core';
import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path';
const url = process.argv[2] || 'http://192.168.0.101:8889/gpui/?bench=1&backend=webgl';
const out = process.argv[3] || 'startprof.cpuprofile';
const extra = (process.env.CHROME_ARGS || '').split(' ').filter(Boolean);
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'anorak-prof-'));
const ctx = await chromium.launchPersistentContext(profile, { executablePath: 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  headless: false, viewport: { width: 1200, height: 600 }, args: ['--no-first-run', '--no-default-browser-check', '--disable-extensions', '--window-size=1216,720', '--window-position=0,0', ...extra] });
const page = ctx.pages()[0] || await ctx.newPage();
const cdp = await ctx.newCDPSession(page);
await page.goto('about:blank'); await page.waitForTimeout(1000);
const runs = +(process.env.RUNS || 1);
for (let r = 0; r < runs; r++) {
  if (r > 0) { await page.goto('about:blank'); await page.waitForTimeout(800); }
  await cdp.send('Profiler.enable'); await cdp.send('Profiler.setSamplingInterval', { interval: 200 });
  await cdp.send('Profiler.start');
  await page.goto(url, { waitUntil: 'commit' });
  let ff = null; const t0 = Date.now();
  while (!ff && Date.now() - t0 < 30000) { ff = await page.evaluate(() => (window.__anorakBench || []).map(s => JSON.parse(s)).find(e => e.event === 'first_frame')).catch(() => null); if (!ff) await page.waitForTimeout(20); }
  const { profile: prof } = await cdp.send('Profiler.stop');
  fs.writeFileSync(out.replace('.cpuprofile', `_${r}.cpuprofile`), JSON.stringify(prof));
  const dt = (prof.timeDeltas || []); const self = new Map(); const byId = new Map(prof.nodes.map(n => [n.id, n]));
  prof.samples.forEach((id, i) => { const n = byId.get(id); const k = `${n.callFrame.functionName || '(anon)'} ${n.callFrame.url ? n.callFrame.url.split('/').pop() : ''}`; self.set(k, (self.get(k) || 0) + (dt[i] || 0) / 1000); });
  const top = [...self.entries()].sort((a, b) => b[1] - a[1]).slice(0, 25);
  console.log(`run ${r}: first_frame ${ff && ff.perf_now_ms} ms, wasm start->frame ${ff && ff.elapsed_ms}; total sampled ${(dt.reduce((a, b) => a + b, 0) / 1000).toFixed(0)} ms`);
  for (const [k, v] of top) console.log(`  ${v.toFixed(1).padStart(8)} ms  ${k}`);
}
await ctx.close(); fs.rmSync(profile, { recursive: true, force: true });
