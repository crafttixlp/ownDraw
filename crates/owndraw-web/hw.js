// Handschrift-Erkennung im Browser (Werkzeug „Umwandeln“) mit onnxruntime-web.
//
// Die Erkennung läuft nur hier, lokal. Der OwnDraw-Server stellt die Modell-Pakete nur bereit
// (GET /api/models, /api/models/<id>/<datei>); auch onnxruntime-web selbst ist ein Paket (Art „laufzeit“),
// es kommt nie von einem CDN. Geladene Dateien liegen im Cache Storage des Browsers („owndraw-hw-v1“).
//
// Schnittstelle für die App (Rust, platform/web/hw.rs), alles synchron, Ergebnisse als JSON-Text:
//   owndrawHw.info()                     Zustand: Pakete, Download-Fortschritt, WebGPU
//   owndrawHw.liste()                    Paketliste vom Server holen (asynchron, Ergebnis in info())
//   owndrawHw.laden('["id", …]')         Pakete herunterladen und cachen (nur auf Klick der Nutzerin)
//   owndrawHw.loeschen()                 gespeicherte Modelle löschen
//   owndrawHw.erkennen(json, w, h, grau) Erkennung starten → Auftragsnummer; grau = Uint8Array w·h (255 = weiß)
//   owndrawHw.ergebnis(nr)               "" solange es läuft, sonst {"ok":true,"kandidaten":[{text,score}],"ms":…}
const CACHE = "owndraw-hw-v1";
const API = "api/models";

const S = {
  liste: "leer", // leer | laedt | ok | fehler
  fehler: "",
  pakete: [], // Manifeste vom Server
  zustand: {}, // id -> {zustand: fehlt|laedt|da|fehler, geladen, gesamt, fehler}
  jobs: new Map(),
  nr: 0,
  ort: null, // {ort, gpu}
  ortLaden: null,
  sitzungen: new Map(), // paket-id -> Promise<{enc, dec, tok, mf}>
  gpu: null, // null = unbekannt, true/false
  ep: "",
};
const mem = new Map(); // Ersatz, wenn es keinen Cache Storage gibt (unsicherer Kontext)

const url = (id, datei) => `${API}/${encodeURIComponent(id)}/${encodeURIComponent(datei)}`;
const schluessel = (id, d) => new URL(url(id, d.name) + "?sha256=" + d.sha256, location.href).href;
const cacheOk = () => typeof caches !== "undefined";

async function cacheGet(key) {
  if (!cacheOk()) return mem.get(key) || null;
  const c = await caches.open(CACHE);
  const r = await c.match(key);
  return r ? new Uint8Array(await r.arrayBuffer()) : null;
}
async function cacheHas(key) {
  if (!cacheOk()) return mem.has(key);
  const c = await caches.open(CACHE);
  return !!(await c.match(key));
}
async function cachePut(key, bytes) {
  if (!cacheOk()) return void mem.set(key, bytes);
  const c = await caches.open(CACHE);
  await c.put(key, new Response(bytes, { headers: { "content-type": "application/octet-stream" } }));
}

async function sha256hex(bytes) {
  if (!crypto?.subtle) return null; // nur in sicheren Kontexten
  const h = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(h), (b) => b.toString(16).padStart(2, "0")).join("");
}

async function pruefeCache(m) {
  let alle = true;
  for (const d of m.dateien) if (!(await cacheHas(schluessel(m.id, d)))) { alle = false; break; }
  const z = S.zustand[m.id];
  if (z && z.zustand === "laedt") return;
  S.zustand[m.id] = { zustand: alle ? "da" : "fehlt", geladen: alle ? gesamt(m) : 0, gesamt: gesamt(m), fehler: "" };
}
const gesamt = (m) => m.dateien.reduce((s, d) => s + (d.groesse || 0), 0);

async function liste() {
  if (S.liste === "laedt") return;
  S.liste = "laedt";
  try {
    const r = await fetch(API, { headers: { accept: "application/json" }, cache: "no-cache" });
    if (r.status === 404) throw new Error("Dieser Server stellt keine Erkennungsmodelle bereit.");
    if (!r.ok) throw new Error(`Server antwortet mit ${r.status}`);
    const j = await r.json();
    S.pakete = (j.pakete || []).filter((m) => m && m.id && Array.isArray(m.dateien));
    for (const m of S.pakete) await pruefeCache(m);
    S.liste = "ok";
    S.fehler = "";
  } catch (e) {
    S.liste = "fehler";
    S.fehler = String(e?.message || e);
  }
}

