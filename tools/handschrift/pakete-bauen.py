#!/usr/bin/env python3
"""Baut die Modell-Pakete der Handschrift-Erkennung (OwnDraw, Werkzeug „Umwandeln“).

Ein Paket ist ein Ordner `<id>/` mit `manifest.json` und den Dateien (ONNX-Modelle, Tokenizer, Lizenz),
dazu ein Archiv `<id>.tar` zum Hochladen im Admin-Bereich (Abschnitt „Erkennungsmodelle“). Der Server legt
die Pakete unter `data/models/<id>/` ab; die Web-App lädt sie von dort und erkennt nur lokal im Browser.

Pakete (nur MIT/Apache-2.0/AFL-3.0, kein MathWriting-/NC-Material):
  onnxruntime-web    Laufzeit (onnxruntime-web 1.31.1 aus npm, MIT)                      ~45 MB
  formel-p2t-int8    breezedeus/pix2text-mfr-1.5, int8 (MIT)       Formel, Stufe schlecht+mittel  ~32 MB
  text-trocr-de-int8 EpsilonGreedy/TrOCR_small_german_handwritten, int8 (AFL-3.0)  Text   ~63 MB
  zahl-ppocr-latin   PaddlePaddle/latin_PP-OCRv5_mobile_rec (Apache-2.0, paddle2onnx) Zahl  ~8 MB
  zahl-ppocr-v6-tiny PaddlePaddle/PP-OCRv6_tiny_rec_onnx (Apache-2.0), Ausweich für Zahl   ~5 MB
  formel-texteller-fp16 OleehyO/TexTeller fp16 (Apache-2.0)        Formel, Stufe hoch     ~620 MB

Voraussetzungen (in ein venv, nicht ins Repo):
  uv venv venv && venv/bin/pip install torch --index-url https://download.pytorch.org/whl/cpu
  venv/bin/pip install "transformers<4.50" "optimum[onnxruntime]<1.25" onnx onnxruntime onnxconverter-common \
      huggingface_hub pyyaml sentencepiece
  # nur für zahl-ppocr-latin: venv/bin/pip install paddlepaddle paddle2onnx
  # nur für onnxruntime-web: npm (lädt das npm-Paket in einen Zwischenordner)

Aufruf:
  venv/bin/python tools/handschrift/pakete-bauen.py --out /pfad/handschrift-modelle [--nur id,id] [--tmp /tmp/x]
  Danach: Admin → Erkennungsmodelle → „Paket hochladen“ (<id>.tar) oder den Ordner nach data/models/ kopieren.
"""
import argparse, hashlib, json, os, shutil, subprocess, sys, tarfile, urllib.request

ORT_VERSION = '1.31.1'
SPDX = 'https://raw.githubusercontent.com/spdx/license-list-data/main/text/{}.txt'

# Revisionen fest (Herkunft nachvollziehbar, gleiche Dateien bei jedem Bau)
REV = {
    'breezedeus/pix2text-mfr-1.5': '1cef9f0bdcd6a4c63df7de1311fb0894593340cc',
    'EpsilonGreedy/TrOCR_small_german_handwritten': '8a47330344ecaa97da08880a5223cfcb6ebdd844',
    'OleehyO/TexTeller': '7b96df06b9d81cdb129c3bef68b7250bc3e2b0ea',
    'PaddlePaddle/latin_PP-OCRv5_mobile_rec': 'ab2cd5cc5fa6309be2e5acdfe66eca2c2c127d57',
    'PaddlePaddle/PP-OCRv6_tiny_rec_onnx': '2612ab37152ae0a677521bae4e1e3d4fb4cf7c30',
}

# Einheiten, die als eigene Zeichenkette sinnvoll sind (Kontext-Lexikon des Zahl-Modells)
EINHEITEN = ['mm', 'cm', 'dm', 'm', 'km', 'mm²', 'cm²', 'm²', 'mm³', 'cm³', 'm³', 'l', 'ml', 'g', 'kg', 't', 's', 'ms',
             'min', 'h', 'N', 'kN', 'MN', 'Nm', 'N·m', 'kN·m', 'Pa', 'kPa', 'MPa', 'N/mm²', 'bar', 'J', 'kJ', 'W', 'kW',
             'MW', 'Wh', 'kWh', 'V', 'mV', 'kV', 'A', 'mA', 'Ω', 'kΩ', 'F', 'µF', 'Hz', 'kHz', '°', '°C', 'K', '%',
             'm/s', 'km/h', 'm/s²', 'rad', 'U/min', '1/min']


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            h.update(b)
    return h.hexdigest()


def hf(repo, name, tmp):
    from huggingface_hub import hf_hub_download
    return hf_hub_download(repo, name, revision=REV[repo], cache_dir=os.path.join(tmp, 'hf'))


