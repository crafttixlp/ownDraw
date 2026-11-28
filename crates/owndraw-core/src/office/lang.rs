//! Rechtschreibprüfung, Silbentrennung und Autokorrektur (Deutsch/Englisch), reines Rust.
//!
//! Daten (gzip, per `include_bytes!`): Häufigkeitslisten (FrequencyWords, CC-BY-SA-4.0) und
//! Trennmuster (hyph-utf8, MIT bzw. Kuiken-Lizenz); siehe `assets/lang/LICENSE-wortlisten.md`.
//! Alles wird beim ersten Aufruf entpackt (`OnceLock`).

use flate2::read::GzDecoder;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::{Mutex, OnceLock};

/// Sprache für Trennung und Wörterbuch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
}

// ---------------------------------------------------------------- Daten

const DE_GZ: &[u8] = include_bytes!("../../assets/lang/de.txt.gz");
const EN_GZ: &[u8] = include_bytes!("../../assets/lang/en.txt.gz");
const HYPH_DE_GZ: &[u8] = include_bytes!("../../assets/lang/hyph-de.pat.gz");
const HYPH_EN_GZ: &[u8] = include_bytes!("../../assets/lang/hyph-en.pat.gz");

fn gunzip(b: &[u8]) -> String {
    let mut s = String::new();
    let _ = GzDecoder::new(b).read_to_string(&mut s);
    s
}

/// Maske für den Vorfilter: Editierdistanz <= 2 bedeutet höchstens 4 verschiedene Buchstaben-Bits.
fn mask(w: &[char]) -> u64 {
    let mut m = 0u64;
    for &c in w {
        let b = if c.is_ascii_lowercase() { c as u32 - 'a' as u32 } else { 26 + (c as u32 % 38) };
        m |= 1 << b;
    }
    m
}

/// Ein Wörterbuch: Wörter in Häufigkeitsreihenfolge (Index = Rang).
struct Dict {
    text: &'static str,
    spans: Vec<(u32, u32)>,
    /// Indizes in `spans`, alphabetisch sortiert (Binärsuche).
    sorted: Vec<u32>,
    /// nach Zeichenlänge: (Rang, Maske)
    buckets: Vec<Vec<(u32, u64)>>,
}

const MAX_BUCKET: usize = 32;

impl Dict {
    fn load(gz: &[u8], blocked: &HashSet<&str>) -> Dict {
        let text: &'static str = Box::leak(gunzip(gz).into_boxed_str());
        let mut spans = Vec::new();
        let mut buckets: Vec<Vec<(u32, u64)>> = vec![Vec::new(); MAX_BUCKET + 1];
        let mut pos = 0usize;
        let mut chars: Vec<char> = Vec::new();
        for line in text.split('\n') {
            let (s, e) = (pos, pos + line.len());
            pos = e + 1;
            if line.is_empty() || blocked.contains(line) {
                continue;
            }
            chars.clear();
            chars.extend(line.chars());
            let rank = spans.len() as u32;
            spans.push((s as u32, e as u32));
            if chars.len() <= MAX_BUCKET {
                buckets[chars.len()].push((rank, mask(&chars)));
            }
        }
        let mut sorted: Vec<u32> = (0..spans.len() as u32).collect();
        sorted.sort_unstable_by(|&a, &b| {
            let (x, y) = (spans[a as usize], spans[b as usize]);
            text[x.0 as usize..x.1 as usize].cmp(&text[y.0 as usize..y.1 as usize])
        });
        Dict { text, spans, sorted, buckets }
    }

    fn word(&self, rank: u32) -> &'static str {
        let (s, e) = self.spans[rank as usize];
        &self.text[s as usize..e as usize]
    }

    fn contains(&self, w: &str) -> bool {
        self.sorted.binary_search_by(|&i| self.word(i).cmp(w)).is_ok()
    }
}

struct Dicts {
    de: Dict,
    en: Dict,
}

