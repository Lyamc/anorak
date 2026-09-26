// Browser parity checks for the GPUI wasm client, using only synthetic (CDP) input.
// Oracle for text-field contents: the search_term of the /api/query request that Enter sends.
import { chromium } from 'playwright-core';
import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path';
const base = process.argv[2] || 'http://192.168.0.101:8889/gpui/';
const outDir = process.argv[3] || 'parity'; fs.mkdirSync(outDir, { recursive: true });
const steps = (process.env.STEPS || 'layout').split(',');
const extra = (process.env.CHROME_ARGS || '').split(' ').filter(Boolean);
const W = +(process.env.W || 1200), H = +(process.env.H || 600);
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'anorak-par-'));
const ctx = await chromium.launchPersistentContext(profile, {
  executablePath: 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  headless: false, viewport: process.env.NOVIEWPORT ? null : { width: W, height: H },
  ...(process.env.DSF ? { deviceScaleFactor: +process.env.DSF } : {}),
  args: ['--no-first-run', '--no-default-browser-check', '--disable-extensions', `--window-size=${W + 16},${H + 120}`, '--window-position=0,0', ...extra],
});
const page = ctx.pages()[0] || await ctx.newPage();
const log = (...a) => console.log(...a);
page.on('console', m => { const t = m.text(); if (/WARN|ERROR|error|panic/i.test(t) && !/integrity/.test(t)) log('[console]', t.slice(0, 240)); });
page.on('pageerror', e => log('[pageerror]', e.message));
const queries = []; const grabs = [];
page.on('request', r => { const u = r.url();
  if (u.includes('/api/query')) queries.push(decodeURIComponent(new URL(u).searchParams.get('search_term') || ''));
  if (u.includes('/send-to-rqbit')) grabs.push(r.postData()); });
const shot = async n => { await page.waitForTimeout(250); await page.screenshot({ path: `${outDir}/${n}.png` }); log('shot', n); };
const lastQuery = async () => { await page.waitForTimeout(400); return queries[queries.length - 1]; };
const cdp = await ctx.newCDPSession(page);
await page.goto(base + (process.env.QS || ''), { waitUntil: 'load' });
await page.waitForFunction(() => document.querySelector('canvas'), null, { timeout: 30000 });
await page.waitForTimeout(1500);
log('INFO', JSON.stringify(await page.evaluate(() => ({ dpr: devicePixelRatio, secure: isSecureContext, clip: !!navigator.clipboard,
  canvas: [...document.querySelectorAll('canvas')].map(c => [c.width, c.height, c.clientWidth, c.clientHeight]),
  active: document.activeElement && (document.activeElement.tagName + '.' + (document.activeElement.className || '')),
  textareas: document.querySelectorAll('textarea').length }))));
const P = JSON.parse(process.env.POS || '{}'); // coordinates discovered from the layout step
const click = async (x, y) => { await page.mouse.move(x, y); await page.mouse.down(); await page.mouse.up(); };