def licence_text(spdx, tmp, holder=None):
    p = os.path.join(tmp, f'lic-{spdx}.txt')
    if not os.path.exists(p):
        urllib.request.urlretrieve(SPDX.format(spdx), p)
    t = open(p, encoding='utf-8').read()
    if spdx == 'MIT' and holder:
        t = t.replace('<year> <copyright holders>', holder).replace('Copyright (c) <year> <copyright holders>', f'Copyright (c) {holder}')
    return t


def quant_int8(src, dst):
    from onnxruntime.quantization import quantize_dynamic, QuantType
    quantize_dynamic(src, dst, weight_type=QuantType.QInt8)


def finish(out, mf, files, tmp, lic_spdx, lic_holder=None):
    """Schreibt Dateien, LICENSE.txt, manifest.json und <id>.tar."""
    d = os.path.join(out, mf['id'])
    os.makedirs(d, exist_ok=True)
    for name, src in files.items():
        if os.path.abspath(src) != os.path.abspath(os.path.join(d, name)):
            shutil.copyfile(src, os.path.join(d, name))
    with open(os.path.join(d, 'LICENSE.txt'), 'w', encoding='utf-8') as f:
        f.write(licence_text(lic_spdx, tmp, lic_holder))
    names = list(files) + ['LICENSE.txt']
    mf['dateien'] = [{'name': n, 'groesse': os.path.getsize(os.path.join(d, n)), 'sha256': sha256(os.path.join(d, n))} for n in names]
    mf['groesse'] = sum(x['groesse'] for x in mf['dateien'])
    mf['lizenz']['datei'] = 'LICENSE.txt'
    with open(os.path.join(d, 'manifest.json'), 'w', encoding='utf-8') as f:
        json.dump(mf, f, ensure_ascii=False, indent=1)
    with tarfile.open(os.path.join(out, mf['id'] + '.tar'), 'w') as t:
        for n in ['manifest.json'] + names:
            t.add(os.path.join(d, n), arcname=n)
    print(f"{mf['id']}: {mf['groesse'] / 1e6:.1f} MB", flush=True)


def base(id_, name, art, stufen, version, spdx, quelle, rev, hinweis=''):
    return {'format': 1, 'id': id_, 'name': name, 'art': art, 'stufen': stufen, 'version': version,
            'lizenz': {'spdx': spdx, 'hinweis': hinweis},
            'quelle': {'url': quelle, 'revision': rev}}


def ved(enc, dec, start, eos, pad, max_len, beams, webgpu=False):
    return {'typ': 'ved', 'encoder': enc, 'decoder': dec, 'start_id': start, 'eos_id': eos, 'pad_id': pad,
            'max_len': max_len, 'beams': beams, 'webgpu': webgpu}


# ---------------------------------------------------------------- Pakete

def p_ort(out, tmp):
    d = os.path.join(tmp, 'npm')
    os.makedirs(d, exist_ok=True)
    if not os.path.exists(os.path.join(d, 'node_modules/onnxruntime-web')):
        subprocess.run(['npm', 'install', '--prefix', d, f'onnxruntime-web@{ORT_VERSION}', '--no-save', '--no-audit', '--no-fund'], check=True)
    dist = os.path.join(d, 'node_modules/onnxruntime-web/dist')
    names = ['ort.wasm.min.mjs', 'ort-wasm-simd-threaded.mjs', 'ort-wasm-simd-threaded.wasm',
             'ort.webgpu.min.mjs', 'ort-wasm-simd-threaded.jsep.mjs', 'ort-wasm-simd-threaded.jsep.wasm']
    mf = base('onnxruntime-web', f'ONNX Runtime Web {ORT_VERSION}', 'laufzeit', ['schlecht', 'mittel', 'hoch'], ORT_VERSION, 'MIT',
              f'https://www.npmjs.com/package/onnxruntime-web/v/{ORT_VERSION}', ORT_VERSION,
              'Laufzeit für die Erkennung im Browser (WebAssembly, optional WebGPU). Von Microsoft, MIT-Lizenz.')
    mf['laufzeit'] = {'wasm': 'ort.wasm.min.mjs', 'webgpu': 'ort.webgpu.min.mjs'}
    finish(out, mf, {n: os.path.join(dist, n) for n in names}, tmp, 'MIT', 'Microsoft Corporation')