async function ladeDatei(m, d, z) {
  const key = schluessel(m.id, d);
  if (await cacheHas(key)) { z.geladen += d.groesse; return; }
  const r = await fetch(url(m.id, d.name), { cache: "no-store" });
  if (!r.ok) throw new Error(`${d.name}: Server antwortet mit ${r.status}`);
  const teile = [];
  let n = 0;
  const rd = r.body.getReader();
  for (;;) {
    const { done, value } = await rd.read();
    if (done) break;
    teile.push(value);
    n += value.length;
    z.geladen += value.length;
  }
  const buf = new Uint8Array(n);
  let o = 0;
  for (const t of teile) { buf.set(t, o); o += t.length; }
  if (d.groesse && n !== d.groesse) throw new Error(`${d.name}: Größe stimmt nicht (${n} statt ${d.groesse} Bytes)`);
  const h = await sha256hex(buf);
  if (h && h !== d.sha256) throw new Error(`${d.name}: Prüfsumme stimmt nicht`);
  await cachePut(key, buf);
}

async function laden(ids) {
  for (const id of ids) {
    const m = S.pakete.find((p) => p.id === id);
    if (!m) continue;
    const z = (S.zustand[id] = { zustand: "laedt", geladen: 0, gesamt: gesamt(m), fehler: "" });
    try {
      for (const d of m.dateien) await ladeDatei(m, d, z);
      z.zustand = "da";
      z.geladen = z.gesamt;
    } catch (e) {
      z.zustand = "fehler";
      z.fehler = String(e?.message || e);
    }
  }
}

async function loeschen() {
  S.sitzungen.clear();
  mem.clear();
  if (cacheOk()) await caches.delete(CACHE);
  for (const m of S.pakete) await pruefeCache(m);
}

// ---------------------------------------------------------------- Laufzeit

async function hatGpu() {
  if (S.gpu !== null) return S.gpu;
  try {
    const a = navigator.gpu ? await navigator.gpu.requestAdapter() : null;
    S.gpu = !!a && !(a.info && a.info.isFallbackAdapter) && !a.isFallbackAdapter;
  } catch { S.gpu = false; }
  return S.gpu;
}

async function blobUrl(m, name, typ) {
  const d = m.dateien.find((x) => x.name === name);
  if (!d) throw new Error(`Laufzeit: ${name} fehlt im Paket`);
  const b = await cacheGet(schluessel(m.id, d));
  if (!b) throw new Error("Laufzeit nicht geladen. Einstellungen → Handschrift-Erkennung → „Modelle laden“.");
  return URL.createObjectURL(new Blob([b], { type: typ }));
}

async function ortHolen(gpuWunsch) {
  if (S.ort) return S.ort;
  if (S.ortLaden) return S.ortLaden;
  S.ortLaden = (async () => {
    const m = S.pakete.find((p) => p.art === "laufzeit");
    if (!m) throw new Error("Auf dem Server fehlt das Paket onnxruntime-web.");
    const gpu = gpuWunsch && (await hatGpu()) && m.laufzeit?.webgpu && m.dateien.some((d) => d.name === m.laufzeit.webgpu);
    const haupt = gpu ? m.laufzeit.webgpu : m.laufzeit?.wasm || "ort.wasm.min.mjs";
    const glue = gpu ? "ort-wasm-simd-threaded.jsep.mjs" : "ort-wasm-simd-threaded.mjs";
    const wasm = gpu ? "ort-wasm-simd-threaded.jsep.wasm" : "ort-wasm-simd-threaded.wasm";
    const ort = await import(await blobUrl(m, haupt, "text/javascript"));
    ort.env.wasm.wasmPaths = { mjs: await blobUrl(m, glue, "text/javascript"), wasm: await blobUrl(m, wasm, "application/wasm") };
    ort.env.wasm.numThreads = self.crossOriginIsolated ? Math.min(4, navigator.hardwareConcurrency || 1) : 1;
    ort.env.logLevel = "error";
    S.ort = { ort, gpu };
    return S.ort;
  })();
  try { return await S.ortLaden; } finally { S.ortLaden = null; }
}

// ---------------------------------------------------------------- Tokenizer