for (const step of steps) {
  log('== step', step);
  if (step === 'layout') {
    await shot('00_initial');
    await page.keyboard.type('one punch'); await page.keyboard.press('Enter');
    log('query ->', JSON.stringify(await lastQuery()));
    await page.waitForTimeout(800); await shot('01_results');
  }
  if (step === 'typing') { // edit keys: Home/End/Backspace/word-delete/selection-replace
    await page.keyboard.type('one punch'); await page.keyboard.press('Home'); await page.keyboard.type('X ');
    await page.keyboard.press('End'); await page.keyboard.press('Backspace'); await page.keyboard.press('Enter');
    log('typing (expect "X one punc") ->', JSON.stringify(await lastQuery()));
    await page.keyboard.press('Control+a'); await page.keyboard.type('attack on titan'); await page.keyboard.press('Enter');
    log('select-all + type (expect "attack on titan") ->', JSON.stringify(await lastQuery()));
    await page.keyboard.press('Shift+Home'); await page.keyboard.press('Backspace'); await page.keyboard.type('naïve café – ✓'); await page.keyboard.press('Enter');
    log('shift+home+backspace, non-ASCII (expect "naïve café – ✓") ->', JSON.stringify(await lastQuery()));
    await shot('10_typing');
  }
  if (step === 'ime') {
    await page.keyboard.press('Control+a'); await page.keyboard.press('Backspace');
    await cdp.send('Input.imeSetComposition', { text: 'わんぱん', selectionStart: 4, selectionEnd: 4 });
    await shot('20_ime_composing');
    await cdp.send('Input.imeSetComposition', { text: 'ワンパンマン', selectionStart: 6, selectionEnd: 6 });
    await cdp.send('Input.insertText', { text: 'ワンパンマン' });
    await page.keyboard.press('Enter');
    log('IME commit (expect "ワンパンマン") ->', JSON.stringify(await lastQuery()));
    await shot('21_ime_committed');
    await page.keyboard.press('Control+a'); await page.keyboard.press('Backspace');
    await cdp.send('Input.insertText', { text: '😀 emoji' }); await page.keyboard.press('Enter');
    log('insertText emoji (expect "😀 emoji") ->', JSON.stringify(await lastQuery()));
    await shot('22_emoji');
  }
  if (step === 'clipboard') {
    // in-app round trip through the browser's native clipboard (gpui_web mirrors the field into a
    // hidden textarea). Small waits let the mirror's selection sync land, as it would for a human.
    const k = async key => { await page.keyboard.press(key); await page.waitForTimeout(120); };
    await k('Control+a'); await page.keyboard.type('one punch'); await page.waitForTimeout(120);
    await k('Control+a'); await k('Control+c');
    await k('End'); await k('Control+v'); await page.keyboard.press('Enter');
    log('copy, End, paste (expect "one punchone punch") ->', JSON.stringify(await lastQuery()));
    await k('Control+a'); await k('Control+x'); await page.keyboard.type('cut:'); await k('Control+v'); await page.keyboard.press('Enter');
    log('select-all, cut, type "cut:", paste (expect "cut:one punchone punch") ->', JSON.stringify(await lastQuery()));
    await k('Control+a'); await page.keyboard.type('one punch'); await page.keyboard.press('Enter'); await page.waitForTimeout(800);
    log('navigator.clipboard ->', JSON.stringify(await page.evaluate(() => typeof navigator.clipboard)));
  }
  if (step === 'mouse_select') { // drag-select part of the search text with synthetic mouse events, then type over it
    await page.keyboard.press('Control+a'); await page.keyboard.type('one punch man');
    const [x0, y] = P.searchText || [20, 30];
    await page.mouse.move(x0, y); await page.mouse.down(); await page.mouse.move(x0 + 20, y, { steps: 5 }); await page.mouse.move(x0 + 300, y, { steps: 10 }); await page.mouse.up();
    await shot('30_drag_selected');
    await page.keyboard.type('Z'); await page.keyboard.press('Enter');
    log('drag-select all then type Z (expect "Z") ->', JSON.stringify(await lastQuery()));
    await page.keyboard.press('Control+a'); await page.keyboard.type('one punch man');
    await page.mouse.dblclick(x0 + 10, y); await page.keyboard.type('two'); await page.keyboard.press('Enter');
    log('double-click first word then type (expect "two punch man") ->', JSON.stringify(await lastQuery()));
    await page.keyboard.press('Control+a'); await page.keyboard.type('one punch'); await page.keyboard.press('Enter');
    await page.waitForTimeout(800);
  }
  if (step === 'tab') {
    await page.keyboard.press('Control+a'); await page.keyboard.type('tab test');
    await page.keyboard.press('Tab'); await page.keyboard.type('Q'); await page.keyboard.press('Enter');
    log('after Tab, typed Q + Enter ->', JSON.stringify(await lastQuery()), 'active:', await page.evaluate(() => document.activeElement?.tagName));
    await shot('40_after_tab');
    await page.keyboard.press('Shift+Tab'); await page.keyboard.type('R'); await page.keyboard.press('Enter');
    log('after Shift+Tab, typed R + Enter ->', JSON.stringify(await lastQuery()));
  }
  if (step === 'tooltip') {
    const [x, y] = P.row0Title || [200, 175];
    await page.mouse.move(x, y); await page.mouse.move(x + 3, y, { steps: 3 }); await page.waitForTimeout(1500);
    await shot('50_tooltip');
    await page.mouse.move(W / 2, 20);
  }
  if (step === 'popovers') {
    await click(...(P.filter || [1066, 30])); await shot('60_filter_open');
    await page.keyboard.type('1080'); await page.waitForTimeout(300); await shot('61_filter_typed_1080');
    await page.keyboard.press('Escape'); await shot('62_escape');
    await click(...(P.sort || [1153, 30])); await shot('63_sort_open');
    await click(W / 2, H - 10); await shot('64_outside_click');
  }
  if (step === 'grab') {
    const [gx, gy] = P.row0Grab || [1150, 175];
    await click(gx, gy); await page.waitForTimeout(800); await shot('70_grab_row0');
    const [cx, cy] = P.row1Check || [22, 211]; await click(cx, cy); await click(cx, cy + 36); await shot('71_two_checked');
    await click(...(P.grabSelected || [1135, 80])); await page.waitForTimeout(1200); await shot('72_grab_selected');
    log('grab POST bodies:', JSON.stringify(grabs));
  }
  if (step === 'resize') {
    await page.setViewportSize({ width: 800, height: 500 }); await page.waitForTimeout(600); await shot('80_resized_800x500');
    log('canvas after resize', JSON.stringify(await page.evaluate(() => [...document.querySelectorAll('canvas')].map(c => [c.width, c.height, c.clientWidth, c.clientHeight]))));
    await page.setViewportSize({ width: 1600, height: 800 }); await page.waitForTimeout(600); await shot('81_resized_1600x800');
    await page.setViewportSize({ width: W, height: H }); await page.waitForTimeout(400);
  }
  if (step === 'wheel') {
    const [x, y] = P.row0Title || [200, 175];
    await page.mouse.move(x, y + 100); for (let i = 0; i < 5; i++) { await page.mouse.wheel(0, 120); await page.waitForTimeout(50); }
    await page.waitForTimeout(300); await shot('90_wheel_scrolled');
  }
  if (step === 'wheelfine') { // small wheel steps so a row sits partly under the header; checks list clipping
    const [x, y] = P.row0Title || [200, 175];
    await page.mouse.move(x, y + 100);
    for (let i = 1; i <= 3; i++) { await page.mouse.wheel(0, 13 * i); await page.waitForTimeout(400); await shot(`9${i}_wheel_fine`); }
  }
  if (step === 'info') log('INFO2', JSON.stringify(await page.evaluate(() => ({ dpr: devicePixelRatio, inner: [innerWidth, innerHeight],
    canvas: [...document.querySelectorAll('canvas')].map(c => [c.width, c.height, c.clientWidth, c.clientHeight]), bench: (window.__anorakBench || []).slice(0, 3) }))));
  if (step === 'hold') await page.waitForTimeout(+(process.env.HOLD_MS || 3000));
}
await ctx.close(); fs.rmSync(profile, { recursive: true, force: true });
