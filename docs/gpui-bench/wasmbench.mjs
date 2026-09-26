// GPUI WASM measurement harness: headed Chrome (fresh profile per run) via playwright-core.
// Same viewport, idle window and process accounting as webbench.mjs; timings come from the
// app's own bench log (window.__anorakBench), taken at the paint of the frame that shows
// the change, exactly like the native --bench mode.
import { chromium } from 'playwright-core';
import { execSync } from 'node:child_process';
import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path';

const url = process.argv[2] || 'http://192.168.0.101:8889/gpui/?bench=1';
const runs = +(process.argv[3] || 5);
const W = 1200, H = 600;
const IDLE_S = +(process.env.IDLE_S || 20);
const extra = (process.env.CHROME_ARGS || '').split(' ').filter(Boolean);
const out = [];

function instanceProcs(profile) {
  const needle = path.basename(profile);
  const js = execSync(`powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \\"Name='chrome.exe'\\" | Where-Object { $_.CommandLine -like '*${needle}*' } | Select-Object ProcessId,CommandLine | ConvertTo-Json -Compress"`).toString();
  let arr = JSON.parse(js || '[]'); if (!Array.isArray(arr)) arr = [arr];
  return arr.map(p => { const m = /--type=([a-z-]+)/.exec(p.CommandLine || ''); let type = m ? m[1] : 'browser';
    if (/--extension-process/.test(p.CommandLine || '')) type = 'extension';
    if (type === 'utility') { const u = /--utility-sub-type=([\w.]+)/.exec(p.CommandLine); type = 'utility:' + (u ? u[1].split('.').pop() : '?'); }
    return { id: p.ProcessId, type }; });
}
function procStats(pids) {
  if (!pids.length) return [];
  const js = execSync(`powershell -NoProfile -Command "Get-Process -Id ${pids.join(',')} -ErrorAction SilentlyContinue | Select-Object Id,WorkingSet64,PeakWorkingSet64,PrivateMemorySize64,@{n='Cpu';e={$_.TotalProcessorTime.TotalMilliseconds}} | ConvertTo-Json -Compress; exit 0"`).toString();
  const arr = JSON.parse(js || '[]'); return Array.isArray(arr) ? arr : [arr];
}
function snapshot(procInfo) {
  const st = procStats(procInfo.map(p => p.id)); const byId = Object.fromEntries(st.map(x => [x.Id, x]));
  const ps = procInfo.map(p => ({ type: p.type, id: p.id, ...(byId[p.id] || {}) })).filter(p => p.WorkingSet64);
  const sum = (f, k) => ps.filter(f).reduce((a, p) => a + p[k], 0);
  return { renderer_ws: sum(p => p.type === 'renderer', 'WorkingSet64'), renderer_private: sum(p => p.type === 'renderer', 'PrivateMemorySize64'),
    renderer_peak_ws: sum(p => p.type === 'renderer', 'PeakWorkingSet64'),
    gpu_ws: sum(p => p.type === 'gpu-process', 'WorkingSet64'), gpu_private: sum(p => p.type === 'gpu-process', 'PrivateMemorySize64'),
    total_ws: sum(() => true, 'WorkingSet64'), total_private: sum(() => true, 'PrivateMemorySize64'), procs: ps.length, cpu_ms: sum(() => true, 'Cpu') };
}
async function benchLog(page) { return (await page.evaluate(() => (window.__anorakBench || []).slice())).map(s => JSON.parse(s)); }
async function waitEvent(page, name, timeoutMs = 60000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    const log = await benchLog(page); const e = log.find(x => x.event === name); if (e) return { e, log };
    await page.waitForTimeout(20);
  }
  console.error('TIMEOUT', name, JSON.stringify(await benchLog(page)).slice(0, 2000));
  console.error((globalThis.__consoleLines || []).slice(-30).join('\n'));
  throw new Error(`timeout waiting for ${name}`);
}
function track(cdp) {
  const reqs = new Map();
  cdp.on('Network.requestWillBeSent', e => reqs.set(e.requestId, { url: e.request.url, wire: 0, decoded: 0 }));
  cdp.on('Network.responseReceived', e => { const q = reqs.get(e.requestId); if (q) { q.status = e.response.status; q.fromCache = e.response.fromDiskCache || e.response.fromMemoryCache || false; q.mime = e.response.mimeType; } });
  cdp.on('Network.dataReceived', e => { const q = reqs.get(e.requestId); if (q) q.decoded += e.dataLength; });
  cdp.on('Network.loadingFinished', e => { const q = reqs.get(e.requestId); if (q) { q.wire = e.encodedDataLength; q.done = true; } });
  return reqs;
}
const summarize = reqs => { const d = [...reqs.values()].filter(q => q.done);
  return { requests: d.length, wire_bytes: d.reduce((a, q) => a + q.wire, 0), decoded_bytes: d.reduce((a, q) => a + q.decoded, 0),
    detail: d.map(q => ({ url: q.url.replace(/^https?:\/\/[^/]+/, '').replace(/\?.*$/, '').slice(0, 70), status: q.status, cache: q.fromCache, wire: q.wire, decoded: q.decoded })) }; };