fn dicts() -> &'static Dicts {
    static D: OnceLock<Dicts> = OnceLock::new();
    D.get_or_init(|| {
        let blocked: HashSet<&str> = TYPOS.iter().map(|t| t.0).collect();
        Dicts { de: Dict::load(DE_GZ, &blocked), en: Dict::load(EN_GZ, &blocked) }
    })
}

// ---------------------------------------------------------------- Benutzerwörter

#[derive(Default)]
struct UserWords {
    list: Vec<String>,
    lower: HashSet<String>,
}

fn user() -> &'static Mutex<UserWords> {
    static U: OnceLock<Mutex<UserWords>> = OnceLock::new();
    U.get_or_init(|| Mutex::new(UserWords::default()))
}

/// Wort zum Benutzerwörterbuch hinzufügen (ohne Duplikate, Groß-/Kleinschreibung egal).
pub fn add_user_word(w: &str) {
    let w = w.trim();
    if w.is_empty() {
        return;
    }
    let mut u = user().lock().unwrap_or_else(|e| e.into_inner());
    if u.lower.insert(w.to_lowercase()) {
        u.list.push(w.to_string());
    }
}

/// Alle Benutzerwörter (Reihenfolge des Hinzufügens).
pub fn user_words() -> Vec<String> {
    user().lock().unwrap_or_else(|e| e.into_inner()).list.clone()
}

/// Benutzerwörter ersetzen (z. B. beim Laden der Einstellungen).
pub fn set_user_words(v: Vec<String>) {
    let mut u = user().lock().unwrap_or_else(|e| e.into_inner());
    u.list.clear();
    u.lower.clear();
    for w in v {
        let w = w.trim().to_string();
        if !w.is_empty() && u.lower.insert(w.to_lowercase()) {
            u.list.push(w);
        }
    }
}

fn is_user_word(lc: &str) -> bool {
    user().lock().unwrap_or_else(|e| e.into_inner()).lower.contains(lc)
}

// ---------------------------------------------------------------- Rechtschreibung

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Deutsches Kompositum: zwei bekannte Teile (je >= 3 Buchstaben), links optional mit Fuge.
fn is_compound(lc: &str) -> bool {
    let d = &dicts().de;
    let idx: Vec<usize> = lc.char_indices().map(|(i, _)| i).chain(std::iter::once(lc.len())).collect();
    let n = idx.len() - 1;
    if n < 6 {
        return false;
    }
    for k in 3..=n - 3 {
        let (l, r) = (&lc[..idx[k]], &lc[idx[k]..]);
        if !d.contains(r) {
            continue;
        }
        if d.contains(l) {
            return true;
        }
        for fuge in ["s", "es", "en", "n", "e"] {
            if let Some(stamm) = l.strip_suffix(fuge) {
                if stamm.chars().count() >= 3 && d.contains(stamm) {
                    return true;
                }
            }
        }
    }
    false
}

/// Ein Wort ohne Bindestrich.
fn check_single(p: &str) -> bool {
    if p.chars().count() <= 1 {
        return true;
    }
    let lc = lower(p);
    if is_user_word(&lc) {
        return true;
    }
    let d = dicts();
    if d.de.contains(&lc) || d.en.contains(&lc) {
        return true;
    }
    // Apostroph: "geht's", "it's", "Peter's": Teile einzeln, kurze Reste (s, ll, t ...) erlaubt
    if lc.contains('\'') {
        return lc
            .split('\'')
            .filter(|t| !t.is_empty())
            .all(|t| t.chars().count() <= 2 || d.de.contains(t) || d.en.contains(t));
    }
    is_compound(&lc)
}

/// Ist das Wort in Deutsch oder Englisch bekannt, ein Benutzerwort, eine Zahl, Abkürzung oder URL?
pub fn is_correct(word: &str) -> bool {
    let w = word.trim().replace('\u{2019}', "'");
    if w.contains('@') || w.contains("://") || w.starts_with("www.") {
        return true;
    }
    let w = w.trim_matches(|c: char| !c.is_alphanumeric());
    if w.is_empty() || w.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    let letters = w.chars().filter(|c| c.is_alphabetic());
    // Abkürzung in Großbuchstaben (USA, PDF, ÖPNV)
    if letters.clone().count() <= 6 && letters.clone().all(|c| c.is_uppercase()) && !w.contains('-') {
        return true;
    }
    // "z.B", "u.a": kurze Teile mit Punkten
    if w.contains('.') {
        return w.split('.').all(|t| t.chars().count() <= 3);
    }
    w.split('-').filter(|t| !t.is_empty()).all(check_single)
}

