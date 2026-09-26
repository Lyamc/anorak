// Web UI measurement harness: headed Chrome (fresh profile per run) via playwright-core.
import { chromium } from 'playwright-core';
import { execSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const url = process.argv[2] || 'http://192.168.0.101:8888/';
const runs = +(process.argv[3] || 5);
const term = 'one punch';
const W = 1200, H = 600;
const IDLE_S = +(process.env.IDLE_S || 20);
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
  const ids = pids.join(',');
  const js = execSync(`powershell -NoProfile -Command "Get-Process -Id ${ids} -ErrorAction SilentlyContinue | Select-Object Id,WorkingSet64,PeakWorkingSet64,PrivateMemorySize64,@{n='Cpu';e={$_.TotalProcessorTime.TotalMilliseconds}} | ConvertTo-Json -Compress"`).toString();
  const arr = JSON.parse(js || '[]');
  return Array.isArray(arr) ? arr : [arr];
}

for (let r = 0; r < runs; r++) {
  const res = { run: r };
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'anorak-web-'));
  const tLaunch0 = Date.now();
  const ctx = await chromium.launchPersistentContext(profile, {
    executablePath: 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
    headless: false, viewport: { width: W, height: H },
    args: ['--no-first-run', '--no-default-browser-check', '--disable-extensions', `--window-size=${W + 16},${H + 120}`, '--window-position=0,0'],
  });
  res.browser_launch_ms = Date.now() - tLaunch0;
  const page = ctx.pages()[0] || await ctx.newPage();
  const cdp = await ctx.newCDPSession(page);
  await cdp.send('Network.enable');
  await cdp.send('Network.setCacheDisabled', { cacheDisabled: true });
  const reqs = new Map();
  cdp.on('Network.requestWillBeSent', e => reqs.set(e.requestId, { url: e.request.url, method: e.request.method, decoded: 0, wire: 0 }));
  cdp.on('Network.responseReceived', e => { const q = reqs.get(e.requestId); if (q) { q.status = e.response.status; q.mime = e.response.mimeType; q.enc = (e.response.headers['content-encoding'] || e.response.headers['Content-Encoding'] || ''); } });
  cdp.on('Network.dataReceived', e => { const q = reqs.get(e.requestId); if (q) q.decoded += e.dataLength; });
  cdp.on('Network.loadingFinished', e => { const q = reqs.get(e.requestId); if (q) { q.wire = e.encodedDataLength; q.done = true; } });

  // 1. page load (cold cache)
  await page.goto(url, { waitUntil: 'load' });
  await page.waitForLoadState('networkidle');
  await page.waitForTimeout(500);
  const nav = await page.evaluate(() => {
    const n = performance.getEntriesByType('navigation')[0];
    const fcp = performance.getEntriesByName('first-contentful-paint')[0];
    return { dcl_ms: n.domContentLoadedEventEnd, load_ms: n.loadEventEnd, fcp_ms: fcp ? fcp.startTime : null,
             htmx: typeof window.htmx !== 'undefined' };
  });
  Object.assign(res, nav);
  const loadReqs = [...reqs.values()].filter(q => q.done);
  res.page_load = {
    requests: loadReqs.length,
    wire_bytes: loadReqs.reduce((a, q) => a + q.wire, 0),
    decoded_bytes: loadReqs.reduce((a, q) => a + q.decoded, 0),
    detail: loadReqs.map(q => ({ url: q.url.replace(/\?.*$/, '').slice(0, 90), status: q.status, enc: q.enc, wire: q.wire, decoded: q.decoded })),
  };
  reqs.clear();

  // 2. search -> 100 rows rendered (htmx:afterSettle, then next frame painted)
  res.search_to_render_ms = await page.evaluate((term) => new Promise(resolve => {
    const input = document.getElementById('search_term');
    input.value = term;
    const form = document.querySelector('form.search-form');
    const t0 = performance.now();
    document.body.addEventListener('htmx:afterSettle', function h(e) {
      if (e.detail?.target?.id !== 'mainContent') return;
      document.body.removeEventListener('htmx:afterSettle', h);
      requestAnimationFrame(() => setTimeout(() => resolve(performance.now() - t0), 0));
    });
    form.requestSubmit();
  }), term);
  res.rows = await page.locator('#resultsTable tbody tr').count();
  await page.waitForTimeout(300);
  const qr = [...reqs.values()].find(q => q.url.includes('/query/'));
  res.query_response = qr ? { wire: qr.wire, decoded: qr.decoded, enc: qr.enc } : null;
  await page.waitForLoadState('networkidle');
  const sr = [...reqs.values()].filter(q => q.done);
  res.search_requests = { requests: sr.length, wire_bytes: sr.reduce((a, q) => a + q.wire, 0), decoded_bytes: sr.reduce((a, q) => a + q.decoded, 0),
    detail: sr.map(q => ({ url: q.url.replace(/\?.*$/, '').slice(0, 90), status: q.status, enc: q.enc, wire: q.wire, decoded: q.decoded })) };

  // processes of this browser instance (matched by profile dir on the command line)
  let procInfo = instanceProcs(profile);
  res.results_ws = null;
  { const st = procStats(procInfo.map(p => p.id)); const byId = Object.fromEntries(st.map(x => [x.Id, x]));
    const rs = procInfo.filter(p => p.type === 'renderer').map(p => byId[p.id]).filter(Boolean);
    res.renderer_ws_after_results = rs.map(x => x.WorkingSet64); }
  await page.waitForTimeout(1000);

  // 3. filter + re-sort, then clear; 5 reps each (same script as GPUI --bench)
  res.filter_sort = [];
  for (let rep = 0; rep < 5; rep++) {
    for (const apply of [true, false]) {
      const m = await page.evaluate((apply) => new Promise(resolve => {
        const fn = document.getElementById('filterName'), sp = document.getElementById('sortPrimary');
        const t0 = performance.now();
        fn.value = apply ? '1080' : '';
        fn.dispatchEvent(new Event('input', { bubbles: true }));
        sp.value = apply ? 'size:desc' : 'seeders:desc';
        sp.dispatchEvent(new Event('change', { bubbles: true }));
        const tSync = performance.now() - t0;
        const shown = document.getElementById('visibleCount').textContent;
        requestAnimationFrame(() => setTimeout(() => resolve({ apply, elapsed_ms: performance.now() - t0, compute_ms: tSync, shown }), 0));
      }), apply);
      res.filter_sort.push(m);
      await page.waitForTimeout(150);
    }
  }

  // 4. scripted scroll: 20 px per frame for 120 frames
  res.scroll = await page.evaluate(() => new Promise(resolve => {
    window.scrollTo(0, 0);
    const ts = []; let n = 0;
    function step(t) { ts.push(t); if (n++ < 120) { window.scrollBy(0, 20); requestAnimationFrame(step); } else {
      const iv = ts.slice(1).map((t, i) => t - ts[i]).sort((a, b) => a - b);
      const pct = p => iv[Math.round((iv.length - 1) * p)];
      resolve({ frames: ts.length, mean: iv.reduce((a, b) => a + b, 0) / iv.length, p50: pct(0.5), p95: pct(0.95), max: iv[iv.length - 1], final_scroll: window.scrollY });
    } }
    requestAnimationFrame(step);
  }));
  await page.evaluate(() => window.scrollTo(0, 0));

  // memory after the interaction script (current + peak)
  if (procInfo) {
    const pids = procInfo.map(p => p.id).filter(Boolean);
    const stats = procStats(pids);
    const byId = Object.fromEntries(stats.map(s => [s.Id, s]));
    res.processes = procInfo.map(p => ({ type: p.type, id: p.id, ...(byId[p.id] || {}) }));
    const renderers = res.processes.filter(p => p.type === 'renderer' && p.WorkingSet64);
    res.renderer_ws = renderers.map(p => p.WorkingSet64);
    res.renderer_peak_ws = renderers.map(p => p.PeakWorkingSet64);
    res.renderer_private = renderers.map(p => p.PrivateMemorySize64);
    res.total_ws = res.processes.reduce((a, p) => a + (p.WorkingSet64 || 0), 0);
    res.total_private = res.processes.reduce((a, p) => a + (p.PrivateMemorySize64 || 0), 0);
    // 5. idle CPU over IDLE_S seconds (all processes of this browser instance)
    const cpu0 = procStats(pids).reduce((a, s) => a + s.Cpu, 0);
    const rcpu0 = procStats(renderers.map(p => p.id)).reduce((a, s) => a + s.Cpu, 0);
    await page.waitForTimeout(IDLE_S * 1000);
    const cpu1 = procStats(pids).reduce((a, s) => a + s.Cpu, 0);
    const rcpu1 = procStats(renderers.map(p => p.id)).reduce((a, s) => a + s.Cpu, 0);
    res.idle_cpu_ms_per_s_total = (cpu1 - cpu0) / IDLE_S;
    res.idle_cpu_ms_per_s_renderer = (rcpu1 - rcpu0) / IDLE_S;
  }
  await ctx.close();
  fs.rmSync(profile, { recursive: true, force: true });
  out.push(res);
  console.error(`run ${r}: dcl=${res.dcl_ms?.toFixed(1)} s2r=${res.search_to_render_ms?.toFixed(1)} rows=${res.rows} rendererWS=${res.renderer_ws}`);
}
console.log(JSON.stringify(out));
