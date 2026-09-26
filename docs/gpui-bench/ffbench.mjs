// Firefox check for the GPUI wasm client: puppeteer-core over WebDriver BiDi with the stock
// Firefox install, fresh profile per run. MODE=bench reads the app's own ?bench=1 log (same
// events as wasmbench.mjs); MODE=parity does a short functional pass with synthetic input.
import puppeteer from 'puppeteer-core';
import { execSync } from 'node:child_process';
import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path';
const url = process.argv[2] || 'http://192.168.0.101:8889/gpui/?bench=1';
const runs = +(process.argv[3] || 3);
const MODE = process.env.MODE || 'bench';
const outDir = process.env.OUT_DIR || 'ff'; fs.mkdirSync(outDir, { recursive: true });
const W = 1200, H = 600, IDLE_S = +(process.env.IDLE_S || 20);
const prefs = JSON.parse(process.env.FF_PREFS || '{}');
const sleep = ms => new Promise(r => setTimeout(r, ms));

function treeStats(profile) { // main firefox.exe (its command line names the profile) + all descendants
  const needle = path.basename(profile);
  const ps = `$all = Get-CimInstance Win32_Process -Filter \\"Name='firefox.exe'\\"; $root = $all | Where-Object { $_.CommandLine -like '*${needle}*' -and $_.CommandLine -notlike '*-contentproc*' } | Select -First 1; $ids = @($root.ProcessId); $added = $true; while ($added) { $added = $false; foreach ($p in $all) { if ($ids -notcontains $p.ProcessId -and $ids -contains $p.ParentProcessId) { $ids += $p.ProcessId; $added = $true } } }; $all | Where-Object { $ids -contains $_.ProcessId } | ForEach-Object { $g = Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue; $t = if ($_.CommandLine -match ' (tab|gpu|rdd|socket|utility|gmplugin|forkserver)\\s*$') { $matches[1] } elseif ($_.CommandLine -like '*-contentproc*') { ($_.CommandLine -split ' ')[-1] } else { 'main' }; [pscustomobject]@{ id = $_.ProcessId; type = $t; ws = $g.WorkingSet64; priv = $g.PrivateMemorySize64; peak = $g.PeakWorkingSet64; cpu = $g.TotalProcessorTime.TotalMilliseconds } } | ConvertTo-Json -Compress`;
  let arr = JSON.parse(execSync(`powershell -NoProfile -Command "${ps}"`).toString() || '[]'); if (!Array.isArray(arr)) arr = [arr];
  const sum = (f, k) => arr.filter(f).reduce((a, p) => a + (p[k] || 0), 0);
  return { procs: arr.length, types: arr.map(p => p.type), total_ws: sum(() => true, 'ws'), total_private: sum(() => true, 'priv'),
    gpu_ws: sum(p => p.type === 'gpu', 'ws'), gpu_private: sum(p => p.type === 'gpu', 'priv'),
    max_tab_ws: Math.max(0, ...arr.filter(p => p.type === 'tab').map(p => p.ws)), max_tab_private: Math.max(0, ...arr.filter(p => p.type === 'tab').map(p => p.priv)),
    cpu_ms: sum(() => true, 'cpu') };
}
async function launch() {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'anorak-ff-'));
  const browser = await puppeteer.launch({ browser: 'firefox', executablePath: 'C:\\Program Files\\Mozilla Firefox\\firefox.exe',
    headless: false, userDataDir: profile, defaultViewport: { width: W, height: H }, args: [`-width=${W + 16}`, `-height=${H + 120}`],
    extraPrefsFirefox: prefs });
  const page = (await browser.pages())[0] || await browser.newPage();
  const lines = []; page.on('console', m => lines.push(m.text()));
  page.on('pageerror', e => lines.push('[pageerror] ' + e.message));
  return { profile, browser, page, lines };
}
const benchLog = async page => (await page.evaluate(() => (window.__anorakBench || []).slice())).map(s => JSON.parse(s));
async function waitEvent(page, name, lines, timeoutMs = 60000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) { const log = await benchLog(page); const e = log.find(x => x.event === name); if (e) return { e, log }; await sleep(20); }
  console.error('TIMEOUT', name, lines.slice(-20).join('\n')); throw new Error('timeout ' + name);
}
const navTiming = page => page.evaluate(() => { const n = performance.getEntriesByType('navigation')[0];
  const r = performance.getEntriesByType('resource').filter(e => e.name.endsWith('.wasm'))[0];
  return { dcl_ms: n.domContentLoadedEventEnd, load_ms: n.loadEventEnd, wasm_response_end_ms: r ? r.responseEnd : null, wasm_transfer: r ? r.transferSize : null }; });

