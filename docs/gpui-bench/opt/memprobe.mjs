// wasm linear memory size (bytes) after the in-app bench script, fresh Chrome profile per run.
import { chromium } from 'playwright-core';
import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path';
const url = process.argv[2] || 'http://192.168.0.101:8889/gpui/?bench=1';
const runs = +(process.argv[3] || 3);
const extra = (process.env.CHROME_ARGS || '').split(' ').filter(Boolean);
for (let r = 0; r < runs; r++) {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'anorak-mem-'));
  const ctx = await chromium.launchPersistentContext(profile, { executablePath: 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
    headless: false, viewport: { width: 1200, height: 600 }, args: ['--no-first-run', '--no-default-browser-check', '--disable-extensions', '--window-size=1216,720', '--window-position=0,0', ...extra] });
  const page = ctx.pages()[0] || await ctx.newPage();
  await page.addInitScript(() => addEventListener('TrunkApplicationStarted', e => { window.__wx = e.detail.wasm; }));
  await page.goto(url);
  const sample = () => page.evaluate(() => window.__wx ? window.__wx.memory.buffer.byteLength : null);
  let first = null, t0 = Date.now();
  while (Date.now() - t0 < 60000) {
    const log = await page.evaluate(() => (window.__anorakBench || []).map(s => JSON.parse(s).event));
    if (first === null && log.includes('first_frame')) first = await sample();
    if (log.includes('done')) break;
    await page.waitForTimeout(50);
  }
  const done = await sample();
  const heap = await page.evaluate(() => performance.memory ? performance.memory.usedJSHeapSize : null);
  console.log(JSON.stringify({ run: r, wasm_mem_at_first_frame: first, wasm_mem_after_script: done, js_heap_used: heap }));
  await ctx.close(); fs.rmSync(profile, { recursive: true, force: true });
}
