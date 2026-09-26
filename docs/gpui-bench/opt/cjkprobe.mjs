// Time the first Canvas measureText/fillText of CJK text in a fresh Chrome profile (gpui_web's Canvas fallback path).
import { chromium } from 'playwright-core';
import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path';
const runs = +(process.argv[2] || 3);
for (let r = 0; r < runs; r++) {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'anorak-cjk-'));
  const ctx = await chromium.launchPersistentContext(profile, { executablePath: 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe', headless: false,
    args: ['--no-first-run', '--no-default-browser-check', '--disable-extensions'] });
  const page = ctx.pages()[0] || await ctx.newPage();
  await page.goto('http://192.168.0.101:8889/gpui/?probe=1'.replace('/gpui/?probe=1', '/favicon.ico'));
  await page.waitForTimeout(800);
  const res = await page.evaluate(() => {
    const f = 'normal 400 14px "IBM Plex Sans", sans-serif';
    const c = new OffscreenCanvas(64, 64).getContext('2d', { willReadFrequently: true });
    const t = (fn) => { const a = performance.now(); fn(); return +(performance.now() - a).toFixed(2); };
    const o = {};
    c.font = f;
    o.latin_measure = t(() => c.measureText('One-Punch Man S03E07'));
    o.cjk_measure_1 = t(() => c.measureText('ワ'));
    o.cjk_measure_2 = t(() => c.measureText('ン'));
    o.cjk_fill_1 = t(() => { c.fillText('パ', 0, 20); c.getImageData(0, 0, 32, 32); });
    o.han_measure_1 = t(() => c.measureText('第'));
    o.han_measure_2 = t(() => c.measureText('巻'));
    o.hangul_measure_1 = t(() => c.measureText('한'));
    c.font = 'normal 600 14px "IBM Plex Sans", sans-serif';
    o.cjk_bold_measure_1 = t(() => c.measureText('マ'));
    c.font = 'normal 400 14px emoji';
    o.emoji_measure_1 = t(() => c.measureText('😀'));
    return o;
  });
  console.log(JSON.stringify({ run: r, ...res }));
  await ctx.close(); fs.rmSync(profile, { recursive: true, force: true });
}