def p_p2t(out, tmp):
    r = 'breezedeus/pix2text-mfr-1.5'
    w = os.path.join(tmp, 'p2t')
    os.makedirs(w, exist_ok=True)
    for part in ['encoder', 'decoder']:
        dst = os.path.join(w, f'{part}.onnx')
        if not os.path.exists(dst):
            quant_int8(hf(r, f'{part}_model.onnx', tmp), dst)
    mf = base('formel-p2t-int8', 'Pix2Text MFR 1.5 (int8)', 'formel', ['schlecht', 'mittel'], '1.5-int8-1', 'MIT',
              f'https://huggingface.co/{r}', REV[r], 'Formel → LaTeX, TrOCR-Architektur, selbst nach int8 quantisiert.')
    mf['modell'] = ved('encoder.onnx', 'decoder.onnx', 1, 2, 0, 160, 4)
    mf['vorverarbeitung'] = {'breite': 384, 'hoehe': 384, 'farbe': 'rgb', 'mittel': [0.5, 0.5, 0.5], 'std': [0.5, 0.5, 0.5],
                             'anpassen': 'einpassen', 'rand': 0.08}
    mf['tokenizer'] = {'datei': 'tokenizer.json', 'format': 'hf'}
    finish(out, mf, {'encoder.onnx': os.path.join(w, 'encoder.onnx'), 'decoder.onnx': os.path.join(w, 'decoder.onnx'),
                     'tokenizer.json': hf(r, 'tokenizer.json', tmp)}, tmp, 'MIT', 'breezedeus')


def p_trocr_de(out, tmp):
    r = 'EpsilonGreedy/TrOCR_small_german_handwritten'
    w = os.path.join(tmp, 'trocr-de')
    exp = os.path.join(w, 'export')
    if not os.path.exists(os.path.join(exp, 'decoder_model.onnx')):
        from optimum.exporters.onnx import main_export
        main_export(r, exp, task='image-to-text', revision=REV[r], monolith=False, no_post_process=True,
                    cache_dir=os.path.join(tmp, 'hf'))
    for part in ['encoder', 'decoder']:
        dst = os.path.join(w, f'{part}.onnx')
        if not os.path.exists(dst):
            quant_int8(os.path.join(exp, f'{part}_model.onnx'), dst)
    mf = base('text-trocr-de-int8', 'TrOCR small, deutsche Handschrift (int8)', 'text', ['schlecht', 'mittel', 'hoch'], '1-int8-1',
              'AFL-3.0', f'https://huggingface.co/{r}', REV[r],
              'Feinabgestimmt auf fhswf/german_handwriting (AFL-3.0); mit optimum nach ONNX exportiert und nach int8 quantisiert.')
    mf['modell'] = ved('encoder.onnx', 'decoder.onnx', 0, 2, 1, 64, 4)
    mf['vorverarbeitung'] = {'breite': 384, 'hoehe': 384, 'farbe': 'rgb', 'mittel': [0.5, 0.5, 0.5], 'std': [0.5, 0.5, 0.5],
                             'anpassen': 'strecken', 'rand': 0.04}
    mf['tokenizer'] = {'datei': 'tokenizer.json', 'format': 'hf'}
    finish(out, mf, {'encoder.onnx': os.path.join(w, 'encoder.onnx'), 'decoder.onnx': os.path.join(w, 'decoder.onnx'),
                     'tokenizer.json': hf(r, 'tokenizer.json', tmp)}, tmp, 'AFL-3.0')


def ctc_dict(yml, dst):
    import yaml
    chars = yaml.safe_load(open(yml, encoding='utf-8'))['PostProcess']['character_dict']
    with open(dst, 'w', encoding='utf-8') as f:
        for c in chars:
            f.write(str(c) + '\n')


def p_ppocr_latin(out, tmp):
    r = 'PaddlePaddle/latin_PP-OCRv5_mobile_rec'
    w = os.path.join(tmp, 'ppocr-latin')
    os.makedirs(w, exist_ok=True)
    onnx_p = os.path.join(w, 'rec.onnx')
    if not os.path.exists(onnx_p):
        md = os.path.dirname(hf(r, 'inference.json', tmp))
        hf(r, 'inference.pdiparams', tmp)
        subprocess.run([os.path.join(os.path.dirname(sys.executable), 'paddle2onnx'), '--model_dir', md, '--model_filename', 'inference.json',
                        '--params_filename', 'inference.pdiparams', '--save_file', onnx_p, '--opset_version', '14'], check=True)
    ctc_dict(hf(r, 'inference.yml', tmp), os.path.join(w, 'dict.txt'))
    mf = base('zahl-ppocr-latin', 'PP-OCRv5 mobile latin (Zahlen, Einheiten)', 'zahl', ['schlecht', 'mittel', 'hoch'], '5-1',
              'Apache-2.0', f'https://huggingface.co/{r}', REV[r], 'CTC-Zeilenerkenner, mit paddle2onnx nach ONNX umgewandelt.')
    mf['modell'] = {'typ': 'ctc', 'datei': 'rec.onnx', 'eingang': 'x', 'webgpu': False}
    mf['vorverarbeitung'] = {'hoehe': 48, 'farbe': 'bgr', 'mittel': [0.5, 0.5, 0.5], 'std': [0.5, 0.5, 0.5], 'anpassen': 'hoehe', 'rand': 0.1}
    mf['tokenizer'] = {'datei': 'dict.txt', 'format': 'ctc', 'leerzeichen': True}
    mf['lexikon'] = EINHEITEN
    finish(out, mf, {'rec.onnx': onnx_p, 'dict.txt': os.path.join(w, 'dict.txt')}, tmp, 'Apache-2.0')