async function navTiming(page) { return page.evaluate(() => { const n = performance.getEntriesByType('navigation')[0];
  const r = performance.getEntriesByType('resource').filter(e => e.name.endsWith('.wasm'))[0];
  return { dcl_ms: n.domContentLoadedEventEnd, load_ms: n.loadEventEnd, wasm_response_end_ms: r ? r.responseEnd : null }; }); }

for (let r = 0; r < runs; r++) {
  const res = { run: r, url };
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'anorak-wasm-'));
  const tLaunch0 = Date.now();
  const ctx = await chromium.launchPersistentContext(profile, {
    executablePath: 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
    headless: false, viewport: { width: W, height: H },
    args: ['--no-first-run', '--no-default-browser-check', '--disable-extensions', `--window-size=${W + 16},${H + 120}`, '--window-position=0,0', ...extra],
  });
  res.browser_launch_ms = Date.now() - tLaunch0;
  const page = ctx.pages()[0] || await ctx.newPage();
  const consoleLines = []; globalThis.__consoleLines = consoleLines; page.on('console', m => consoleLines.push(m.text()));
  const cdp = await ctx.newCDPSession(page);
  await cdp.send('Network.enable');
  await page.goto('about:blank'); await page.waitForTimeout(1000); // warm Chrome
  let reqs = track(cdp);

  // 1. cold-cache load -> first frame painted (perf_now_ms is relative to navigation start)
  await page.goto(url, { waitUntil: 'commit' });
  let { e: ff } = await waitEvent(page, 'first_frame');
  res.cold = { first_frame_ms: ff.perf_now_ms, wasm_start_to_frame_ms: ff.elapsed_ms, ...(await navTiming(page)) };
  const backendLine = consoleLines.find(l => l.includes('Browser graphics initialized successfully'));
  res.backend = backendLine ? backendLine.replace(/.*with /, '') : null;
  const info = await page.evaluate(() => ({ secure: isSecureContext, webgpu_api: !!navigator.gpu, dpr: devicePixelRatio }));
  Object.assign(res, info);

  // 2. in-app bench script (starts 500 ms after first frame): search, 5x filter/sort, scroll
  const { e: s2r } = await waitEvent(page, 'search_to_render');
  res.page_load = summarize(reqs); // everything up to results incl. /api/query
  await waitEvent(page, 'results_loaded');
  await page.waitForTimeout(700);
  const procInfo = instanceProcs(profile);
  res.after_results = snapshot(procInfo);
  const { log } = await waitEvent(page, 'done', 90000);
  res.search_to_render_ms = s2r.elapsed_ms; res.search_http_ms = s2r.http_ms; res.search_parse_ms = s2r.json_parse_ms; res.rows = s2r.visible_rows;
  res.filter_sort = log.filter(x => x.event?.startsWith('filter_sort')).map(x => ({ ev: x.event, elapsed_ms: x.elapsed_ms, compute_ms: x.compute_ms, rows: x.visible_rows }));
  res.scroll = log.find(x => x.event === 'scroll');
  await page.waitForTimeout(300);
  res.after_script = snapshot(procInfo);
  // 3. idle CPU (all processes of this browser instance)
  const c0 = snapshot(procInfo).cpu_ms; const rp = procInfo.filter(p => p.type === 'renderer').map(p => p.id);
  const rc0 = procStats(rp).reduce((a, s) => a + s.Cpu, 0);
  const gp = procInfo.filter(p => p.type === 'gpu-process').map(p => p.id); const gc0 = procStats(gp).reduce((a, s) => a + s.Cpu, 0);
  await page.waitForTimeout(IDLE_S * 1000);
  res.idle_cpu_ms_per_s_total = (snapshot(procInfo).cpu_ms - c0) / IDLE_S;
  res.idle_cpu_ms_per_s_renderer = (procStats(rp).reduce((a, s) => a + s.Cpu, 0) - rc0) / IDLE_S;
  res.idle_cpu_ms_per_s_gpu = (procStats(gp).reduce((a, s) => a + s.Cpu, 0) - gc0) / IDLE_S;

  // 4. warm-cache load: same profile, revisit from about:blank
  await page.goto('about:blank'); await page.waitForTimeout(1000);
  reqs = track(cdp);
  await page.goto(url, { waitUntil: 'commit' });
  ({ e: ff } = await waitEvent(page, 'first_frame'));
  res.warm = { first_frame_ms: ff.perf_now_ms, wasm_start_to_frame_ms: ff.elapsed_ms, ...(await navTiming(page)) };
  await page.waitForTimeout(300);
  res.warm_load = summarize(reqs);

  await ctx.close();
  fs.rmSync(profile, { recursive: true, force: true });
  out.push(res);
  if (process.env.OUT_JSONL) fs.appendFileSync(process.env.OUT_JSONL, JSON.stringify(res) + '\n');
  console.error(`run ${r}: backend=${res.backend} cold=${res.cold.first_frame_ms?.toFixed(1)} warm=${res.warm.first_frame_ms?.toFixed(1)} s2r=${res.search_to_render_ms} rendererWS=${(res.after_script.renderer_ws/1048576).toFixed(1)}MB gpuWS=${(res.after_script.gpu_ws/1048576).toFixed(1)}MB idle=${res.idle_cpu_ms_per_s_total.toFixed(1)}`);
}
console.log(JSON.stringify(out));