/// Zeichenbereiche (Start, Ende exklusiv; char-Indizes) falsch geschriebener Wörter.
pub fn misspelled(text: &str) -> Vec<(usize, usize)> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if c[i].is_whitespace() {
            i += 1;
            continue;
        }
        // Block bis zum nächsten Leerzeichen; URLs und E-Mail-Adressen überspringen
        let mut e = i;
        while e < c.len() && !c[e].is_whitespace() {
            e += 1;
        }
        let chunk: String = c[i..e].iter().collect();
        if !(chunk.contains('@') || chunk.contains("://") || chunk.starts_with("www.")) {
            let mut j = i;
            while j < e {
                if !c[j].is_alphanumeric() {
                    j += 1;
                    continue;
                }
                let mut k = j;
                while k < e {
                    let join = matches!(c[k], '\'' | '\u{2019}' | '-')
                        && k > j
                        && k + 1 < e
                        && c[k - 1].is_alphanumeric()
                        && c[k + 1].is_alphanumeric();
                    if c[k].is_alphanumeric() || join {
                        k += 1;
                    } else {
                        break;
                    }
                }
                let w: String = c[j..k].iter().collect();
                // Abkürzungen mit Punkt ("z.B.") nicht zerlegen
                let abk = k < e && c[k] == '.' && k + 1 < e && c[k + 1].is_alphabetic();
                if !abk && !is_correct(&w) {
                    out.push((j, k));
                }
                j = k.max(j + 1);
            }
        }
        i = e;
    }
    out
}

/// Editierdistanz (Damerau/OSA) bis 2; sonst 3.
fn osa(a: &[char], b: &[char]) -> u8 {
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > 2 || n > 40 || m > 40 {
        return 3;
    }
    let mut p2 = [0u8; 42];
    let mut p1 = [0u8; 42];
    let mut cur = [0u8; 42];
    for (j, v) in p1.iter_mut().enumerate().take(m + 1) {
        *v = j as u8;
    }
    for i in 1..=n {
        cur[0] = i as u8;
        let mut rmin = cur[0];
        for j in 1..=m {
            let cost = (a[i - 1] != b[j - 1]) as u8;
            let mut v = (p1[j] + 1).min(cur[j - 1] + 1).min(p1[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(p2[j - 2] + 1);
            }
            cur[j] = v;
            rmin = rmin.min(v);
        }
        if rmin > 2 {
            return 3;
        }
        p2 = p1;
        p1 = cur;
    }
    p1[m].min(3)
}

fn capitalize(s: &str) -> String {
    let mut it = s.chars();
    match it.next() {
        Some(f) => f.to_uppercase().chain(it).collect(),
        None => String::new(),
    }
}

/// Schreibweise von `orig` (alles groß / erster Buchstabe groß / klein) auf `s` übertragen.
fn apply_case(orig: &str, s: &str) -> String {
    let letters: Vec<char> = orig.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|c| c.is_uppercase()) {
        s.to_uppercase()
    } else if letters.first().is_some_and(|c| c.is_uppercase()) {
        capitalize(s)
    } else {
        s.to_string()
    }
}

