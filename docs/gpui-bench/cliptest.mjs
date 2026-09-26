// Does synthetic Ctrl+C / Ctrl+V reach the browser's native clipboard at all here?
// 1) plain textarea on the GPUI page's origin (control), 2) GPUI field, observing DOM copy/paste events.
import { chromium } from 'playwright-core';
import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path';
const url = process.argv[2];
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'anorak-clip-'));
const ctx = await chromium.launchPersistentContext(profile, { executablePath: 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  headless: false, viewport: { width: 1200, height: 600 }, args: ['--no-first-run', '--disable-extensions', '--window-position=0,0'] });
const page = ctx.pages()[0];
const queries = []; page.on('request', r => { if (r.url().includes('/api/query')) queries.push(new URL(r.url()).searchParams.get('search_term')); });
await page.goto(url); await page.waitForTimeout(2500);
await page.evaluate(() => { window.__ev = []; for (const t of ['copy', 'cut', 'paste', 'keydown']) document.addEventListener(t, e => window.__ev.push(t + (e.key ? ':' + e.key : '') + (e.defaultPrevented ? '(prevented)' : '') + ' on ' + e.target.tagName), true); });
// GPUI field
await page.keyboard.type('alpha beta'); await page.keyboard.press('Control+a'); await page.waitForTimeout(100);
const mirror = await page.evaluate(() => { const t = document.querySelector('textarea'); return { value: t.value, s: t.selectionStart, e: t.selectionEnd, focused: document.activeElement === t }; });
console.log('mirror after ctrl+a', JSON.stringify(mirror));
await page.keyboard.press('Control+c'); await page.waitForTimeout(100);
await page.keyboard.press('End'); await page.keyboard.press('Control+v'); await page.waitForTimeout(100); await page.keyboard.press('Enter'); await page.waitForTimeout(500);
console.log('gpui: query after copy + End + paste:', JSON.stringify(queries.at(-1)));
console.log('events', JSON.stringify(await page.evaluate(() => window.__ev.filter(x => !x.startsWith('keydown:') || /c|v/.test(x)))));
// control: plain textarea
await page.evaluate(() => { const a = document.createElement('textarea'); a.id = 'ctl'; a.value = 'control text'; a.style = 'position:fixed;left:0;top:0;z-index:9;'; document.body.appendChild(a); a.focus(); a.select(); window.__ev = []; });
await page.keyboard.press('Control+c'); await page.keyboard.press('End'); await page.keyboard.press('Control+v');
console.log('control textarea value:', JSON.stringify(await page.evaluate(() => document.getElementById('ctl').value)), JSON.stringify(await page.evaluate(() => window.__ev)));
await ctx.close(); fs.rmSync(profile, { recursive: true, force: true });