if (MODE === 'bench') for (let r = 0; r < runs; r++) {
  const res = { run: r, url, browser: 'firefox' };
  const { profile, browser, page, lines } = await launch();
  res.ua = await browser.version();
  await page.goto('about:blank'); await sleep(1000);
  await page.goto(url, { waitUntil: 'domcontentloaded' });
  let { e: ff } = await waitEvent(page, 'first_frame', lines);
  res.cold = { first_frame_ms: ff.perf_now_ms, wasm_start_to_frame_ms: ff.elapsed_ms, ...(await navTiming(page)) };
  res.backend = (lines.find(l => l.includes('Browser graphics initialized successfully')) || '').replace(/.*with /, '') || null;
  Object.assign(res, await page.evaluate(() => ({ secure: isSecureContext, webgpu_api: !!navigator.gpu, dpr: devicePixelRatio })));
  const { e: s2r } = await waitEvent(page, 'search_to_render', lines);
  await waitEvent(page, 'results_loaded', lines); await sleep(700);
  res.after_results = treeStats(profile);
  const { log } = await waitEvent(page, 'done', lines, 90000);
  res.search_to_render_ms = s2r.elapsed_ms; res.search_http_ms = s2r.http_ms; res.rows = s2r.visible_rows;
  res.filter_sort = log.filter(x => x.event?.startsWith('filter_sort')).map(x => ({ ev: x.event, elapsed_ms: x.elapsed_ms, compute_ms: x.compute_ms }));
  res.scroll = log.find(x => x.event === 'scroll');
  await sleep(300); res.after_script = treeStats(profile);
  const c0 = res.after_script.cpu_ms; await sleep(IDLE_S * 1000);
  res.idle_cpu_ms_per_s_total = (treeStats(profile).cpu_ms - c0) / IDLE_S;
  await page.goto('about:blank'); await sleep(1000);
  await page.goto(url, { waitUntil: 'domcontentloaded' });
  ({ e: ff } = await waitEvent(page, 'first_frame', lines));
  res.warm = { first_frame_ms: ff.perf_now_ms, wasm_start_to_frame_ms: ff.elapsed_ms, ...(await navTiming(page)) };
  res.warnings = lines.filter(l => /WARN|ERROR|panic/i.test(l)).map(l => l.slice(0, 200)).slice(0, 10);
  await browser.close(); fs.rmSync(profile, { recursive: true, force: true });
  if (process.env.OUT_JSONL) fs.appendFileSync(process.env.OUT_JSONL, JSON.stringify(res) + '\n');
  console.error(`run ${r}: ${res.ua} backend=${res.backend} cold=${res.cold.first_frame_ms?.toFixed(1)} warm=${res.warm.first_frame_ms?.toFixed(1)} s2r=${res.search_to_render_ms} tabWS=${(res.after_script.max_tab_ws / 1048576).toFixed(1)}MB totalWS=${(res.after_script.total_ws / 1048576).toFixed(1)}MB idle=${res.idle_cpu_ms_per_s_total.toFixed(1)} scroll=${JSON.stringify(res.scroll)}`);
}

if (MODE === 'parity') {
  const { profile, browser, page, lines } = await launch();
  try {
  const log = (...a) => console.log(...a);
  const queries = []; page.on('request', r => { const u = r.url(); if (u.includes('/api/query')) queries.push(decodeURIComponent(new URL(u).searchParams.get('search_term') || '')); });
  const last = async () => { await sleep(500); return queries[queries.length - 1]; };
  const shot = async n => { await sleep(300); await page.screenshot({ path: `${outDir}/${n}.png` }); log('shot', n); };
  const k = async combo => { const ks = combo.split('+'); const key = ks.pop(); for (const m of ks) await page.keyboard.down(m);
    await page.keyboard.press(key); for (const m of ks.reverse()) await page.keyboard.up(m); await sleep(120); };
  await page.goto(url, { waitUntil: 'load' }); await sleep(3000);
  log('UA', await browser.version());
  log('INFO', JSON.stringify(await page.evaluate(() => ({ secure: isSecureContext, gpu: !!navigator.gpu, clip: !!navigator.clipboard, dpr: devicePixelRatio,
    canvas: [...document.querySelectorAll('canvas')].map(c => [c.width, c.height, c.clientWidth, c.clientHeight]), active: document.activeElement?.tagName }))));
  log('backend', lines.find(l => l.includes('Browser graphics initialized successfully')) || '(none logged)');
  await page.mouse.click(300, 26); // make sure the page has focus
  await page.keyboard.type('one punch'); await page.keyboard.press('Enter'); log('type+Enter ->', JSON.stringify(await last()));
  await sleep(800); await shot('01_results');
  await page.keyboard.press('Home'); await page.keyboard.type('X '); await page.keyboard.press('End'); await page.keyboard.press('Backspace'); await page.keyboard.press('Enter');
  log('Home/End/Backspace (expect "X one punc") ->', JSON.stringify(await last()));
  await k('Control+a'); await page.keyboard.type('naïve café – ✓'); await page.keyboard.press('Enter'); log('select-all + non-ASCII ->', JSON.stringify(await last()));
  await k('Shift+Home'); await k('Backspace'); await page.keyboard.type('one punch'); await sleep(120);
  await k('Control+a'); await k('Control+c'); await k('End'); await k('Control+v'); await page.keyboard.press('Enter');
  log('copy/End/paste (expect "one punchone punch") ->', JSON.stringify(await last()));
  await k('Control+a'); await page.keyboard.type('tab test'); await k('Tab'); await page.keyboard.type('Q'); await page.keyboard.press('Enter');
  log('Tab then Q ->', JSON.stringify(await last()), 'active', await page.evaluate(() => document.activeElement?.tagName));
  await k('Shift+Tab'); await k('Control+a'); await page.keyboard.type('one punch'); await page.keyboard.press('Enter'); await sleep(800);
  await page.mouse.move(200, 175); await page.mouse.move(203, 175); await sleep(1500); await shot('50_tooltip');
  await page.mouse.move(200, 275); for (let i = 0; i < 5; i++) { await page.mouse.wheel({ deltaY: 120 }); await sleep(50); } await sleep(400); await shot('90_wheel');
  await page.mouse.click(1066, 30); await shot('60_filter');
  await page.keyboard.press('Escape'); await sleep(200);
  log('warnings', JSON.stringify(lines.filter(l => /WARN|ERROR|panic/i.test(l)).map(l => l.slice(0, 200)).slice(0, 10)));
  } finally { await browser.close().catch(() => {}); fs.rmSync(profile, { recursive: true, force: true }); }
}