/// Bis zu `n` Vorschläge (Editierdistanz <= 2, nach Distanz, dann Häufigkeit).
pub fn suggest(word: &str, n: usize) -> Vec<String> {
    let lc: Vec<char> = lower(word.trim()).chars().collect();
    if n == 0 || lc.is_empty() || lc.len() > 30 {
        return Vec::new();
    }
    let lcs: String = lc.iter().collect();
    let m = mask(&lc);
    let d = dicts();
    // (Distanz, Rang, Wort)
    let mut cand: Vec<(u8, u32, &str)> = Vec::new();
    let mut buf: Vec<char> = Vec::new();
    for dict in [&d.de, &d.en] {
        let lo = lc.len().saturating_sub(2).max(1);
        for len in lo..=(lc.len() + 2).min(MAX_BUCKET) {
            for &(rank, wm) in &dict.buckets[len] {
                if (wm ^ m).count_ones() > 4 {
                    continue;
                }
                let w = dict.word(rank);
                buf.clear();
                buf.extend(w.chars());
                let dist = osa(&lc, &buf);
                if dist <= 2 && w != lcs {
                    // Schlüssel: Distanz*2 + (Länge verschieden), damit Vertauscher vor Auslassungen kommen
                    let key = dist * 2 + (len != lc.len()) as u8;
                    cand.push((key, rank, w));
                }
            }
        }
    }
    // Benutzerwörter zuerst bei gleicher Distanz
    let uw: Vec<String> = user_words().iter().map(|w| lower(w)).collect();
    let mut ucand: Vec<(u8, u32, &str)> = Vec::new();
    for w in &uw {
        buf.clear();
        buf.extend(w.chars());
        let dist = osa(&lc, &buf);
        if dist <= 2 && *w != lcs {
            ucand.push((dist * 2, 0, w.as_str()));
        }
    }
    cand.sort_unstable();
    ucand.sort_unstable();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let (mut a, mut b) = (0, 0);
    while out.len() < n && (a < ucand.len() || b < cand.len()) {
        let take_user = b >= cand.len() || (a < ucand.len() && ucand[a].0 <= cand[b].0);
        let w = if take_user {
            a += 1;
            ucand[a - 1].2
        } else {
            b += 1;
            cand[b - 1].2
        };
        if seen.insert(w) {
            out.push(apply_case(word.trim(), w));
        }
    }
    out
}

// ---------------------------------------------------------------- Silbentrennung (Liang)

struct Patterns {
    map: HashMap<String, Box<[u8]>>,
    maxlen: usize,
    left: usize,
    right: usize,
}

fn load_patterns(gz: &[u8], left: usize, right: usize) -> Patterns {
    let mut map = HashMap::new();
    let mut maxlen = 0;
    for line in gunzip(gz).lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('%') {
            continue;
        }
        let (mut letters, mut vals, mut pending) = (String::new(), Vec::new(), 0u8);
        for ch in line.chars() {
            if let Some(dg) = ch.to_digit(10) {
                pending = dg as u8;
            } else {
                vals.push(pending);
                pending = 0;
                letters.push(ch);
            }
        }
        vals.push(pending);
        maxlen = maxlen.max(letters.chars().count());
        map.insert(letters, vals.into_boxed_slice());
    }
    Patterns { map, maxlen, left, right }
}

fn patterns(l: Lang) -> &'static Patterns {
    static DE: OnceLock<Patterns> = OnceLock::new();
    static EN: OnceLock<Patterns> = OnceLock::new();
    match l {
        Lang::De => DE.get_or_init(|| load_patterns(HYPH_DE_GZ, 2, 2)),
        Lang::En => EN.get_or_init(|| load_patterns(HYPH_EN_GZ, 2, 3)),
    }
}

/// Trennstellen eines reinen Buchstabenwortes (Index = Trennung vor diesem Zeichen).
fn liang(word: &[char], p: &Patterns) -> Vec<usize> {
    let n = word.len();
    if n < p.left + p.right {
        return Vec::new();
    }
    let mut w: Vec<char> = Vec::with_capacity(n + 2);
    w.push('.');
    w.extend(word.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)));
    w.push('.');
    let mut vals = vec![0u8; w.len() + 1];
    let mut key = String::new();
    for i in 0..w.len() {
        key.clear();
        for l in 1..=p.maxlen.min(w.len() - i) {
            key.push(w[i + l - 1]);
            if let Some(v) = p.map.get(key.as_str()) {
                for (k, &x) in v.iter().enumerate() {
                    if vals[i + k] < x {
                        vals[i + k] = x;
                    }
                }
            }
        }
    }
    // Trennung vor Wortzeichen c entspricht vals[c + 1]
    (p.left.max(1)..n).filter(|&c| n - c >= p.right && vals[c + 1] % 2 == 1).collect()
}

