// Bold (expanded-row) rendering of Cyrillic/Greek titles with the subset SemiBold font.
import { chromium } from 'playwright-core';
import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path';
const url = process.argv[2]; const out = process.argv[3] || 'cyr'; fs.mkdirSync(out, { recursive: true });
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'anorak-cyr-'));
const ctx = await chromium.launchPersistentContext(profile, { executablePath: 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  headless: false, viewport: { width: 1200, height: 600 }, args: ['--no-first-run', '--no-default-browser-check', '--disable-extensions', '--window-size=1216,720', '--window-position=0,0'] });
const page = ctx.pages()[0] || await ctx.newPage();
page.on('console', m => { const t = m.text(); if (/WARN|ERROR|panic/i.test(t)) console.log('[console]', t.slice(0, 200)); });
await page.goto(url, { waitUntil: 'load' });
await page.waitForFunction(() => document.querySelector('canvas'), null, { timeout: 30000 });
await page.waitForTimeout(1500);
await page.keyboard.type('one punch'); await page.keyboard.press('Enter'); await page.waitForTimeout(1500);
await page.screenshot({ path: `${out}/1_rows.png`, clip: { x: 0, y: 100, width: 1200, height: 200 } });
for (const [i, y] of [[0, 175], [1, 211], [2, 247]]) {
  await page.mouse.click(250, y); await page.waitForTimeout(500);
  await page.mouse.move(600, 20); await page.waitForTimeout(300);
  await page.screenshot({ path: `${out}/2_expanded_row${i}.png`, clip: { x: 0, y: 100, width: 1200, height: 500 } });
  await page.mouse.click(250, y); await page.waitForTimeout(300);
}
await ctx.close(); fs.rmSync(profile, { recursive: true, force: true });
console.log('ok');
