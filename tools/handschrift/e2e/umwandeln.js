// Ende-zu-Ende-Test „Umwandeln“ (Handschrift-Erkennung) im Browser mit Playwright.
//
// Voraussetzungen: ./build-web.sh, Pakete aus tools/handschrift/pakete-bauen.py in <daten>/models/, Server läuft:
//   OWNDRAW_DATA=<daten> OWNDRAW_WEB=dist/web OWNDRAW_BIND=127.0.0.1:8099 owndraw-server
// Aufruf: node tools/handschrift/e2e/umwandeln.js [Ausgabeordner für Screenshots]
//   OWNDRAW_URL (Standard http://127.0.0.1:8099), PLAYWRIGHT (Pfad zum playwright-Modul, Standard „playwright“).
//
// Ablauf: neues Board, „F = 2 kN“, „Haus“ und „12,5 kN“ schreiben, Werkzeug „Umwandeln“, „F“ antippen,
// „Modelle laden“, Erkennung abwarten (Vorauswahl Formel), Enter ersetzt; „12,5 kN“ mit Lasso (Zahl), „Haus“
// antippen (Text). Prüft die besten Kandidaten über owndrawHw.protokoll() und legt Screenshots ab.
// Die Klick-Koordinaten passen zum Fenster 1280×800 (Gerätepixel 2) und zur Standard-Leiste.
const { chromium } = require(process.env.PLAYWRIGHT || 'playwright');
const fs = require('fs');
const os = require('os');
const path = require('path');

const BASE = process.env.OWNDRAW_URL || 'http://127.0.0.1:8099';
const OUT = process.argv[2] || fs.mkdtempSync(path.join(os.tmpdir(), 'owndraw-hw-'));

// ---- einfache Druckschrift: Striche in einer Einheitsbox (Höhe 1)
const bogen = (cx, cy, rx, ry, a0, a1, n = 14) => Array.from({ length: n + 1 }, (_, i) => {
  const a = ((a0 + (a1 - a0) * i / n) * Math.PI) / 180;
  return [cx + rx * Math.cos(a), cy + ry * Math.sin(a)];
});
const G = {
  F: [0.6, [[[0, 0], [0, 1]], [[0, 0], [0.6, 0]], [[0, 0.5], [0.45, 0.5]]]],
  '=': [0.6, [[[0, 0.42], [0.6, 0.42]], [[0, 0.68], [0.6, 0.68]]]],
  2: [0.6, [[[0.02, 0.22], [0.12, 0.05], [0.3, 0], [0.48, 0.05], [0.58, 0.22], [0.52, 0.42], [0, 1], [0.62, 1]]]],
  1: [0.35, [[[0, 0.2], [0.3, 0], [0.3, 1]]]],
  5: [0.55, [[[0.55, 0], [0.05, 0], [0, 0.45], [0.3, 0.4], [0.52, 0.55], [0.55, 0.78], [0.4, 0.97], [0.15, 1], [0, 0.9]]]],
  ',': [0.15, [[[0.1, 0.92], [0.1, 1.0], [0.0, 1.15]]]],
  k: [0.5, [[[0, 0], [0, 1]], [[0.48, 0.42], [0.02, 0.75]], [[0.15, 0.65], [0.5, 1]]]],
  N: [0.65, [[[0, 1], [0.01, 0]], [[0, 0.02], [0.65, 1]], [[0.64, 1], [0.65, 0]]]],
  H: [0.6, [[[0, 0], [0, 1]], [[0.6, 0], [0.6, 1]], [[0, 0.5], [0.6, 0.5]]]],
  a: [0.5, [bogen(0.25, 0.72, 0.25, 0.28, -20, 340), [[0.5, 0.44], [0.5, 1]]]],
  u: [0.5, [[[0, 0.45], [0.02, 0.85], [0.15, 1], [0.35, 1], [0.5, 0.85]], [[0.5, 0.45], [0.5, 1]]]],
  s: [0.45, [[[0.45, 0.5], [0.3, 0.44], [0.1, 0.46], [0.02, 0.58], [0.15, 0.7], [0.35, 0.76], [0.45, 0.88], [0.35, 0.99], [0.12, 1], [0, 0.94]]]],
  ' ': [0.35, []],
};
// Enden leicht kürzen: ein Strich, der auf einem anderen beginnt, würde sonst mit ihm verbunden
function kuerzen(s, d = 0.07) {
  s = s.slice();
  const zu = (p, q) => {
    const l = Math.hypot(q[0] - p[0], q[1] - p[1]);
    return l < 3 * d ? p : [p[0] + ((q[0] - p[0]) * d) / l, p[1] + ((q[1] - p[1]) * d) / l];
  };
  s[0] = zu(s[0], s[1]);
  s[s.length - 1] = zu(s[s.length - 1], s[s.length - 2]);
  return s;
}
function zeile(text, x, y, h) {
  const out = [];
  for (const ch of text) {
    const [w, strokes] = G[ch];
    for (const s of strokes) out.push(kuerzen(s).map(([px, py]) => [x + px * h, y + py * h]));
    x += (w + 0.28) * h;
  }
  return out;
}