/// Sprache wählen: Umlaut/ß -> Deutsch; nur im englischen Wörterbuch bekannt -> Englisch; sonst Deutsch.
fn pick_lang(lc: &str) -> Lang {
    if lc.chars().any(|c| matches!(c, 'ä' | 'ö' | 'ü' | 'ß')) {
        return Lang::De;
    }
    let d = dicts();
    if d.en.contains(lc) && !d.de.contains(lc) {
        Lang::En
    } else {
        Lang::De
    }
}

/// Erlaubte Trennstellen als char-Index (Trennung vor diesem Zeichen).
pub fn hyphenate(word: &str) -> Vec<usize> {
    let c: Vec<char> = word.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if !c[i].is_alphabetic() {
            i += 1;
            continue;
        }
        let mut e = i;
        while e < c.len() && c[e].is_alphabetic() {
            e += 1;
        }
        let seg: String = c[i..e].iter().collect();
        let lang = pick_lang(&lower(&seg));
        out.extend(liang(&c[i..e], patterns(lang)).into_iter().map(|x| x + i));
        // Bindestrich selbst ist auch eine Trennstelle (danach)
        if e + 1 < c.len() && c[e] == '-' && c[e + 1].is_alphabetic() {
            out.push(e + 1);
        }
        i = e;
    }
    out
}

// ---------------------------------------------------------------- Autokorrektur

/// Ergebnis einer Autokorrektur.
///
/// Der Aufrufer löscht `remove` Zeichen vor der Schreibmarke und fügt `insert` ein. Ist
/// `consume_typed` wahr, wurde das getippte Zeichen bereits in die Korrektur eingearbeitet (typografische
/// Anführungszeichen) und darf NICHT zusätzlich eingefügt werden; sonst fügt der Aufrufer es danach ein.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Correction {
    pub remove: usize,
    pub insert: String,
    pub consume_typed: bool,
}

/// Häufige Tippfehler: (falsch kleingeschrieben, richtig). Beim Laden aus dem Wörterbuch ausgeblendet.
const TYPOS: &[(&str, &str)] = &[
    // Deutsch
    ("dsa", "das"), ("adss", "dass"), ("nciht", "nicht"), ("nihct", "nicht"), ("ihc", "ich"),
    ("udn", "und"), ("mcih", "mich"), ("dcih", "dich"), ("sihc", "sich"), ("ncoh", "noch"),
    ("abre", "aber"), ("wnen", "wenn"), ("eienr", "einer"), ("eien", "eine"), ("habne", "haben"),
    ("htate", "hatte"), ("wsa", "was"), ("sidn", "sind"), ("vielliecht", "vielleicht"),
    ("vileicht", "vielleicht"), ("villeicht", "vielleicht"), ("jezt", "jetzt"), ("jetz", "jetzt"),
    ("allso", "also"), ("alos", "also"), ("wiel", "weil"), ("wirklcih", "wirklich"),
    ("wiklich", "wirklich"), ("eigentlcih", "eigentlich"), ("natürlcih", "natürlich"),
    ("deshlab", "deshalb"), ("desshalb", "deshalb"), ("schno", "schon"), ("dnan", "dann"),
    ("dnake", "danke"), ("mti", "mit"), ("vno", "von"), ("zmu", "zum"), ("zru", "zur"),
    ("auhc", "auch"), ("acuh", "auch"), ("odre", "oder"), ("kien", "kein"), ("imemr", "immer"),
    ("immre", "immer"), ("imer", "immer"), ("heuet", "heute"),
    // Englisch
    ("teh", "the"), ("hte", "the"), ("adn", "and"), ("nad", "and"), ("taht", "that"),
    ("thta", "that"), ("waht", "what"), ("wiht", "with"), ("wtih", "with"), ("jsut", "just"),
    ("recieve", "receive"), ("recieved", "received"), ("seperate", "separate"),
    ("definately", "definitely"), ("occured", "occurred"), ("untill", "until"),
    ("thier", "their"), ("becuase", "because"), ("beacuse", "because"), ("alot", "a lot"),
    ("dont", "don't"), ("didnt", "didn't"), ("doesnt", "doesn't"), ("isnt", "isn't"),
    ("wasnt", "wasn't"), ("couldnt", "couldn't"), ("wouldnt", "wouldn't"),
    ("shouldnt", "shouldn't"), ("freind", "friend"), ("beleive", "believe"), ("wierd", "weird"),
    ("tommorow", "tomorrow"), ("goverment", "government"), ("neccessary", "necessary"),
    ("begining", "beginning"), ("yuo", "you"), ("yoru", "your"), ("peopel", "people"),
    ("poeple", "people"),
];