function byteDecoder() {
  // Umkehrung von bytes_to_unicode (GPT-2/RoBERTa)
  const bs = [];
  for (let i = 33; i <= 126; i++) bs.push(i);
  for (let i = 161; i <= 172; i++) bs.push(i);
  for (let i = 174; i <= 255; i++) bs.push(i);
  const cs = bs.slice();
  let n = 0;
  for (let b = 0; b < 256; b++) if (!bs.includes(b)) { bs.push(b); cs.push(256 + n); n++; }
  const m = new Map();
  bs.forEach((b, i) => m.set(String.fromCharCode(cs[i]), b));
  return m;
}

function tokenizer(mf, bytes) {
  const t = mf.tokenizer || {};
  const txt = new TextDecoder().decode(bytes);
  if (t.format === "ctc" || t.format === "ppocr-yml") {
    let zeichen;
    if (t.format === "ctc") zeichen = txt.split("\n").map((l) => l.replace(/\r$/, ""));
    else {
      // inference.yml: Liste unter „character_dict:“ (- x, - 'x', - "x")
      zeichen = [];
      let drin = false;
      for (const l of txt.split("\n")) {
        if (/^\s*character_dict:\s*$/.test(l)) { drin = true; continue; }
        if (!drin) continue;
        const m = l.match(/^\s*-\s?(.*)$/);
        if (!m) { if (l.trim()) break; continue; }
        let v = m[1];
        if ((v.startsWith("'") && v.endsWith("'") && v.length >= 2) || (v.startsWith('"') && v.endsWith('"') && v.length >= 2)) v = v.slice(1, -1).replace(/''/g, "'");
        zeichen.push(v);
      }
    }
    while (zeichen.length && zeichen[zeichen.length - 1] === "") zeichen.pop();
    if (t.leerzeichen !== false) zeichen.push(" ");
    return { ctc: zeichen };
  }
  const j = JSON.parse(txt);
  const id2tok = [];
  const v = j.model?.vocab;
  if (Array.isArray(v)) v.forEach((p, i) => (id2tok[i] = Array.isArray(p) ? p[0] : p));
  else if (v) for (const [k, i] of Object.entries(v)) id2tok[i] = k;
  const spezial = new Set();
  const extra = new Map();
  for (const a of j.added_tokens || []) {
    if (a.special) spezial.add(a.id);
    else extra.set(a.id, a.content);
    id2tok[a.id] = a.content;
  }
  const dec = j.decoder || {};
  const typ = dec.type === "Sequence" ? (dec.decoders || []).map((x) => x.type).find((x) => x === "ByteLevel" || x === "Metaspace") : dec.type;
  const bd = typ === "ByteLevel" ? byteDecoder() : null;
  const repl = dec.replacement || "▁";
  const text = (ids) => {
    const tokens = ids.filter((i) => !spezial.has(i)).map((i) => [i, id2tok[i] ?? ""]);
    if (bd) {
      const out = [];
      for (const [i, s] of tokens) {
        if (extra.has(i)) { out.push(...new TextEncoder().encode(s)); continue; }
        for (const ch of s) { const b = bd.get(ch); if (b !== undefined) out.push(b); else out.push(...new TextEncoder().encode(ch)); }
      }
      return new TextDecoder().decode(new Uint8Array(out)).trim();
    }
    const s = tokens.map((x) => x[1]).join("");
    if (typ === "Metaspace") return s.split(repl).join(" ").trim();
    if (typ === "WordPiece") return tokens.map((x) => x[1]).join(" ").replace(/ ##/g, "").trim();
    return s.trim();
  };
  return { text };
}

// ---------------------------------------------------------------- Sitzungen

async function sitzung(id) {
  if (S.sitzungen.has(id)) return S.sitzungen.get(id);
  const p = (async () => {
    const mf = S.pakete.find((m) => m.id === id);
    if (!mf) throw new Error(`Paket ${id} ist auf dem Server nicht vorhanden.`);
    const datei = async (name) => {
      const d = mf.dateien.find((x) => x.name === name);
      if (!d) throw new Error(`${name} fehlt im Paket ${id}`);
      const b = await cacheGet(schluessel(id, d));
      if (!b) throw new Error("Modell nicht geladen. Einstellungen → Handschrift-Erkennung → „Modelle laden“.");
      return b;
    };
    const mod = mf.modell || {};
    const { ort, gpu } = await ortHolen(!!mod.webgpu);
    const eps = gpu && mod.webgpu ? ["webgpu", "wasm"] : ["wasm"];
    S.ep = eps[0];
    const opt = { executionProviders: eps, graphOptimizationLevel: "all" };
    const tok = tokenizer(mf, await datei(mf.tokenizer.datei));
    if (mod.typ === "ctc") {
      const enc = await ort.InferenceSession.create(await datei(mod.datei), opt);
      return { mf, ort, enc, tok };
    }
    const enc = await ort.InferenceSession.create(await datei(mod.encoder), opt);
    const dec = await ort.InferenceSession.create(await datei(mod.decoder), opt);
    return { mf, ort, enc, dec, tok };
  })();
  S.sitzungen.set(id, p);
  p.catch(() => S.sitzungen.delete(id));
  return p;
}

// ---------------------------------------------------------------- Vorverarbeitung

function leinwand(w, h) {
  if (typeof OffscreenCanvas !== "undefined") return new OffscreenCanvas(w, h);
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  return c;
}

function quelle(w, h, grau) {
  const c = leinwand(w, h);
  const g = c.getContext("2d");
  const im = g.createImageData(w, h);
  for (let i = 0; i < w * h; i++) {
    const v = grau[i];
    im.data[i * 4] = im.data[i * 4 + 1] = im.data[i * 4 + 2] = v;
    im.data[i * 4 + 3] = 255;
  }
  g.putImageData(im, 0, 0);
  return c;
}

function tensor(ort, vv, w, h, grau) {
  const src = quelle(w, h, grau);
  const art = vv.anpassen || "strecken";
  const rand = vv.rand || 0;
  let W, H;
  if (art === "hoehe") {
    H = vv.hoehe || 48;
    W = Math.max(16, Math.min(3200, Math.round((H * w) / h * (1 + 2 * rand))));
  } else {
    W = vv.breite || 384;
    H = vv.hoehe || 384;
  }
  const c = leinwand(W, H);
  const g = c.getContext("2d");
  g.fillStyle = "#fff";
  g.fillRect(0, 0, W, H);
  g.imageSmoothingEnabled = true;
  g.imageSmoothingQuality = "high";
  if (art === "oben-links") {
    const k = Math.min(W, H) * (1 - 2 * rand) / Math.max(w, h);
    g.drawImage(src, Math.round(W * rand), Math.round(H * rand), Math.round(w * k), Math.round(h * k));
  } else if (art === "einpassen") {
    const k = Math.min((W * (1 - 2 * rand)) / w, (H * (1 - 2 * rand)) / h);
    g.drawImage(src, Math.round((W - w * k) / 2), Math.round((H - h * k) / 2), Math.round(w * k), Math.round(h * k));
  } else if (art === "hoehe") {
    const k = H / h;
    g.drawImage(src, Math.round((W - w * k) / 2), 0, Math.round(w * k), H);
  } else {
    g.drawImage(src, Math.round(W * rand), Math.round(H * rand), Math.round(W * (1 - 2 * rand)), Math.round(H * (1 - 2 * rand)));
  }
  const d = g.getImageData(0, 0, W, H).data;
  const farbe = vv.farbe || "rgb";
  const mi = vv.mittel || [0.5, 0.5, 0.5];
  const sd = vv.std || [0.5, 0.5, 0.5];
  const C = farbe === "grau" ? 1 : 3;
  const a = new Float32Array(C * W * H);
  for (let i = 0; i < W * H; i++) {
    if (C === 1) {
      const v = (d[i * 4] * 0.299 + d[i * 4 + 1] * 0.587 + d[i * 4 + 2] * 0.114) / 255;
      a[i] = (v - mi[0]) / sd[0];
    } else {
      for (let k = 0; k < 3; k++) {
        const ch = farbe === "bgr" ? 2 - k : k;
        a[k * W * H + i] = (d[i * 4 + ch] / 255 - mi[k % mi.length]) / sd[k % sd.length];
      }
    }
  }
  return new ort.Tensor("float32", a, [1, C, H, W]);
}

// ---------------------------------------------------------------- Dekodieren

function logSoftmax(L, off, V) {
  let mx = -Infinity;
  for (let v = 0; v < V; v++) if (L[off + v] > mx) mx = L[off + v];
  let s = 0;
  for (let v = 0; v < V; v++) s += Math.exp(L[off + v] - mx);
  const ls = Math.log(s) + mx;
  return (v) => L[off + v] - ls;
}

function topk(L, off, V, k) {
  const best = [];
  for (let v = 0; v < V; v++) {
    const x = L[off + v];
    if (best.length < k) { best.push([v, x]); best.sort((a, b) => b[1] - a[1]); }
    else if (x > best[k - 1][1]) { best[k - 1] = [v, x]; best.sort((a, b) => b[1] - a[1]); }
  }
  return best.map((b) => b[0]);
}

async function ved(s, x, nbest) {
  const { ort, enc, dec, mf, tok } = s;
  const mod = mf.modell;
  const inName = enc.inputNames.includes("pixel_values") ? "pixel_values" : enc.inputNames[0];
  const eo = await enc.run({ [inName]: x });
  const hid = eo.last_hidden_state || eo[enc.outputNames[0]];
  const hd = await hid.getData();
  const [, T, D] = hid.dims;
  const beams = Math.max(1, Math.min(mod.beams || 4, nbest || 5));
  const reps = new Map();
  const hidFor = (B) => {
    if (!reps.has(B)) {
      const a = new Float32Array(B * T * D);
      for (let b = 0; b < B; b++) a.set(hd, b * T * D);
      reps.set(B, new ort.Tensor("float32", a, [B, T, D]));
    }
    return reps.get(B);
  };
  let alive = [{ ids: [mod.start_id], lp: 0 }];
  const fertig = [];
  const max = mod.max_len || 128;
  for (let step = 0; step < max && alive.length; step++) {
    const B = alive.length, L = alive[0].ids.length;
    const ids = new BigInt64Array(B * L);
    alive.forEach((b, i) => b.ids.forEach((t, j) => (ids[i * L + j] = BigInt(t))));
    const feeds = { input_ids: new ort.Tensor("int64", ids, [B, L]), encoder_hidden_states: hidFor(B) };
    if (dec.inputNames.includes("encoder_attention_mask")) feeds.encoder_attention_mask = new ort.Tensor("int64", new BigInt64Array(B * T).fill(1n), [B, T]);
    const out = await dec.run(feeds);
    const lg = out.logits || out[dec.outputNames[0]];
    const Ld = await lg.getData();
    const V = lg.dims[2];
    const neu = [];
    alive.forEach((b, i) => {
      const off = (i * L + L - 1) * V;
      const ls = logSoftmax(Ld, off, V);
      for (const t of topk(Ld, off, V, beams)) {
        let p = ls(t);
        // Endlosschleifen (gleiches Token immer wieder) bremsen
        const n = b.ids.length;
        if (n >= 4 && b.ids[n - 1] === t && b.ids[n - 2] === t && b.ids[n - 3] === t) p -= 2;
        neu.push({ ids: [...b.ids, t], lp: b.lp + p });
      }
    });
    neu.sort((a, b) => b.lp - a.lp);
    alive = [];
    for (const c of neu) {
      if (c.ids[c.ids.length - 1] === mod.eos_id) fertig.push(c);
      else if (alive.length < beams) alive.push(c);
      if (alive.length >= beams && fertig.length >= beams) break;
    }
    // vorzeitiges Ende: genug fertige und keine laufende kann sie noch schlagen (Länge normiert)
    if (fertig.length >= beams) {
      const worst = Math.min(...fertig.map((f) => f.lp / f.ids.length));
      if (alive.every((a) => a.lp / (a.ids.length + 1) < worst)) break;
    }
  }
  const alle = fertig.concat(alive).map((c) => ({ text: tok.text(c.ids.slice(1)), score: c.lp / Math.max(1, c.ids.length - 1) }));
  alle.sort((a, b) => b.score - a.score);
  const seen = new Set();
  return alle.filter((k) => (seen.has(k.text) ? false : (seen.add(k.text), true))).slice(0, nbest || 5);
}

async function ctc(s, x, nbest) {
  const { enc, tok } = s;
  const inName = s.mf.modell.eingang && enc.inputNames.includes(s.mf.modell.eingang) ? s.mf.modell.eingang : enc.inputNames[0];
  const o = await enc.run({ [inName]: x });
  const t = o[enc.outputNames[0]];
  const P = await t.getData();
  const [, T, V] = t.dims;
  // Wahrscheinlichkeiten? (PP-OCR liefert Softmax), sonst selbst normieren
  let sum0 = 0;
  for (let v = 0; v < V; v++) sum0 += P[v];
  const prob = Math.abs(sum0 - 1) < 0.05;
  const zeichen = tok.ctc;
  // Präfix-Beam-Search (Leerzeichen-Klasse 0 = blank), je Frame nur die 6 wahrscheinlichsten Klassen
  let beams = new Map([["", { pb: 0, pnb: -Infinity, last: -1 }]]);
  const lse = (a, b) => (a === -Infinity ? b : b === -Infinity ? a : Math.max(a, b) + Math.log1p(Math.exp(-Math.abs(a - b))));
  const W = 8;
  for (let f = 0; f < T; f++) {
    const off = f * V;
    const lp = prob ? (v) => Math.log(Math.max(P[off + v], 1e-12)) : logSoftmax(P, off, V);
    const cand = topk(P, off, V, 6);
    const nb = new Map();
    const get = (k, last) => nb.get(k) || (nb.set(k, { pb: -Infinity, pnb: -Infinity, last }), nb.get(k));
    for (const [pre, b] of beams) {
      const tot = lse(b.pb, b.pnb);
      for (const c of cand) {
        const p = lp(c);
        if (c === 0) { const e = get(pre, b.last); e.pb = lse(e.pb, tot + p); continue; }
        const ch = zeichen[c - 1] ?? "";
        const np = pre + "\u0000" + c;
        if (c === b.last) {
          const e1 = get(pre, b.last); e1.pnb = lse(e1.pnb, b.pnb + p); // wiederholt, ohne blank dazwischen
          const e2 = get(np, c); e2.pnb = lse(e2.pnb, b.pb + p);
        } else {
          const e = get(np, c); e.pnb = lse(e.pnb, tot + p);
        }
        void ch;
      }
    }
    beams = new Map([...nb.entries()].sort((a, b) => lse(b[1].pb, b[1].pnb) - lse(a[1].pb, a[1].pnb)).slice(0, W));
  }
  const out = [...beams.entries()].map(([pre, b]) => {
    const ids = pre.split("\u0000").filter(Boolean).map(Number);
    const text = ids.map((i) => zeichen[i - 1] ?? "").join("").trim();
    return { text, score: lse(b.pb, b.pnb) / Math.max(1, ids.length + 1) };
  });
  out.sort((a, b) => b.score - a.score);
  const seen = new Set();
  return out.filter((k) => k.text && (seen.has(k.text) ? false : (seen.add(k.text), true))).slice(0, nbest || 5);
}

async function erkennen(auftrag, w, h, grau) {
  const t0 = performance.now();
  const s = await sitzung(auftrag.paket);
  const t1 = performance.now();
  const x = tensor(s.ort, s.mf.vorverarbeitung || {}, w, h, grau);
  const k = s.mf.modell.typ === "ctc" ? await ctc(s, x, auftrag.nbest) : await ved(s, x, auftrag.nbest);
  return { ok: true, kandidaten: k, ms: Math.round(performance.now() - t1), laden_ms: Math.round(t1 - t0), ep: S.ep };
}

// ---------------------------------------------------------------- Schnittstelle

const fehlerText = (e) => String(e?.message || e).slice(0, 300);

window.owndrawHw = {
  info() {
    return JSON.stringify({
      verfuegbar: typeof WebAssembly === "object",
      cache: cacheOk(),
      webgpu: !!navigator.gpu,
      gpu: S.gpu,
      ep: S.ep,
      liste: S.liste,
      fehler: S.fehler,
      pakete: S.pakete.map((m) => ({ ...m, ...(S.zustand[m.id] || { zustand: "fehlt", geladen: 0, gesamt: gesamt(m) }) })),
    });
  },
  liste() { liste(); },
  laden(ids) { laden(JSON.parse(ids)); },
  loeschen() { loeschen(); },
  erkennen(json, w, h, grau) {
    const nr = ++S.nr;
    S.jobs.set(nr, null);
    erkennen(JSON.parse(json), w, h, grau)
      .then((r) => S.jobs.set(nr, r))
      .catch((e) => S.jobs.set(nr, { ok: false, fehler: fehlerText(e) }));
    return nr;
  },
  ergebnis(nr) {
    const r = S.jobs.get(nr);
    if (r === undefined) return JSON.stringify({ ok: false, fehler: "Unbekannter Auftrag" });
    if (r === null) return "";
    S.jobs.delete(nr);
    return JSON.stringify(r);
  },
};