(async () => {
  const profil = fs.mkdtempSync(path.join(os.tmpdir(), 'owndraw-hw-profil-'));
  // --force-device-scale-factor statt nur deviceScaleFactor: sonst meldet ResizeObserver die CSS-Größe
  const ctx = await chromium.launchPersistentContext(profil, {
    headless: true, viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2,
    args: ['--no-proxy-server', '--force-device-scale-factor=2'],
  });
  const page = ctx.pages()[0] || (await ctx.newPage());
  page.on('pageerror', (e) => console.log('[pageerror]', e.message));
  const shot = (n) => page.screenshot({ path: path.join(OUT, n + '.png') });
  const click = async (x, y) => {
    await page.mouse.move(x, y); await page.waitForTimeout(80);
    await page.mouse.down(); await page.waitForTimeout(80); await page.mouse.up(); await page.waitForTimeout(400);
  };
  const strich = async (pts) => {
    await page.mouse.move(pts[0][0], pts[0][1]); await page.waitForTimeout(30);
    await page.mouse.down(); await page.waitForTimeout(30);
    for (let i = 1; i < pts.length; i++) {
      const [x0, y0] = pts[i - 1], [x1, y1] = pts[i];
      const n = Math.max(1, Math.round(Math.hypot(x1 - x0, y1 - y0) / 4));
      for (let k = 1; k <= n; k++) { await page.mouse.move(x0 + ((x1 - x0) * k) / n, y0 + ((y1 - y0) * k) / n); await page.waitForTimeout(4); }
    }
    await page.waitForTimeout(30); await page.mouse.up(); await page.waitForTimeout(80);
  };
  const protokoll = () => page.evaluate(() => JSON.parse(window.owndrawHw.protokoll()));
  const warte = (n, ms = 300000) => page.waitForFunction((n) => JSON.parse(window.owndrawHw.protokoll()).length >= n, n, { timeout: ms, polling: 250 });
  const pruefe = (ok, was) => { console.log((ok ? 'OK   ' : 'FEHL ') + was); if (!ok) process.exitCode = 1; };

  await page.goto(BASE + '/');
  await page.evaluate(() => localStorage.setItem('owndraw:/owndraw/config/settings.json', JSON.stringify({ tool_on: { convert: true } })));
  await page.goto(BASE + '/');
  await page.waitForTimeout(4000);
  await click(579, 470); // Neues Board
  await click(535, 262); // Unendlich
  await click(806, 558); // Erstellen
  await page.waitForTimeout(2000);
  for (const [t, x, y] of [['F = 2 kN', 300, 200], ['Haus', 300, 330], ['12,5 kN', 300, 450]]) for (const s of zeile(t, x, y, 42)) await strich(s);
  await click(33, 384); // Werkzeug „Umwandeln“
  await shot('01-werkzeug');
  await click(300, 221); // „F“ antippen: ganze Zeile
  await shot('02-modelle-fehlen');
  const t0 = Date.now();
  await page.waitForTimeout(1500);
  const da = () => page.evaluate(() => JSON.parse(window.owndrawHw.info()).pakete.filter((p) => p.zustand === 'da').length);
  if ((await da()) < 4) {
    await click(401, 412); // Modelle laden (fehlt, wenn das Profil sie schon im Cache hat)
    await page.waitForFunction(() => JSON.parse(window.owndrawHw.info()).pakete.filter((p) => p.zustand === 'da').length >= 4, null, { timeout: 300000 });
    console.log('Modelle geladen nach', Date.now() - t0, 'ms');
  } else console.log('Modelle schon im Browser-Cache');
  await warte(3);
  await page.waitForTimeout(1500);
  await shot('03-formel');
  let p = await protokoll();
  const by = (paket) => p.filter((x) => x.paket.startsWith(paket)).pop();
  pruefe(by('zahl').beste[0] === 'F = 2 kN', 'Zahl liest „F = 2 kN“: ' + by('zahl').beste[0]);
  pruefe(/k\s*N/.test(by('formel').beste[0]), 'Formel enthält kN: ' + by('formel').beste[0]);
  for (const x of p) console.log(`  ${x.paket}: ${x.ms} ms (Sitzung ${x.laden_ms} ms, ${x.ep}) → ${x.beste.join(' | ')}`);
  await page.keyboard.press('Enter'); // Ersetzen (Formel ist vorausgewählt)
  await page.waitForTimeout(1200);
  await shot('04-ersetzt');
  await strich([[285, 432], [505, 432], [505, 508], [285, 508], [285, 436]]); // Lasso um „12,5 kN“
  await warte(6);
  await page.waitForTimeout(1500);
  await shot('05-zahl');
  p = await protokoll();
  pruefe(p.slice(3, 6).some((x) => x.paket.startsWith('zahl') && x.beste.slice(0, 2).includes('12,5 kN')), 'Lasso: Zahl „12,5 kN“ unter den ersten zwei');
  await page.keyboard.press('Escape');
  await page.waitForTimeout(800);
  await click(300, 345); // „Haus“ antippen
  await warte(9);
  await page.waitForTimeout(1500);
  await shot('06-text');
  p = await protokoll();
  pruefe(p.slice(6, 9).some((x) => x.paket.startsWith('text') && x.beste[0] === 'Haus'), 'Text liest „Haus“');
  console.log('Screenshots:', OUT);
  await ctx.close();
  fs.rmSync(profil, { recursive: true, force: true });
})();