fn typo_fix(lc: &str) -> Option<&'static str> {
    TYPOS.iter().find(|t| t.0 == lc).map(|t| t.1)
}

/// Kürzel, nach deren Punkt kein neuer Satz beginnt.
const ABKUERZUNGEN: &[&str] = &[
    "bzw", "usw", "ca", "vgl", "nr", "etc", "evtl", "ggf", "ggfs", "inkl", "max", "min", "str", "dr",
    "prof", "abb", "tel", "bzgl", "sog", "bspw", "mr", "mrs", "ms", "st", "vs", "mio", "mrd", "hrsg",
    "engl", "dt", "frz", "bd", "kap", "abs", "art", "vol", "no", "fig", "sen", "jun", "fa", "hr", "fr",
];

const OEFFNER: &str = "\"„‚“‘'([{«»¡¿";
const SCHLIESSER: &str = "\"“”‘’')]}«»";

fn is_boundary(c: char) -> bool {
    c.is_whitespace() || matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '…')
}

/// Beginnt an Position `s` (Wortanfang) ein neuer Satz?
fn satzanfang(c: &[char], s: usize) -> bool {
    let mut j = s;
    while j > 0 && OEFFNER.contains(c[j - 1]) {
        j -= 1;
    }
    if j == 0 {
        return true;
    }
    let ws_ende = j;
    while j > 0 && c[j - 1].is_whitespace() {
        j -= 1;
    }
    if j == ws_ende {
        return false;
    }
    if j == 0 {
        return true;
    }
    while j > 0 && SCHLIESSER.contains(c[j - 1]) {
        j -= 1;
    }
    if j == 0 || !matches!(c[j - 1], '.' | '!' | '?' | '…') {
        return false;
    }
    if c[j - 1] == '.' {
        // Wort vor dem Punkt: Ordnungszahl, "z.B.", Kürzel, Einzelbuchstabe -> kein Satzende
        let mut k = j - 1;
        while k > 0 && !c[k - 1].is_whitespace() {
            k -= 1;
        }
        let tok: String = c[k..j - 1].iter().collect::<String>().to_lowercase();
        let tok = tok.trim_start_matches(|ch: char| OEFFNER.contains(ch));
        if tok.is_empty()
            || tok.ends_with(|ch: char| ch.is_ascii_digit())
            || tok.contains('.')
            || tok.chars().count() <= 1
            || ABKUERZUNGEN.contains(&tok)
        {
            return false;
        }
    }
    true
}

/// Zwei Großbuchstaben am Wortanfang ("HAllo" -> "Hallo").
fn zwei_grossbuchstaben(w: &str) -> Option<String> {
    let c: Vec<char> = w.chars().collect();
    if c.len() >= 4 && c[0].is_uppercase() && c[1].is_uppercase() && c[2..].iter().all(|x| x.is_lowercase()) {
        let rest: String = c[1..].iter().collect();
        return Some(format!("{}{}", c[0], rest.to_lowercase()));
    }
    None
}