def p_ppocr_v6(out, tmp):
    r = 'PaddlePaddle/PP-OCRv6_tiny_rec_onnx'
    w = os.path.join(tmp, 'ppocr-v6')
    os.makedirs(w, exist_ok=True)
    ctc_dict(hf(r, 'inference.yml', tmp), os.path.join(w, 'dict.txt'))
    mf = base('zahl-ppocr-v6-tiny', 'PP-OCRv6 tiny (Zahlen, Einheiten)', 'zahl', ['schlecht', 'mittel', 'hoch'], '6-1', 'Apache-2.0',
              f'https://huggingface.co/{r}', REV[r], 'CTC-Zeilenerkenner (Ausweich, wenn paddle2onnx fehlt).')
    mf['modell'] = {'typ': 'ctc', 'datei': 'rec.onnx', 'eingang': 'x', 'webgpu': False}
    mf['vorverarbeitung'] = {'hoehe': 48, 'farbe': 'bgr', 'mittel': [0.5, 0.5, 0.5], 'std': [0.5, 0.5, 0.5], 'anpassen': 'hoehe', 'rand': 0.1}
    mf['tokenizer'] = {'datei': 'dict.txt', 'format': 'ctc', 'leerzeichen': True}
    mf['lexikon'] = EINHEITEN
    finish(out, mf, {'rec.onnx': hf(r, 'inference.onnx', tmp), 'dict.txt': os.path.join(w, 'dict.txt')}, tmp, 'Apache-2.0')


def p_texteller(out, tmp):
    r = 'OleehyO/TexTeller'
    w = os.path.join(tmp, 'texteller')
    os.makedirs(w, exist_ok=True)
    import onnx
    from onnxconverter_common import float16
    for part in ['encoder', 'decoder']:
        dst = os.path.join(w, f'{part}.onnx')
        if not os.path.exists(dst):
            m = onnx.load(hf(r, f'{part}_model.onnx', tmp))
            m = float16.convert_float_to_float16(m, keep_io_types=True)
            onnx.save(m, dst)
    mf = base('formel-texteller-fp16', 'TexTeller (fp16)', 'formel', ['hoch'], '2-fp16-1', 'Apache-2.0', f'https://huggingface.co/{r}', REV[r],
              'Formel → LaTeX, groß (≈ 300 M Parameter); nach fp16 umgewandelt. Mit WebGPU deutlich schneller.')
    mf['modell'] = ved('encoder.onnx', 'decoder.onnx', 2, 2, 1, 200, 3, webgpu=True)
    mf['vorverarbeitung'] = {'breite': 448, 'hoehe': 448, 'farbe': 'grau', 'mittel': [0.9545467], 'std': [0.15394445], 'anpassen': 'oben-links', 'rand': 0.05}
    mf['tokenizer'] = {'datei': 'tokenizer.json', 'format': 'hf'}
    finish(out, mf, {'encoder.onnx': os.path.join(w, 'encoder.onnx'), 'decoder.onnx': os.path.join(w, 'decoder.onnx'),
                     'tokenizer.json': hf(r, 'tokenizer.json', tmp)}, tmp, 'Apache-2.0')


PAKETE = {'onnxruntime-web': p_ort, 'formel-p2t-int8': p_p2t, 'text-trocr-de-int8': p_trocr_de, 'zahl-ppocr-latin': p_ppocr_latin,
          'zahl-ppocr-v6-tiny': p_ppocr_v6, 'formel-texteller-fp16': p_texteller}

if __name__ == '__main__':
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--out', required=True)
    ap.add_argument('--tmp', default=os.path.join(os.environ.get('TMPDIR', '/tmp'), 'owndraw-hw-bau'))
    ap.add_argument('--nur', default='', help='Komma-Liste der Paket-IDs (Standard: alle)')
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    os.makedirs(a.tmp, exist_ok=True)
    ids = [x for x in a.nur.split(',') if x] or list(PAKETE)
    fehler = []
    for i in ids:
        try:
            PAKETE[i](a.out, a.tmp)
        except Exception as e:  # ein Paket darf scheitern (z. B. paddle2onnx fehlt), die anderen werden gebaut
            print(f'{i}: FEHLER {e}', file=sys.stderr, flush=True)
            fehler.append(i)
    if fehler:
        sys.exit(f'nicht gebaut: {", ".join(fehler)}')