/// Autokorrektur beim Tippen. `before` = Absatztext bis zur Schreibmarke (ohne `typed`).
///
/// - `"` und `'`: typografische Anführungszeichen („…“, ‚…‘), `consume_typed = true`.
/// - Bei Wortgrenzen (Leerzeichen, Satzzeichen): Symbole (`--`, `(c)`, `(r)`, `(tm)`, `->`, `<-`, `=>`,
///   `...`, `1/2` ...), Tippfehlerliste, "HAllo" -> "Hallo", Satzanfang groß. Hier ist
///   `consume_typed = false`: der Aufrufer fügt `typed` nach dem Ersatz ein.
pub fn autocorrect(before: &str, typed: char) -> Option<Correction> {
    let c: Vec<char> = before.chars().collect();
    // Anführungszeichen
    if typed == '"' || typed == '\'' {
        let oeffnend = c.last().is_none_or(|&p| p.is_whitespace() || "([{–—„‚«/".contains(p));
        let z = if typed == '"' {
            if oeffnend { '„' } else { '“' }
        } else if oeffnend {
            '‚'
        } else {
            let offen = c.iter().filter(|&&x| x == '‚').count() > c.iter().filter(|&&x| x == '‘').count();
            if offen { '‘' } else { '’' }
        };
        // Apostroph mitten im Wort ("geht's") nicht ersetzen, wenn kein ‚ offen ist
        return Some(Correction { remove: 0, insert: z.to_string(), consume_typed: true });
    }
    if !is_boundary(typed) {
        return None;
    }
    // Block ab dem letzten Leerzeichen
    let ts = c.iter().rposition(|x| x.is_whitespace()).map_or(0, |p| p + 1);
    let tok: String = c[ts..].iter().collect();
    let tl = tok.to_lowercase();
    let tn = tok.chars().count();
    // Symbole am Blockende
    if !(typed == '.' && tok.ends_with('.')) {
        const SYM: &[(&str, &str)] = &[
            ("(c)", "©"), ("(r)", "®"), ("(tm)", "™"), ("->", "→"), ("<-", "←"), ("=>", "⇒"), ("...", "…"),
        ];
        for &(pat, rep) in SYM {
            if tl.ends_with(pat) && !(pat == "..." && tl.ends_with("....")) {
                return Some(Correction { remove: pat.chars().count(), insert: rep.to_string(), consume_typed: false });
            }
        }
        // "--" im Block (nicht "---")
        let tc: Vec<char> = tok.chars().collect();
        if let Some(p) = (0..tc.len().saturating_sub(1)).rev().find(|&i| tc[i] == '-' && tc[i + 1] == '-') {
            let davor = p > 0 && tc[p - 1] == '-';
            let danach = p + 2 < tc.len() && (tc[p + 2] == '-' || tc[p + 2] == '>');
            if !davor && !danach {
                let rest: String = tc[p + 2..].iter().collect();
                return Some(Correction {
                    remove: tn - p,
                    insert: format!("–{rest}"),
                    consume_typed: false,
                });
            }
        }
    }
    // Brüche
    let bruch = match tok.as_str() {
        "1/2" => Some("½"),
        "1/4" => Some("¼"),
        "3/4" => Some("¾"),
        _ => None,
    };
    if let Some(b) = bruch {
        return Some(Correction { remove: 3, insert: b.to_string(), consume_typed: false });
    }
    // Wort am Ende
    let mut s = c.len();
    while s > 0 && c[s - 1].is_alphabetic() {
        s -= 1;
    }
    if s == c.len() {
        return None;
    }
    if s > 0 {
        let p = c[s - 1];
        if p.is_alphanumeric() || matches!(p, '-' | '/' | '@' | '.' | '_' | '\'' | '\u{2019}' | ':' | '#' | '&') {
            return None;
        }
    }
    let orig: String = c[s..].iter().collect();
    let mut w = match typo_fix(&lower(&orig)) {
        Some(r) => apply_case(&orig, r),
        None => orig.clone(),
    };
    if let Some(f) = zwei_grossbuchstaben(&w) {
        w = f;
    }
    let first = w.chars().next()?;
    if first.is_lowercase() && w.chars().count() >= 2 && satzanfang(&c, s) {
        w = capitalize(&w);
    }
    (w != orig).then(|| Correction { remove: c.len() - s, insert: w, consume_typed: false })
}

// ---------------------------------------------------------------- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rechtschreibung() {
        for w in ["Haus", "Häuser", "Fahrrad", "Hausboot", "the", "Hallo,", "E-Mail", "USA", "2024", "geht's"] {
            assert!(is_correct(w), "{w} sollte richtig sein");
        }
        for w in ["Huas", "teh", "Fahrrdd", "Xqzvbn"] {
            assert!(!is_correct(w), "{w} sollte falsch sein");
        }
        assert!(suggest("Huas", 5).contains(&"Haus".to_string()), "{:?}", suggest("Huas", 5));
        assert!(suggest("teh", 5).contains(&"the".to_string()));
        assert_eq!(suggest("Huas", 1), vec!["Haus".to_string()]);
    }

    #[test]
    fn benutzerwort_und_bereiche() {
        set_user_words(vec![]);
        assert!(!is_correct("Zorkblatt"));
        add_user_word("Zorkblatt");
        add_user_word("zorkblatt");
        assert!(is_correct("Zorkblatt") && is_correct("zorkblatt"));
        assert_eq!(user_words(), vec!["Zorkblatt".to_string()]);
        // Zeichen-, nicht Byte-Indizes (ä = 1 Zeichen)
        let r = misspelled("Größe Huas www.beispiel.de x@y.de gut");
        assert_eq!(r, vec![(6, 10)], "{r:?}");
        set_user_words(vec![]);
    }

    #[test]
    fn silbentrennung() {
        assert_eq!(hyphenate("Silbentrennung"), vec![3, 6, 10]);
        assert!(hyphenate("Haus").is_empty() || hyphenate("Haus").iter().all(|&i| (2..=2).contains(&i)));
        assert!(!hyphenate("international").is_empty());
        // Bindestrich: Trennung danach
        assert!(hyphenate("Wort-Teil").contains(&5));
    }

    #[test]
    fn autokorrektur() {
        let k = |r: usize, i: &str| Some(Correction { remove: r, insert: i.to_string(), consume_typed: false });
        assert_eq!(autocorrect("a --", ' '), k(2, "–"));
        assert_eq!(autocorrect("Copyright (c)", ' '), k(3, "©"));
        assert_eq!(autocorrect("(R)", ' '), k(3, "®"));
        assert_eq!(autocorrect("Name(tm)", '.'), k(4, "™"));
        assert_eq!(autocorrect("a ->", ' '), k(2, "→"));
        assert_eq!(autocorrect("a <-", ' '), k(2, "←"));
        assert_eq!(autocorrect("a =>", ' '), k(2, "⇒"));
        assert_eq!(autocorrect("Hm...", ' '), k(3, "…"));
        assert_eq!(autocorrect("1/2", ' '), k(3, "½"));
        assert_eq!(autocorrect("Ich mag dsa", ' '), k(3, "das"));
        assert_eq!(autocorrect("dsa", ' '), k(3, "Das"));
        assert_eq!(autocorrect("Nciht", ' '), k(5, "Nicht"));
        assert_eq!(autocorrect("Ist es teh", ' '), k(3, "the"));
        assert_eq!(autocorrect("HAllo", ' '), k(5, "Hallo"));
        assert_eq!(autocorrect("Das ist gut. neuer", ' '), k(5, "Neuer"));
        assert_eq!(autocorrect("hallo", ' '), k(5, "Hallo"));
        assert_eq!(autocorrect("Das ist gut", ' '), None);
        assert_eq!(autocorrect("z.B. neuer", ' '), None);
        assert_eq!(autocorrect("am 1. mai", ' '), None);
        assert_eq!(autocorrect("ÖPNV", ' '), None);
        let q = |i: &str| Some(Correction { remove: 0, insert: i.to_string(), consume_typed: true });
        assert_eq!(autocorrect("", '"'), q("„"));
        assert_eq!(autocorrect("Er sagt \u{201e}Hallo", '"'), q("“"));
        assert_eq!(autocorrect("Er sagt ", '\''), q("‚"));
        assert_eq!(autocorrect("Er sagt ‚Hallo", '\''), q("‘"));
        assert_eq!(autocorrect("geht", '\''), q("’"));
    }
}
