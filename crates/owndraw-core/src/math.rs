//! Mathe-Formeln: eine nützliche LaTeX-Teilmenge parsen, setzen und zeichnen – ohne System-TeX, ohne JavaScript.
//!
//! Unterstützt: `\frac`, `\sqrt[n]{…}`, `^`/`_`, `\left( … \right)` (skaliert), griechische Buchstaben,
//! Operatoren/Relationen/Pfeile, `\sum \prod \int` mit Grenzen, `\lim \sin …`, `\text{…}`, Akzente
//! (`\vec \bar \hat \dot \overline \underline`), Leerräume, Matrizen (`matrix pmatrix bmatrix vmatrix cases`).
//!
//! Glyphen kommen aus der eingebetteten Schrift (Ubuntu Light, Ausweichschrift Hack für Pfeile und
//! Mengenzeichen); Bruchstriche, Wurzelzeichen, große Klammern und Akzente sind Pfade.
//! Fehlerhafte Eingabe stürzt nie ab: die Stelle wird rot gezeichnet, `Formula::errors` nennt den Grund.

use crate::canvas::{Canvas, Cap, Paint, Style};
use tiny_skia::PathBuilder;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

/// Farbe fehlerhafter Stellen.
pub const ERR_COLOR: u32 = 0xFFE5484D;

thread_local! {
    static FACES: Vec<Face<'static>> = {
        let mut v = Vec::new();
        for data in [epaint_default_fonts::UBUNTU_LIGHT, epaint_default_fonts::HACK_REGULAR] {
            if let Ok(f) = Face::parse(data, 0) {
                v.push(f);
            }
        }
        v
    };
}

// ---------------------------------------------------------------- Syntaxbaum

/// Abstandsklasse eines Atoms (wie TeX: Ord, Op, Bin, Rel, Open, Close, Punct).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cls {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
}

/// Schriftschnitt einer Zeichenfolge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fnt {
    /// kursiv (Variablen)
    It,
    /// aufrecht (Ziffern, Operatoren, Funktionsnamen)
    Up,
    /// doppelt gezeichnet (`\mathbb`, `\mathbf`)
    Bold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acc {
    Vec,
    Bar,
    Hat,
    Tilde,
    Dot,
    Ddot,
    Under,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Sym { s: String, f: Fnt, cls: Cls },
    /// `\text{…}`: aufrecht, Leerzeichen bleiben.
    Text(String),
    Group(Vec<Node>),
    Frac(Box<Node>, Box<Node>),
    Sqrt(Option<Box<Node>>, Box<Node>),
    Scripts { base: Box<Node>, sup: Option<Box<Node>>, sub: Option<Box<Node>> },
    /// Großer Operator oder Funktionsname; `big` = vergrößertes Zeichen, `limits` = Grenzen über/unter (abgesetzt).
    Op { s: String, big: bool, limits: bool },
    /// `\left l … \right r` ('\0' = keine Klammer, `.`).
    Delim { l: char, r: char, body: Box<Node> },
    /// Matrix; `left` = Zellen linksbündig (cases).
    Matrix { rows: Vec<Vec<Node>>, l: char, r: char, left: bool },
    Accent(Acc, Box<Node>),
    /// Leerraum in em (negativ erlaubt).
    Space(f32),
    /// Fehlerhafte Stelle (Quelltext), rot.
    Err(String),
}

/// Ergebnis des Parsers: Baum (immer vorhanden) und Fehlermeldungen (leer = gültig).
#[derive(Clone, Debug)]
pub struct Parsed {
    pub root: Node,
    pub errors: Vec<String>,
}

const MAX_DEPTH: usize = 48;
const MAX_LEN: usize = 4000;

struct P {
    c: Vec<char>,
    i: usize,
    errs: Vec<String>,
    depth: usize,
}

/// Wo eine Liste endet.
#[derive(Clone, Copy, PartialEq)]
enum Stop {
    Eof,
    Brace,
    Bracket,
    Right,
    Env,
}

pub fn parse(src: &str) -> Parsed {
    let mut c: Vec<char> = src.chars().collect();
    let mut errs = Vec::new();
    if c.len() > MAX_LEN {
        c.truncate(MAX_LEN);
        errs.push("Formel zu lang".to_string());
    }
    let mut p = P { c, i: 0, errs, depth: 0 };
    let list = p.list(Stop::Eof);
    Parsed { root: Node::Group(list), errors: p.errs }
}

fn sym(s: &str, f: Fnt, cls: Cls) -> Node {
    Node::Sym { s: s.to_string(), f, cls }
}

/// Befehle für Zeichen: Name → (Zeichen, Klasse, kursiv).
fn symbol(name: &str) -> Option<(&'static str, Cls, bool)> {
    use Cls::*;
    Some(match name {
        // griechisch klein (kursiv)
        "alpha" => ("α", Ord, true),
        "beta" => ("β", Ord, true),
        "gamma" => ("γ", Ord, true),
        "delta" => ("δ", Ord, true),
        "epsilon" | "varepsilon" => ("ε", Ord, true),
        "zeta" => ("ζ", Ord, true),
        "eta" => ("η", Ord, true),
        "theta" | "vartheta" => ("θ", Ord, true),
        "iota" => ("ι", Ord, true),
        "kappa" => ("κ", Ord, true),
        "lambda" => ("λ", Ord, true),
        "mu" => ("μ", Ord, true),
        "nu" => ("ν", Ord, true),
        "xi" => ("ξ", Ord, true),
        "omicron" => ("ο", Ord, true),
        "pi" => ("π", Ord, true),
        "rho" | "varrho" => ("ρ", Ord, true),
        "sigma" => ("σ", Ord, true),
        "varsigma" => ("ς", Ord, true),
        "tau" => ("τ", Ord, true),
        "upsilon" => ("υ", Ord, true),
        "phi" | "varphi" => ("φ", Ord, true),
        "chi" => ("χ", Ord, true),
        "psi" => ("ψ", Ord, true),
        "omega" => ("ω", Ord, true),
        // griechisch groß (aufrecht)
        "Gamma" => ("Γ", Ord, false),
        "Delta" => ("Δ", Ord, false),
        "Theta" => ("Θ", Ord, false),
        "Lambda" => ("Λ", Ord, false),
        "Xi" => ("Ξ", Ord, false),
        "Pi" => ("Π", Ord, false),
        "Sigma" => ("Σ", Ord, false),
        "Upsilon" => ("Υ", Ord, false),
        "Phi" => ("Φ", Ord, false),
        "Psi" => ("Ψ", Ord, false),
        "Omega" => ("Ω", Ord, false),
        // Binäre Operatoren
        "cdot" => ("·", Bin, false),
        "times" => ("×", Bin, false),
        "div" => ("÷", Bin, false),
        "pm" => ("±", Bin, false),
        "mp" => ("∓", Bin, false),
        "ast" => ("∗", Bin, false),
        "circ" => ("∘", Bin, false),
        "cup" => ("∪", Bin, false),
        "cap" => ("∩", Bin, false),
        "setminus" => ("∖", Bin, false),
        "wedge" | "land" => ("∧", Bin, false),
        "vee" | "lor" => ("∨", Bin, false),
        "oplus" => ("⊕", Bin, false),
        "otimes" => ("⊗", Bin, false),
        // Relationen
        "le" | "leq" => ("≤", Rel, false),
        "ge" | "geq" => ("≥", Rel, false),
        "ne" | "neq" => ("≠", Rel, false),
        "approx" => ("≈", Rel, false),
        "equiv" => ("≡", Rel, false),
        "sim" => ("∼", Rel, false),
        "simeq" => ("≃", Rel, false),
        "cong" => ("≅", Rel, false),
        "propto" => ("∝", Rel, false),
        "ll" => ("≪", Rel, false),
        "gg" => ("≫", Rel, false),
        "in" => ("∈", Rel, false),
        "notin" => ("∉", Rel, false),
        "ni" => ("∋", Rel, false),
        "subset" => ("⊂", Rel, false),
        "supset" => ("⊃", Rel, false),
        "subseteq" => ("⊆", Rel, false),
        "supseteq" => ("⊇", Rel, false),
        "perp" => ("⊥", Rel, false),
        "parallel" => ("∥", Rel, false),
        "mid" => ("|", Rel, false),
        "to" | "rightarrow" => ("→", Rel, false),
        "leftarrow" | "gets" => ("←", Rel, false),
        "leftrightarrow" => ("↔", Rel, false),
        "Rightarrow" | "implies" => ("⇒", Rel, false),
        "Leftarrow" => ("⇐", Rel, false),
        "Leftrightarrow" | "iff" => ("⇔", Rel, false),
        "mapsto" => ("↦", Rel, false),
        "uparrow" => ("↑", Rel, false),
        "downarrow" => ("↓", Rel, false),
        // Sonstiges
        "infty" => ("∞", Ord, false),
        "partial" => ("∂", Ord, false),
        "nabla" => ("∇", Ord, false),
        "forall" => ("∀", Ord, false),
        "exists" => ("∃", Ord, false),
        "neg" | "lnot" => ("¬", Ord, false),
        "emptyset" | "varnothing" => ("∅", Ord, false),
        "angle" => ("∠", Ord, false),
        "degree" => ("°", Ord, false),
        "prime" => ("′", Ord, false),
        "ldots" | "dots" => ("…", Ord, false),
        "cdots" => ("⋯", Ord, false),
        "hbar" => ("ħ", Ord, false),
        "ell" => ("ℓ", Ord, false),
        "Re" => ("ℜ", Ord, false),
        "Im" => ("ℑ", Ord, false),
        "aleph" => ("ℵ", Ord, false),
        "langle" => ("⟨", Open, false),
        "rangle" => ("⟩", Close, false),
        "lfloor" => ("⌊", Open, false),
        "rfloor" => ("⌋", Close, false),
        "lceil" => ("⌈", Open, false),
        "rceil" => ("⌉", Close, false),
        "lbrace" => ("{", Open, false),
        "rbrace" => ("}", Close, false),
        "vert" | "lvert" | "rvert" => ("|", Ord, false),
        "Vert" | "lVert" | "rVert" => ("‖", Ord, false),
        _ => return None,
    })
}

/// Große Operatoren und Funktionsnamen: Name → (Zeichen, groß, Grenzen über/unter).
fn operator(name: &str) -> Option<(&'static str, bool, bool)> {
    Some(match name {
        "sum" => ("∑", true, true),
        "prod" => ("∏", true, true),
        "coprod" => ("∐", true, true),
        "bigcup" => ("⋃", true, true),
        "bigcap" => ("⋂", true, true),
        "int" => ("∫", true, false),
        "iint" => ("∬", true, false),
        "iiint" => ("∭", true, false),
        "oint" => ("∮", true, false),
        "lim" | "max" | "min" | "sup" | "inf" | "det" | "gcd" | "Pr" | "liminf" | "limsup" | "argmax" | "argmin" => {
            let s = match name {
                "lim" => "lim",
                "max" => "max",
                "min" => "min",
                "sup" => "sup",
                "inf" => "inf",
                "det" => "det",
                "gcd" => "gcd",
                "Pr" => "Pr",
                "liminf" => "lim inf",
                "limsup" => "lim sup",
                "argmax" => "arg max",
                _ => "arg min",
            };
            (s, false, true)
        }
        "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "arcsin" | "arccos" | "arctan" | "sinh" | "cosh" | "tanh" | "coth" | "log" | "ln" | "lg"
        | "exp" | "dim" | "ker" | "deg" | "arg" | "hom" => {
            let all = [
                "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth", "log", "ln", "lg", "exp", "dim", "ker", "deg",
                "arg", "hom",
            ];
            (all.iter().find(|s| **s == name).copied().unwrap_or("?"), false, false)
        }
        _ => return None,
    })
}

/// Klammerzeichen nach `\left`/`\right` bzw. als Matrix-Rand.
fn delim_char(name: &str) -> Option<char> {
    Some(match name {
        "(" | ")" | "[" | "]" | "|" | "/" => name.chars().next().unwrap_or('('),
        "." => '\0',
        "{" | "lbrace" => '{',
        "}" | "rbrace" => '}',
        "langle" => '⟨',
        "rangle" => '⟩',
        "lfloor" => '⌊',
        "rfloor" => '⌋',
        "lceil" => '⌈',
        "rceil" => '⌉',
        "vert" | "lvert" | "rvert" => '|',
        "Vert" | "lVert" | "rVert" => '‖',
        _ => return None,
    })
}

impl P {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }
    fn peek_at(&self, k: usize) -> Option<char> {
        self.c.get(self.i + k).copied()
    }
    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }
    fn err(&mut self, m: String) {
        if self.errs.len() < 20 && !self.errs.contains(&m) {
            self.errs.push(m);
        }
    }
    /// Befehlsname hinter `\` (an Position i, ohne den Backslash): Buchstaben oder ein Zeichen.
    fn cmd_name_at(&self, at: usize) -> (String, usize) {
        let mut j = at;
        let mut s = String::new();
        while let Some(ch) = self.c.get(j) {
            if ch.is_ascii_alphabetic() {
                s.push(*ch);
                j += 1;
            } else {
                break;
            }
        }
        if s.is_empty() {
            if let Some(ch) = self.c.get(j) {
                s.push(*ch);
                j += 1;
            }
        }
        (s, j)
    }
    fn at_cmd(&self, name: &str) -> bool {
        self.peek() == Some('\\') && self.cmd_name_at(self.i + 1).0 == name
    }

    fn list(&mut self, stop: Stop) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        loop {
            self.skip_ws();
            let Some(ch) = self.peek() else { break };
            match ch {
                '}' if stop == Stop::Brace => break,
                ']' if stop == Stop::Bracket => break,
                '}' => {
                    self.i += 1;
                    self.err("„}“ ohne „{“".into());
                    out.push(Node::Err("}".into()));
                }
                '&' if stop == Stop::Env => break,
                '&' => {
                    self.i += 1;
                    self.err("„&“ nur in Matrizen".into());
                    out.push(Node::Err("&".into()));
                }
                '\\' if self.peek_at(1) == Some('\\') => {
                    if stop == Stop::Env {
                        break;
                    }
                    self.i += 2;
                    self.err("„\\\\“ nur in Matrizen".into());
                    out.push(Node::Err("\\\\".into()));
                }
                '\\' if self.at_cmd("right") => {
                    if stop == Stop::Right {
                        break;
                    }
                    let (_, j) = self.cmd_name_at(self.i + 1);
                    self.i = j;
                    self.skip_ws();
                    self.delim_token();
                    self.err("\\right ohne \\left".into());
                    out.push(Node::Err("\\right".into()));
                }
                '\\' if self.at_cmd("end") => {
                    if stop == Stop::Env {
                        break;
                    }
                    let (_, j) = self.cmd_name_at(self.i + 1);
                    self.i = j;
                    let _ = self.raw_group();
                    self.err("\\end ohne \\begin".into());
                    out.push(Node::Err("\\end".into()));
                }
                '^' | '_' | '\'' => {
                    self.i += 1;
                    let base = out.pop().unwrap_or(Node::Group(Vec::new()));
                    let (b, mut sup, mut sub) = match base {
                        Node::Scripts { base, sup, sub } => (*base, sup, sub),
                        other => (other, None, None),
                    };
                    if ch == '\'' {
                        // f' : Strich in voller Größe direkt hinter dem Zeichen; mehrere zusammen
                        let mut s = String::from("′");
                        while self.peek() == Some('\'') {
                            self.i += 1;
                            s.push('′');
                        }
                        out.push(if sup.is_none() && sub.is_none() { b } else { Node::Scripts { base: Box::new(b), sup, sub } });
                        out.push(sym(&s, Fnt::Up, Cls::Ord));
                        continue;
                    }
                    let arg = self.arg();
                    let arg = match arg {
                        Some(a) => a,
                        None => {
                            self.err(format!("Nach „{ch}“ fehlt etwas"));
                            Node::Err("?".into())
                        }
                    };
                    let taken = if ch == '_' { sub.is_some() } else { sup.is_some() };
                    if taken {
                        self.err(if ch == '_' { "Doppelter Index".into() } else { "Doppelter Exponent".into() });
                        out.push(Node::Scripts { base: Box::new(b), sup, sub });
                        out.push(Node::Err(ch.to_string()));
                        out.push(arg);
                        continue;
                    }
                    if ch == '_' {
                        sub = Some(Box::new(arg));
                    } else {
                        sup = Some(Box::new(arg));
                    }
                    out.push(Node::Scripts { base: Box::new(b), sup, sub });
                }
                _ => {
                    if let Some(n) = self.atom() {
                        out.push(n);
                    }
                }
            }
        }
        out
    }

    /// Ein Argument: `{…}`, ein Befehl oder ein Zeichen. None am Ende der Eingabe.
    fn arg(&mut self) -> Option<Node> {
        self.skip_ws();
        match self.peek()? {
            '}' | '&' | ']' => None,
            '\\' if self.at_cmd("right") || self.at_cmd("end") || self.peek_at(1) == Some('\\') => None,
            '^' | '_' => None,
            c if c.is_ascii_digit() => {
                // nur eine Ziffer (x^23 = x^2 3 wie in TeX)
                self.i += 1;
                Some(sym(&c.to_string(), Fnt::Up, Cls::Ord))
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Option<Node> {
        let ch = self.peek()?;
        if self.depth > MAX_DEPTH {
            self.err("Formel zu tief verschachtelt".into());
            self.i = self.c.len();
            return Some(Node::Err("…".into()));
        }
        self.depth += 1;
        let n = self.atom_inner(ch);
        self.depth -= 1;
        n
    }

    fn atom_inner(&mut self, ch: char) -> Option<Node> {
        match ch {
            '{' => {
                self.i += 1;
                let l = self.list(Stop::Brace);
                if self.peek() == Some('}') {
                    self.i += 1;
                    Some(Node::Group(l))
                } else {
                    self.err("„}“ fehlt".into());
                    let mut l = l;
                    l.push(Node::Err("}".into()));
                    Some(Node::Group(l))
                }
            }
            '\\' => self.command(),
            c if c.is_ascii_digit() || (c == '.' && self.peek_at(1).is_some_and(|d| d.is_ascii_digit())) => {
                let mut s = String::new();
                while let Some(d) = self.peek() {
                    if d.is_ascii_digit() || (d == '.' && self.peek_at(1).is_some_and(|x| x.is_ascii_digit())) {
                        s.push(d);
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                Some(sym(&s, Fnt::Up, Cls::Ord))
            }
            _ => {
                self.i += 1;
                Some(match ch {
                    '+' => sym("+", Fnt::Up, Cls::Bin),
                    '-' => sym("−", Fnt::Up, Cls::Bin),
                    '*' => sym("∗", Fnt::Up, Cls::Bin),
                    '=' | '<' | '>' | ':' => sym(&ch.to_string(), Fnt::Up, Cls::Rel),
                    '(' | '[' => sym(&ch.to_string(), Fnt::Up, Cls::Open),
                    ')' | ']' => sym(&ch.to_string(), Fnt::Up, Cls::Close),
                    ',' | ';' => sym(&ch.to_string(), Fnt::Up, Cls::Punct),
                    '~' => Node::Space(0.33),
                    c if c.is_alphabetic() && !c.is_uppercase() && ('α'..='ω').contains(&c) => sym(&c.to_string(), Fnt::It, Cls::Ord),
                    c if c.is_ascii_alphabetic() => sym(&c.to_string(), Fnt::It, Cls::Ord),
                    '≤' | '≥' | '≠' | '≈' | '→' | '←' | '⇒' | '⇔' | '∈' | '⊂' | '⊆' => sym(&ch.to_string(), Fnt::Up, Cls::Rel),
                    '·' | '×' | '±' | '÷' | '∪' | '∩' => sym(&ch.to_string(), Fnt::Up, Cls::Bin),
                    c => sym(&c.to_string(), Fnt::Up, Cls::Ord),
                })
            }
        }
    }

    /// Inhalt einer `{…}`-Gruppe als Rohtext (für `\text`, `\begin`). None ohne `{`.
    fn raw_group(&mut self) -> Option<String> {
        self.skip_ws();
        if self.peek() != Some('{') {
            return None;
        }
        self.i += 1;
        let mut depth = 0;
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            self.i += 1;
            match ch {
                '\\' => {
                    // \{ \} \\ im Text: Zeichen übernehmen
                    if let Some(n) = self.peek() {
                        self.i += 1;
                        s.push(n);
                    }
                    continue;
                }
                '{' => depth += 1,
                '}' if depth == 0 => return Some(s),
                '}' => depth -= 1,
                _ => {}
            }
            s.push(ch);
        }
        self.err("„}“ fehlt".into());
        Some(s)
    }

    /// Klammer hinter `\left`/`\right`. None (mit Fehler), wenn keine gültige folgt.
    fn delim_token(&mut self) -> Option<char> {
        self.skip_ws();
        let ch = self.peek()?;
        let name = if ch == '\\' {
            let (n, j) = self.cmd_name_at(self.i + 1);
            self.i = j;
            if n == "|" {
                "Vert".to_string()
            } else {
                n
            }
        } else {
            self.i += 1;
            ch.to_string()
        };
        let d = delim_char(&name);
        if d.is_none() {
            self.err(format!("Unbekannte Klammer „{name}“"));
        }
        d
    }

    fn command(&mut self) -> Option<Node> {
        let start = self.i;
        let (name, j) = self.cmd_name_at(self.i + 1);
        self.i = j;
        if name.is_empty() {
            self.err("„\\“ am Ende".into());
            return Some(Node::Err("\\".into()));
        }
        let missing = |p: &mut P, what: &str| -> Node {
            p.err(format!("Argument fehlt nach \\{what}"));
            Node::Err("?".into())
        };
        Some(match name.as_str() {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let a = self.arg().unwrap_or_else(|| missing(self, &name));
                let b = self.arg().unwrap_or_else(|| missing(self, &name));
                Node::Frac(Box::new(a), Box::new(b))
            }
            "binom" => {
                let a = self.arg().unwrap_or_else(|| missing(self, &name));
                let b = self.arg().unwrap_or_else(|| missing(self, &name));
                Node::Delim { l: '(', r: ')', body: Box::new(Node::Matrix { rows: vec![vec![a], vec![b]], l: '\0', r: '\0', left: false }) }
            }
            "sqrt" => {
                self.skip_ws();
                let idx = if self.peek() == Some('[') {
                    self.i += 1;
                    let l = self.list(Stop::Bracket);
                    if self.peek() == Some(']') {
                        self.i += 1;
                    } else {
                        self.err("„]“ fehlt".into());
                    }
                    Some(Box::new(Node::Group(l)))
                } else {
                    None
                };
                let b = self.arg().unwrap_or_else(|| missing(self, "sqrt"));
                Node::Sqrt(idx, Box::new(b))
            }
            "left" => {
                let l = self.delim_token();
                let body = self.list(Stop::Right);
                let r = if self.at_cmd("right") {
                    let (_, j) = self.cmd_name_at(self.i + 1);
                    self.i = j;
                    self.delim_token()
                } else {
                    self.err("\\right fehlt".into());
                    None
                };
                let mut body = body;
                let (l, r) = match (l, r) {
                    (Some(l), Some(r)) => (l, r),
                    (l, r) => {
                        body.push(Node::Err("\\right".into()));
                        (l.unwrap_or('\0'), r.unwrap_or('\0'))
                    }
                };
                Node::Delim { l, r, body: Box::new(Node::Group(body)) }
            }
            "text" | "textrm" | "mbox" | "textit" | "textbf" => match self.raw_group() {
                Some(s) => Node::Text(s),
                None => missing(self, &name),
            },
            "mathrm" | "operatorname" | "mathbf" | "mathbb" | "boldsymbol" | "mathit" | "mathcal" => {
                let f = match name.as_str() {
                    "mathbf" | "mathbb" | "boldsymbol" => Fnt::Bold,
                    "mathit" | "mathcal" => Fnt::It,
                    _ => Fnt::Up,
                };
                match self.raw_group() {
                    Some(s) if name == "operatorname" => Node::Op { s, big: false, limits: false },
                    Some(s) => sym(&s, f, Cls::Ord),
                    None => match self.arg() {
                        Some(Node::Sym { s, cls, .. }) => Node::Sym { s, f, cls },
                        Some(n) => n,
                        None => missing(self, &name),
                    },
                }
            }
            "vec" | "bar" | "overline" | "hat" | "widehat" | "tilde" | "widetilde" | "dot" | "ddot" | "underline" => {
                let a = match name.as_str() {
                    "vec" => Acc::Vec,
                    "bar" | "overline" => Acc::Bar,
                    "hat" | "widehat" => Acc::Hat,
                    "tilde" | "widetilde" => Acc::Tilde,
                    "dot" => Acc::Dot,
                    "ddot" => Acc::Ddot,
                    _ => Acc::Under,
                };
                let b = self.arg().unwrap_or_else(|| missing(self, &name));
                Node::Accent(a, Box::new(b))
            }
            "begin" => self.env(),
            "," | "thinspace" => Node::Space(3.0 / 18.0),
            ":" | ">" | "medspace" => Node::Space(4.0 / 18.0),
            ";" | "thickspace" => Node::Space(5.0 / 18.0),
            "!" => Node::Space(-3.0 / 18.0),
            " " => Node::Space(0.33),
            "quad" => Node::Space(1.0),
            "qquad" => Node::Space(2.0),
            "{" => sym("{", Fnt::Up, Cls::Open),
            "}" => sym("}", Fnt::Up, Cls::Close),
            "|" => sym("‖", Fnt::Up, Cls::Ord),
            "%" | "$" | "&" | "#" | "_" => sym(&name, Fnt::Up, Cls::Ord),
            "displaystyle" | "textstyle" | "limits" | "nolimits" => Node::Group(Vec::new()),
            n => {
                if let Some((s, cls, it)) = symbol(n) {
                    sym(s, if it { Fnt::It } else { Fnt::Up }, cls)
                } else if let Some((s, big, limits)) = operator(n) {
                    Node::Op { s: s.to_string(), big, limits }
                } else {
                    self.err(format!("Unbekannter Befehl \\{n}"));
                    Node::Err(self.c[start..self.i].iter().collect())
                }
            }
        })
    }

    /// `\begin{env} … \end{env}` (Matrizen und Fallunterscheidung).
    fn env(&mut self) -> Node {
        let Some(name) = self.raw_group() else {
            self.err("\\begin ohne {Umgebung}".into());
            return Node::Err("\\begin".into());
        };
        let (l, r, left) = match name.trim() {
            "matrix" | "smallmatrix" | "array" | "aligned" | "align" | "align*" | "gathered" => ('\0', '\0', false),
            "pmatrix" => ('(', ')', false),
            "bmatrix" => ('[', ']', false),
            "Bmatrix" => ('{', '}', false),
            "vmatrix" => ('|', '|', false),
            "Vmatrix" => ('‖', '‖', false),
            "cases" => ('{', '\0', true),
            other => {
                self.err(format!("Unbekannte Umgebung „{other}“"));
                ('\0', '\0', false)
            }
        };
        if name.trim() == "array" {
            // Spaltenangabe {cc} überspringen
            let _ = self.raw_group();
        }
        let mut rows: Vec<Vec<Node>> = vec![Vec::new()];
        loop {
            let cell = self.list(Stop::Env);
            rows.last_mut().map(|r| r.push(Node::Group(cell)));
            match self.peek() {
                Some('&') => self.i += 1,
                Some('\\') if self.peek_at(1) == Some('\\') => {
                    self.i += 2;
                    rows.push(Vec::new());
                }
                Some('\\') if self.at_cmd("end") => {
                    let (_, j) = self.cmd_name_at(self.i + 1);
                    self.i = j;
                    let end = self.raw_group().unwrap_or_default();
                    if end.trim() != name.trim() {
                        self.err(format!("\\end{{{}}} passt nicht zu \\begin{{{}}}", end.trim(), name.trim()));
                    }
                    break;
                }
                _ => {
                    self.err(format!("\\end{{{}}} fehlt", name.trim()));
                    if let Some(r) = rows.last_mut() {
                        r.push(Node::Err("\\end".into()));
                    }
                    break;
                }
            }
        }
        // leere letzte Zeile (abschließendes \\) weglassen
        if rows.len() > 1 && rows.last().is_some_and(|r| r.iter().all(|c| matches!(c, Node::Group(v) if v.is_empty()))) {
            rows.pop();
        }
        if rows.iter().map(Vec::len).sum::<usize>() > 2500 {
            self.err("Matrix zu groß".into());
            rows.truncate(1);
        }
        let m = Node::Matrix { rows, l: '\0', r: '\0', left };
        if l == '\0' && r == '\0' {
            m
        } else {
            Node::Delim { l, r, body: Box::new(m) }
        }
    }
}

// ---------------------------------------------------------------- Schrift

fn glyph(faces: &[Face<'static>], ch: char) -> Option<(usize, GlyphId)> {
    faces.iter().enumerate().find_map(|(i, f)| f.glyph_index(ch).map(|g| (i, g)))
}

/// Breite, Ober- und Unterlänge (aus den Glyphen-Umrissen) einer Zeichenfolge.
fn ink(s: &str, size: f32) -> (f32, f32, f32) {
    FACES.with(|faces| {
        let (mut w, mut asc, mut desc) = (0.0f32, 0.0f32, 0.0f32);
        for ch in s.chars() {
            match glyph(faces, ch) {
                Some((i, g)) => {
                    let f = &faces[i];
                    let k = size / f.units_per_em() as f32;
                    w += f.glyph_hor_advance(g).unwrap_or(0) as f32 * k;
                    if let Some(bb) = f.glyph_bounding_box(g) {
                        asc = asc.max(bb.y_max as f32 * k);
                        desc = desc.max(-(bb.y_min as f32) * k);
                    }
                }
                None => {
                    w += size * 0.6;
                    asc = asc.max(size * 0.7);
                }
            }
        }
        (w, asc, desc)
    })
}

/// Hat die Schrift ein Zeichen?
pub fn has_glyph(ch: char) -> bool {
    FACES.with(|faces| glyph(faces, ch).is_some())
}

struct Sink<'a> {
    pb: &'a mut PathBuilder,
    k: f32,
    ox: f32,
    oy: f32,
    sk: f32,
}

impl Sink<'_> {
    fn p(&self, x: f32, y: f32) -> (f32, f32) {
        (self.ox + (x + y * self.sk) * self.k, self.oy - y * self.k)
    }
}

impl OutlineBuilder for Sink<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (a, b) = self.p(x, y);
        self.pb.move_to(a, b);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (a, b) = self.p(x, y);
        self.pb.line_to(a, b);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (a, b) = self.p(x1, y1);
        let (c, d) = self.p(x, y);
        self.pb.quad_to(a, b, c, d);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b) = self.p(x1, y1);
        let (c, d) = self.p(x2, y2);
        let (e, f) = self.p(x, y);
        self.pb.cubic_to(a, b, c, d, e, f);
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

/// Glyphen-Umrisse von s (Grundlinie bei y) an den Pfad anhängen; `skew` neigt (kursiv).
fn append_glyphs(pb: &mut PathBuilder, s: &str, x: f32, y: f32, size: f32, skew: f32) {
    FACES.with(|faces| {
        let mut pen = x;
        for ch in s.chars() {
            match glyph(faces, ch) {
                Some((i, g)) => {
                    let f = &faces[i];
                    let k = size / f.units_per_em() as f32;
                    let mut sink = Sink { pb, k, ox: pen, oy: y, sk: skew };
                    let _ = f.outline_glyph(g, &mut sink);
                    pen += f.glyph_hor_advance(g).unwrap_or(0) as f32 * k;
                }
                None => {
                    // Ersatzkästchen
                    if let Some(r) = tiny_skia::Rect::from_xywh(pen + size * 0.1, y - size * 0.7, size * 0.4, size * 0.7) {
                        pb.push_rect(r);
                    }
                    pen += size * 0.6;
                }
            }
        }
    });
}

// ---------------------------------------------------------------- Satz (Layout)

/// Zeichenelement relativ zur Grundlinie (y nach unten).
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    /// Zeichenfolge, Grundlinie bei (x, y).
    Glyphs { s: String, x: f32, y: f32, size: f32, f: Fnt, err: bool },
    /// Gefülltes Rechteck (Bruchstrich).
    Rect { x: f32, y: f32, w: f32, h: f32, err: bool },
    /// Gestrichener Linienzug (x0, y0, x1, y1, …).
    Line { p: Vec<f32>, w: f32, err: bool },
    /// Gefülltes Vieleck (große Klammern, Pfeilspitzen, Punkte).
    Poly { p: Vec<f32>, err: bool },
}

impl Prim {
    fn shift(&mut self, dx: f32, dy: f32) {
        match self {
            Prim::Glyphs { x, y, .. } | Prim::Rect { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            Prim::Line { p, .. } | Prim::Poly { p, .. } => {
                for c in p.chunks_exact_mut(2) {
                    c[0] += dx;
                    c[1] += dy;
                }
            }
        }
    }
    fn set_err(&mut self) {
        match self {
            Prim::Glyphs { err, .. } | Prim::Rect { err, .. } | Prim::Line { err, .. } | Prim::Poly { err, .. } => *err = true,
        }
    }
    pub fn is_err(&self) -> bool {
        match self {
            Prim::Glyphs { err, .. } | Prim::Rect { err, .. } | Prim::Line { err, .. } | Prim::Poly { err, .. } => *err,
        }
    }
}

/// Gesetzter Kasten: Breite, Ober-/Unterlänge über/unter der Grundlinie, Elemente.
#[derive(Clone, Debug, Default)]
struct MBox {
    w: f32,
    asc: f32,
    desc: f32,
    prims: Vec<Prim>,
    /// Klasse für Abstände (None = Leerraum).
    cls: Option<Cls>,
    italic: bool,
}

impl MBox {
    fn put(&mut self, mut b: MBox, dx: f32, dy: f32) {
        for p in b.prims.iter_mut() {
            p.shift(dx, dy);
        }
        self.prims.append(&mut b.prims);
    }
}

/// Achse (Mitte von Bruchstrich und Minus) in em.
const AXIS: f32 = 0.27;
/// Strichstärke von Bruchstrich und Wurzel in em.
const RULE: f32 = 0.055;
const ITALIC_SKEW: f32 = 0.2;

#[derive(Clone, Copy)]
struct Ctx {
    size: f32,
    display: bool,
    min: f32,
}

impl Ctx {
    fn script(self) -> Ctx {
        Ctx { size: (self.size * 0.7).max(self.min), display: false, min: self.min }
    }
    fn text(self) -> Ctx {
        Ctx { display: false, ..self }
    }
}

fn glyph_box(s: &str, f: Fnt, size: f32, cls: Cls) -> MBox {
    let (w, asc, desc) = ink(s, size);
    let w = if f == Fnt::Bold { w + size * 0.04 } else { w };
    MBox { w, asc, desc, prims: vec![Prim::Glyphs { s: s.to_string(), x: 0.0, y: 0.0, size, f, err: false }], cls: Some(cls), italic: f == Fnt::It }
}

/// Abstand (em) zwischen zwei Klassen (vereinfachte TeX-Tabelle).
fn spacing(a: Cls, b: Cls, display_or_text: bool) -> f32 {
    use Cls::*;
    let (thin, med, thick) = (3.0 / 18.0, 4.0 / 18.0, 5.0 / 18.0);
    let v = match (a, b) {
        (Ord, Op) | (Op, Ord) | (Op, Op) | (Close, Op) => thin,
        (Bin, _) | (_, Bin) => med,
        (Rel, Rel) => 0.0,
        (Rel, _) | (_, Rel) => thick,
        (Punct, _) => thin,
        _ => 0.0,
    };
    if !display_or_text && v != thin {
        0.0
    } else {
        v
    }
}

fn hlist(nodes: &[Node], cx: Ctx) -> MBox {
    let mut boxes: Vec<MBox> = nodes.iter().map(|n| layout_node(n, cx)).collect();
    // Bin am Anfang oder nach Op/Bin/Rel/Open/Punct wird zu Ord (Vorzeichen)
    let mut prev: Option<Cls> = None;
    for b in boxes.iter_mut() {
        if let Some(c) = b.cls {
            if c == Cls::Bin && matches!(prev, None | Some(Cls::Bin | Cls::Rel | Cls::Open | Cls::Punct | Cls::Op)) {
                b.cls = Some(Cls::Ord);
            }
            prev = b.cls;
        }
    }
    if let Some(b) = boxes.iter_mut().rev().find(|b| b.cls.is_some()) {
        if b.cls == Some(Cls::Bin) {
            b.cls = Some(Cls::Ord);
        }
    }
    let mut out = MBox { cls: Some(Cls::Ord), ..MBox::default() };
    let mut x = 0.0f32;
    let mut prev: Option<Cls> = None;
    // in Hoch-/Tiefstellungen nur dünne Abstände (wie TeX)
    let big = cx.size >= cx.min / 0.45 * 0.95;
    for b in boxes {
        if let (Some(p), Some(c)) = (prev, b.cls) {
            x += spacing(p, c, big) * cx.size;
        }
        if b.cls.is_some() {
            prev = b.cls;
        }
        out.asc = out.asc.max(b.asc);
        out.desc = out.desc.max(b.desc);
        let w = b.w;
        out.italic = b.italic;
        out.put(b, x, 0.0);
        x += w;
    }
    out.w = x.max(0.0);
    out
}

fn layout_node(n: &Node, cx: Ctx) -> MBox {
    let s = cx.size;
    match n {
        Node::Sym { s: t, f, cls } => glyph_box(t, *f, s, *cls),
        Node::Text(t) => glyph_box(t, Fnt::Up, s, Cls::Ord),
        Node::Group(v) => hlist(v, cx),
        Node::Space(em) => MBox { w: em * s, cls: None, ..MBox::default() },
        Node::Err(t) => {
            let mut b = glyph_box(t, Fnt::Up, s, Cls::Ord);
            // Wellenlinie darunter
            let (w, y) = (b.w.max(s * 0.3), b.desc.max(s * 0.08) + s * 0.08);
            let n = ((w / (s * 0.12)).ceil() as usize).max(2);
            let mut p = Vec::with_capacity(n * 2 + 2);
            for i in 0..=n {
                p.push(w * i as f32 / n as f32);
                p.push(y + if i % 2 == 0 { 0.0 } else { s * 0.06 });
            }
            b.prims.push(Prim::Line { p, w: s * 0.04, err: true });
            b.desc = y + s * 0.1;
            b.w = w;
            for p in b.prims.iter_mut() {
                p.set_err();
            }
            b
        }
        Node::Frac(a, b) => {
            let inner = if cx.display { cx.text() } else { cx.script() };
            let (num, den) = (layout_node(a, inner), layout_node(b, inner));
            let t = (RULE * s).max(0.4);
            let gap = if cx.display { 0.2 } else { 0.12 } * s;
            let pad = 0.12 * s;
            let ax = AXIS * s;
            let u = (ax + t / 2.0 + gap + num.desc).max(if cx.display { 0.68 * s } else { 0.0 });
            let v = (-ax + t / 2.0 + gap + den.asc).max(if cx.display { 0.68 * s } else { 0.0 });
            let w = num.w.max(den.w) + 2.0 * pad;
            let mut out = MBox { w, asc: u + num.asc, desc: v + den.desc, cls: Some(Cls::Ord), ..MBox::default() };
            let (nw, dw) = (num.w, den.w);
            out.put(num, (w - nw) / 2.0, -u);
            out.put(den, (w - dw) / 2.0, v);
            out.prims.push(Prim::Rect { x: pad * 0.5, y: -ax - t / 2.0, w: w - pad, h: t, err: false });
            out
        }
        Node::Sqrt(idx, b) => {
            let body = layout_node(b, cx);
            let t = (RULE * s).max(0.4);
            let gap = s * if cx.display { 0.16 } else { 0.12 };
            let top = -(body.asc.max(s * 0.5) + gap + t / 2.0);
            let bottom = body.desc.max(s * 0.05) + s * 0.06;
            let h = bottom - top;
            let hs = h.min(s * 1.2);
            let sw = s * 0.42 + h * 0.08;
            let mut ox = 0.0f32;
            let mut out = MBox { cls: Some(Cls::Ord), ..MBox::default() };
            if let Some(i) = idx {
                let ib = layout_node(i, Ctx { size: (s * 0.5).max(cx.min * 0.7), display: false, min: cx.min * 0.7 });
                if !ib.prims.is_empty() {
                    let base_y = bottom - 0.55 * h;
                    let right = 0.55 * sw;
                    ox = (ib.w - right + 0.05 * s).max(0.0);
                    out.asc = out.asc.max(-(base_y - ib.asc));
                    let iw = ib.w;
                    out.put(ib, ox + right - iw, base_y);
                }
            }
            let bw = body.w;
            let p = |x: f32, y: f32| [ox + x, y];
            let (p0, p1, p2, p3) = (p(0.0, bottom - 0.42 * hs), p(0.2 * sw, bottom - 0.5 * hs), p(0.48 * sw, bottom), p(sw, top));
            out.prims.push(Prim::Line { p: [p0, p1].concat(), w: t, err: false });
            out.prims.push(Prim::Line { p: [p1, p2].concat(), w: t * 1.6, err: false });
            out.prims.push(Prim::Line { p: [p2, p3, [ox + sw + bw + 0.1 * s, top]].concat(), w: t, err: false });
            out.asc = out.asc.max(-top + t);
            out.desc = bottom + t;
            out.put(body, ox + sw + 0.05 * s, 0.0);
            out.w = ox + sw + bw + 0.15 * s;
            out
        }
        Node::Op { s: t, big, limits } => op_box(t, *big, *limits, cx).0,
        Node::Scripts { base, sup, sub } => scripts(base, sup.as_deref(), sub.as_deref(), cx),
        Node::Delim { l, r, body } => {
            let b = layout_node(body, cx);
            wrap_delims(b, *l, *r, s)
        }
        Node::Matrix { rows, l, r, left } => {
            let m = matrix(rows, *left, cx);
            if *l == '\0' && *r == '\0' {
                m
            } else {
                wrap_delims(m, *l, *r, s)
            }
        }
        Node::Accent(a, b) => {
            let mut body = layout_node(b, cx);
            let t = (RULE * s).max(0.4);
            let w = body.w;
            let sk = if body.italic { ITALIC_SKEW * body.asc * 0.5 } else { 0.0 };
            let y = -(body.asc.max(s * 0.48) + s * 0.12);
            match a {
                Acc::Bar => body.prims.push(Prim::Line { p: vec![sk, y, w + sk * 0.5, y], w: t, err: false }),
                Acc::Under => {
                    let y = body.desc + s * 0.12;
                    body.prims.push(Prim::Line { p: vec![0.0, y, w, y], w: t, err: false });
                    body.desc = y + t;
                }
                Acc::Vec => {
                    let (x0, x1) = (sk, (w + sk).max(sk + s * 0.4));
                    let hs = s * 0.12;
                    body.prims.push(Prim::Line { p: vec![x0, y, x1 - t, y], w: t, err: false });
                    body.prims.push(Prim::Poly { p: vec![x1, y, x1 - hs * 2.0, y - hs, x1 - hs * 1.4, y, x1 - hs * 2.0, y + hs], err: false });
                    body.w = body.w.max(x1);
                }
                Acc::Hat => {
                    let c = w / 2.0 + sk;
                    let hw = (w * 0.4).clamp(s * 0.15, s * 1.2);
                    body.prims.push(Prim::Line { p: vec![c - hw, y + s * 0.02, c, y - s * 0.14, c + hw, y + s * 0.02], w: t, err: false });
                }
                Acc::Tilde => {
                    let c = w / 2.0 + sk;
                    let hw = (w * 0.4).clamp(s * 0.18, s * 1.2);
                    let mut p = Vec::new();
                    for i in 0..=12 {
                        let u = i as f32 / 12.0;
                        p.push(c - hw + 2.0 * hw * u);
                        p.push(y - s * 0.05 - (u * std::f32::consts::TAU).sin() * s * 0.05);
                    }
                    body.prims.push(Prim::Line { p, w: t, err: false });
                }
                Acc::Dot | Acc::Ddot => {
                    let c = w / 2.0 + sk;
                    let r = s * 0.05;
                    let xs: &[f32] = if *a == Acc::Dot { &[0.0] } else { &[-s * 0.12, s * 0.12] };
                    for dx in xs {
                        body.prims.push(Prim::Poly { p: circle(c + dx, y, r), err: false });
                    }
                }
            }
            if *a != Acc::Under {
                body.asc = body.asc.max(-y + s * 0.16);
            }
            body
        }
    }
}

fn circle(cx: f32, cy: f32, r: f32) -> Vec<f32> {
    (0..10).flat_map(|i| {
        let a = i as f32 / 10.0 * std::f32::consts::TAU;
        [cx + r * a.cos(), cy + r * a.sin()]
    }).collect()
}

/// Großer Operator / Funktionsname. Liefert (Kasten, Grenzen über/unter setzen).
fn op_box(t: &str, big: bool, limits: bool, cx: Ctx) -> (MBox, bool) {
    let s = cx.size;
    if !big {
        let mut b = glyph_box(t, Fnt::Up, s, Cls::Op);
        b.italic = false;
        return (b, limits && cx.display);
    }
    let integral = t.contains('∫') || t.contains('∬') || t.contains('∭') || t.contains('∮');
    let gs = s * match (integral, cx.display) {
        (true, true) => 2.0,
        (true, false) => 1.4,
        (false, true) => 1.6,
        (false, false) => 1.15,
    };
    let (w, a, d) = ink(t, gs);
    // Glyphenmitte auf die Achse
    let dy = -AXIS * s - (d - a) / 2.0;
    let mut b = MBox { w: w + if integral { s * 0.1 } else { 0.0 }, asc: a - dy, desc: d + dy, cls: Some(Cls::Op), ..MBox::default() };
    // Integrale neigen (wirken dann wie das kursive TeX-Integral)
    let f = if integral { Fnt::It } else { Fnt::Up };
    b.prims.push(Prim::Glyphs { s: t.to_string(), x: 0.0, y: dy, size: gs, f, err: false });
    if t == "∮" && !has_glyph('∮') {
        b.prims.clear();
    }
    (b, limits && cx.display)
}

fn scripts(base: &Node, sup: Option<&Node>, sub: Option<&Node>, cx: Ctx) -> MBox {
    let s = cx.size;
    let (mut bb, lim) = match base {
        Node::Op { s: t, big, limits } => op_box(t, *big, *limits, cx),
        n => (layout_node(n, cx), false),
    };
    let sc = cx.script();
    let up = sup.map(|n| layout_node(n, sc));
    let dn = sub.map(|n| layout_node(n, sc));
    let cls = bb.cls;
    if lim {
        // Grenzen über und unter dem Operator, zentriert
        let w = bb.w.max(up.as_ref().map_or(0.0, |b| b.w)).max(dn.as_ref().map_or(0.0, |b| b.w));
        let gap = s * 0.12;
        let mut out = MBox { w, asc: bb.asc, desc: bb.desc, cls, ..MBox::default() };
        let (bw, basc, bdesc) = (bb.w, bb.asc, bb.desc);
        out.put(std::mem::take(&mut bb), (w - bw) / 2.0, 0.0);
        if let Some(u) = up {
            let y = -(basc + gap + u.desc);
            out.asc = -y + u.asc;
            let uw = u.w;
            out.put(u, (w - uw) / 2.0, y);
        }
        if let Some(d) = dn {
            let y = bdesc + gap + d.asc;
            out.desc = y + d.desc;
            let dw = d.w;
            out.put(d, (w - dw) / 2.0, y);
        }
        return out;
    }
    let mut u = (bb.asc - 0.3 * s).max(0.4 * s);
    let mut v = (bb.desc + 0.05 * s).max(0.2 * s);
    if let Some(b) = &up {
        u = u.max(b.desc + 0.25 * s);
    }
    if let Some(b) = &dn {
        v = v.max(b.asc - 0.45 * s);
    }
    if let (Some(a), Some(b)) = (&up, &dn) {
        let gap = (u - a.desc) - (b.asc - v);
        let need = 0.2 * s;
        if gap < need {
            v += need - gap;
        }
    }
    let x0 = bb.w;
    let ic = if bb.italic { 0.06 * s } else { 0.0 };
    let integral = matches!(base, Node::Op { big: true, s: t, .. } if t.contains('∫'));
    let (sup_x, sub_x) = if integral { (x0 + 0.02 * s, x0 - 0.25 * s) } else { (x0 + ic, x0) };
    let mut out = MBox { asc: bb.asc, desc: bb.desc, cls, ..MBox::default() };
    let mut w = x0;
    out.put(bb, 0.0, 0.0);
    if let Some(a) = up {
        out.asc = out.asc.max(u + a.asc);
        w = w.max(sup_x + a.w);
        out.put(a, sup_x, -u);
    }
    if let Some(b) = dn {
        out.desc = out.desc.max(v + b.desc);
        w = w.max(sub_x + b.w);
        out.put(b, sub_x, v);
    }
    out.w = w + 0.05 * s;
    out
}

fn matrix(rows: &[Vec<Node>], left: bool, cx: Ctx) -> MBox {
    let s = cx.size;
    let inner = cx.text();
    let cells: Vec<Vec<MBox>> = rows.iter().map(|r| r.iter().map(|n| layout_node(n, inner)).collect()).collect();
    let ncols = cells.iter().map(Vec::len).max().unwrap_or(0);
    let mut cw = vec![0.0f32; ncols];
    for r in &cells {
        for (i, c) in r.iter().enumerate() {
            cw[i] = cw[i].max(c.w);
        }
    }
    let col_gap = if left { 1.0 } else { 0.8 } * s;
    let row_gap = 0.3 * s;
    // Grundlinien der Zeilen (erste bei 0)
    let mut ys = Vec::with_capacity(cells.len());
    let mut y = 0.0f32;
    let mut prev_desc = 0.0f32;
    for (i, r) in cells.iter().enumerate() {
        let asc = r.iter().map(|c| c.asc).fold(0.7 * s, f32::max);
        if i > 0 {
            y += (prev_desc + row_gap + asc).max(1.2 * s);
        }
        ys.push((y, asc));
        prev_desc = r.iter().map(|c| c.desc).fold(0.25 * s, f32::max);
    }
    let top = ys.first().map_or(0.0, |(y, a)| y - a);
    let bottom = y + prev_desc;
    let h = bottom - top;
    // vertikal auf die Achse zentrieren
    let dy = -AXIS * s - (top + h / 2.0);
    let w: f32 = cw.iter().sum::<f32>() + col_gap * (ncols.saturating_sub(1)) as f32;
    let mut out = MBox { w, asc: -(top + dy), desc: bottom + dy, cls: Some(Cls::Ord), ..MBox::default() };
    for (r, (by, _)) in cells.into_iter().zip(ys) {
        let mut x = 0.0f32;
        for (i, c) in r.into_iter().enumerate() {
            let off = if left { 0.0 } else { (cw[i] - c.w) / 2.0 };
            out.put(c, x + off, by + dy);
            x += cw[i] + col_gap;
        }
    }
    out
}

/// Klammer der Höhe [top, bottom] (y nach unten) bei x0 mit Breite w; `right` spiegelt.
fn delim_prims(ch: char, x0: f32, w: f32, top: f32, bottom: f32, s: f32, right: bool) -> Vec<Prim> {
    let t = (RULE * s).max(0.4) * 1.2;
    let h = bottom - top;
    let mid = (top + bottom) / 2.0;
    let mx = |x: f32| if right { x0 + w - x } else { x0 + x };
    let quad = |a: (f32, f32), c: (f32, f32), b: (f32, f32), out: &mut Vec<f32>| {
        for i in 0..=12 {
            let u = i as f32 / 12.0;
            let v = 1.0 - u;
            out.push(v * v * a.0 + 2.0 * v * u * c.0 + u * u * b.0);
            out.push(v * v * a.1 + 2.0 * v * u * c.1 + u * u * b.1);
        }
    };
    let ch = match (ch, right) {
        (')', _) => '(',
        (']', _) => '[',
        ('}', _) => '{',
        ('⟩', _) => '⟨',
        ('⌋', _) => '⌊',
        ('⌉', _) => '⌈',
        (c, _) => c,
    };
    match ch {
        '(' => {
            // Mondsichel: außen und innen je eine Kurve, in der Mitte dicker
            let th = (s * 0.07).max(t) + h * 0.015;
            let mut p = Vec::new();
            quad((mx(w * 0.9), top), (mx(-w * 0.15), mid), (mx(w * 0.9), bottom), &mut p);
            let mut q = Vec::new();
            quad((mx(w * 0.9), bottom), (mx(-w * 0.15 + th * 2.0), mid), (mx(w * 0.9), top), &mut q);
            p.extend(q);
            vec![Prim::Poly { p, err: false }]
        }
        '[' => vec![Prim::Line { p: vec![mx(w * 0.85), top, mx(w * 0.25), top, mx(w * 0.25), bottom, mx(w * 0.85), bottom], w: t, err: false }],
        '⌊' => vec![Prim::Line { p: vec![mx(w * 0.25), top, mx(w * 0.25), bottom, mx(w * 0.85), bottom], w: t, err: false }],
        '⌈' => vec![Prim::Line { p: vec![mx(w * 0.85), top, mx(w * 0.25), top, mx(w * 0.25), bottom], w: t, err: false }],
        '|' => vec![Prim::Line { p: vec![x0 + w / 2.0, top, x0 + w / 2.0, bottom], w: t, err: false }],
        '‖' => vec![
            Prim::Line { p: vec![x0 + w * 0.3, top, x0 + w * 0.3, bottom], w: t, err: false },
            Prim::Line { p: vec![x0 + w * 0.7, top, x0 + w * 0.7, bottom], w: t, err: false },
        ],
        '/' => vec![Prim::Line { p: vec![x0 + w * 0.9, top, x0 + w * 0.1, bottom], w: t, err: false }],
        '⟨' => vec![Prim::Line { p: vec![mx(w * 0.85), top, mx(w * 0.2), mid, mx(w * 0.85), bottom], w: t, err: false }],
        '{' => {
            let mut p = Vec::new();
            let (a, b, c) = (w * 0.9, w * 0.5, w * 0.1);
            let q = h * 0.25;
            quad((mx(a), top), (mx(b), top), (mx(b), top + q * 0.5), &mut p);
            quad((mx(b), top + q), (mx(b), mid), (mx(c), mid), &mut p);
            quad((mx(c), mid), (mx(b), mid), (mx(b), bottom - q), &mut p);
            quad((mx(b), bottom - q * 0.5), (mx(b), bottom), (mx(a), bottom), &mut p);
            vec![Prim::Line { p, w: t, err: false }]
        }
        _ => Vec::new(),
    }
}

fn wrap_delims(b: MBox, l: char, r: char, s: f32) -> MBox {
    let ax = AXIS * s;
    let ext = (b.asc - ax).max(b.desc + ax).max(0.5 * s) * 1.05 + 0.05 * s;
    let (top, bottom) = (-ax - ext, -ax + ext);
    let h = bottom - top;
    let dw = |c: char| if c == '\0' { 0.12 * s } else { (0.3 * s + 0.04 * h).min(0.7 * s) };
    let (lw, rw) = (dw(l), dw(r));
    let mut out = MBox { asc: b.asc.max(-top), desc: b.desc.max(bottom), cls: Some(Cls::Ord), ..MBox::default() };
    out.prims.extend(delim_prims(l, 0.0, lw, top, bottom, s, false));
    let pad = 0.06 * s;
    let bw = b.w;
    out.put(b, lw + pad, 0.0);
    let rx = lw + pad + bw + pad;
    out.prims.extend(delim_prims(r, rx, rw, top, bottom, s, true));
    out.w = rx + rw;
    out
}

// ---------------------------------------------------------------- Ergebnis

/// Gesetzte Formel: Elemente relativ zur linken oberen Ecke (Grundlinie bei y = `asc`).
#[derive(Clone, Debug)]
pub struct Formula {
    pub w: f32,
    pub h: f32,
    pub asc: f32,
    pub prims: Vec<Prim>,
    pub errors: Vec<String>,
}

/// Parst und setzt `src` bei Schriftgröße `size` (abgesetzte Formel).
pub fn layout(src: &str, size: f32) -> Formula {
    let p = parse(src);
    let cx = Ctx { size, display: true, min: size * 0.45 };
    let mut b = layout_node(&p.root, cx);
    let pad = size * 0.08;
    let (asc, desc) = (b.asc.max(size * 0.7), b.desc.max(size * 0.2));
    for pr in b.prims.iter_mut() {
        pr.shift(pad, asc + pad);
    }
    let finite = b.prims.iter().all(|p| match p {
        Prim::Glyphs { x, y, size, .. } => x.is_finite() && y.is_finite() && size.is_finite(),
        Prim::Rect { x, y, w, h, .. } => x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite(),
        Prim::Line { p, w, .. } => w.is_finite() && p.iter().all(|v| v.is_finite()),
        Prim::Poly { p, .. } => p.iter().all(|v| v.is_finite()),
    });
    if !finite {
        b.prims.clear();
    }
    Formula { w: (b.w + 2.0 * pad).max(size * 0.5), h: asc + desc + 2.0 * pad, asc: asc + pad, prims: b.prims, errors: p.errors }
}

impl Formula {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Zeichnet die Formel mit der linken oberen Ecke im Ursprung des Canvas.
    pub fn draw(&self, c: &mut Canvas, color: u32) {
        for err in [false, true] {
            let col = if err { ERR_COLOR } else { color };
            let mut fill = PathBuilder::new();
            let mut bold = PathBuilder::new();
            for p in self.prims.iter().filter(|p| p.is_err() == err) {
                match p {
                    Prim::Glyphs { s, x, y, size, f, .. } => {
                        let sk = if *f == Fnt::It { ITALIC_SKEW } else { 0.0 };
                        append_glyphs(&mut fill, s, *x, *y, *size, sk);
                        if *f == Fnt::Bold {
                            append_glyphs(&mut bold, s, *x + size * 0.04, *y, *size, 0.0);
                        }
                    }
                    Prim::Rect { x, y, w, h, .. } => {
                        if let Some(r) = tiny_skia::Rect::from_xywh(*x, *y, w.max(0.01), h.max(0.01)) {
                            fill.push_rect(r);
                        }
                    }
                    Prim::Poly { p, .. } => {
                        if p.len() >= 6 {
                            fill.move_to(p[0], p[1]);
                            for q in p[2..].chunks_exact(2) {
                                fill.line_to(q[0], q[1]);
                            }
                            fill.close();
                        }
                    }
                    Prim::Line { p, w, .. } => {
                        if p.len() >= 4 {
                            let mut pb = PathBuilder::new();
                            pb.move_to(p[0], p[1]);
                            for q in p[2..].chunks_exact(2) {
                                pb.line_to(q[0], q[1]);
                            }
                            if let Some(path) = pb.finish() {
                                c.draw_path(&path, &Paint::stroke(col, *w, Cap::Butt));
                            }
                        }
                    }
                }
            }
            for pb in [fill, bold] {
                if let Some(path) = pb.finish() {
                    c.draw_path(&path, &Paint { color: col, style: Style::Fill, ..Paint::default() });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(src: &str) -> Parsed {
        let p = parse(src);
        assert!(p.errors.is_empty(), "{src}: {:?}", p.errors);
        p
    }

    #[test]
    fn gueltige_formeln() {
        for src in [
            "x^2 + y^2 = r^2",
            "\\frac{a}{b}",
            "\\sqrt{x}",
            "\\sqrt[3]{x+1}",
            "e^{i\\pi} + 1 = 0",
            "a_{n+1} = a_n \\cdot q",
            "\\left( \\frac{1}{2} \\right)^2",
            "\\sum_{k=1}^{n} k = \\frac{n(n+1)}{2}",
            "\\int_0^\\infty e^{-x^2}\\,dx = \\frac{\\sqrt{\\pi}}{2}",
            "\\prod_{i=1}^n x_i",
            "\\alpha \\beta \\Gamma \\Omega \\pm \\times \\le \\ge \\ne \\approx \\rightarrow",
            "\\text{Fläche } A = \\pi r^2",
            "\\lim_{x \\to 0} \\frac{\\sin x}{x} = 1",
            "\\begin{pmatrix} 1 & 0 \\\\ 0 & 1 \\end{pmatrix}",
            "|x| = \\begin{cases} x & x \\ge 0 \\\\ -x & x < 0 \\end{cases}",
            "\\vec{v} \\cdot \\vec{w} = \\bar{x} \\hat{y}",
            "f'(x) = \\lim_{h\\to0}\\frac{f(x+h)-f(x)}{h}",
            "\\left[ x \\right] \\left\\{ y \\right\\} \\left. z \\right|",
            "",
        ] {
            ok(src);
            let f = layout(src, 40.0);
            assert!(f.w > 0.0 && f.h > 0.0 && f.ok(), "{src}");
        }
    }

    #[test]
    fn ungueltige_formeln() {
        for (src, frag) in [
            ("\\frac{a}", "Argument fehlt"),
            ("{x", "„}“ fehlt"),
            ("x}", "ohne"),
            ("\\foo + 1", "Unbekannter Befehl \\foo"),
            ("\\left( x", "\\right fehlt"),
            ("x \\right)", "\\right ohne \\left"),
            ("x^2^3", "Doppelter Exponent"),
            ("\\begin{pmatrix} 1 & 2", "\\end{pmatrix} fehlt"),
            ("a & b", "nur in Matrizen"),
            ("\\sqrt", "Argument fehlt"),
            ("x^", "fehlt"),
            ("\\", "am Ende"),
        ] {
            let p = parse(src);
            assert!(p.errors.iter().any(|e| e.contains(frag)), "{src}: {:?}", p.errors);
            // Layout stürzt nicht ab und zeigt rot
            let f = layout(src, 40.0);
            assert!(!f.ok());
            assert!(f.prims.iter().any(|p| p.is_err()), "{src}: rote Stelle");
        }
    }

    #[test]
    fn robust_bei_muell() {
        let deep = "{".repeat(500) + &"}".repeat(500);
        let f = layout(&deep, 40.0);
        assert!(!f.ok());
        for src in ["\\left", "\\right", "^_^_", "\\begin", "\\end{x}", "\\frac", "}}}{{{", "\\left\\foo x\\right\\bar", &"x^".repeat(300), "\\sqrt[", "\\text{abc"] {
            let f = layout(src, 20.0);
            assert!(f.w.is_finite() && f.h.is_finite(), "{src}");
        }
    }

    #[test]
    fn layout_grundfaelle() {
        let s = 40.0;
        let x = layout("x", s);
        // Exponent hebt die Oberkante, Index senkt die Unterkante
        let sup = layout("x^2", s);
        let sub = layout("x_2", s);
        assert!(sup.asc > x.asc - 0.01 && sup.w > x.w);
        assert!(sub.h - sub.asc > x.h - x.asc - 0.01);
        // Bruch: höher als ein Zeichen, Breite nach dem breiteren Teil
        let fr = layout("\\frac{1}{2}", s);
        let wide = layout("\\frac{1}{2222}", s);
        assert!(fr.h > x.h * 1.5);
        assert!(wide.w > fr.w);
        assert!(fr.prims.iter().any(|p| matches!(p, Prim::Rect { .. })), "Bruchstrich");
        // Wurzel: Zeichen als Linienzug, wächst mit dem Inhalt
        let r1 = layout("\\sqrt{x}", s);
        let r2 = layout("\\sqrt{\\frac{a}{b}}", s);
        assert!(r1.prims.iter().any(|p| matches!(p, Prim::Line { .. })));
        assert!(r2.h > r1.h);
        // \left( \right) skaliert mit dem Inhalt
        let small = layout("\\left( x \\right)", s);
        let big = layout("\\left( \\frac{\\frac{a}{b}}{c} \\right)", s);
        let hp = |f: &Formula| {
            f.prims
                .iter()
                .filter_map(|p| match p {
                    Prim::Poly { p, .. } => {
                        let ys: Vec<f32> = p.chunks_exact(2).map(|c| c[1]).collect();
                        Some(ys.iter().cloned().fold(f32::MIN, f32::max) - ys.iter().cloned().fold(f32::MAX, f32::min))
                    }
                    _ => None,
                })
                .fold(0.0, f32::max)
        };
        assert!(hp(&big) > hp(&small) * 1.5, "{} {}", hp(&big), hp(&small));
        // Summe mit Grenzen: Grenzen über/unter (Breite nicht Summe der Teile)
        let sum = layout("\\sum_{i=1}^{n}", s);
        let sum_inline = layout("\\sum", s);
        assert!(sum.h > sum_inline.h);
        assert!(sum.w < sum_inline.w + layout("i=1", s * 0.7).w);
        // Abstände um Relationen
        assert!(layout("a=b", s).w > layout("ab", s).w + layout("=", s).w);
        // Vorzeichen ohne Abstand
        assert!(layout("-a", s).w < layout("b-a", s).w - layout("b", s).w + 1.0);
        // Matrix: zwei Zeilen höher als eine
        assert!(layout("\\begin{matrix}1\\\\2\\end{matrix}", s).h > layout("\\begin{matrix}1&2\\end{matrix}", s).h);
    }

    #[test]
    fn zeichnet_pixel() {
        let f = layout("\\frac{\\alpha}{\\sqrt{2}} \\le \\sum_{k=0}^\\infty x^k \\rightarrow \\in", 30.0);
        let mut pm = tiny_skia::Pixmap::new(f.w.ceil() as u32 + 2, f.h.ceil() as u32 + 2).unwrap();
        {
            let mut c = Canvas::new(&mut pm);
            f.draw(&mut c, 0xFF000000);
        }
        let n = pm.pixels().iter().filter(|p| p.alpha() > 0).count();
        assert!(n > 200, "{n}");
        // Fehler rot
        let f = layout("\\foo", 30.0);
        let mut pm = tiny_skia::Pixmap::new(f.w.ceil() as u32 + 2, f.h.ceil() as u32 + 2).unwrap();
        {
            let mut c = Canvas::new(&mut pm);
            f.draw(&mut c, 0xFF000000);
        }
        assert!(pm.pixels().iter().any(|p| p.red() > 150 && p.green() < 120 && p.alpha() > 200));
    }

    #[test]
    fn zeichen_vorhanden() {
        for ch in "αβγΓΔΩ·×±≤≥≠≈∞∂→⇒∈⊂∪∑∏∫−".chars() {
            assert!(has_glyph(ch), "{ch}");
        }
    }

    /// Sichtprüfung: `cargo test -p owndraw-core math::tests::bogen -- --ignored` schreibt ein PNG nach $OWNDRAW_SHOT.
    #[test]
    #[ignore]
    fn bogen() {
        let srcs = [
            "x^2 + y^2 = r^2 \\quad a_{n+1} = a_n \\cdot q",
            "\\frac{a+b}{c} \\cdot \\sqrt{x} + \\sqrt[3]{\\frac{1}{2}}",
            "\\sum_{k=1}^{n} k = \\frac{n(n+1)}{2}",
            "\\int_0^\\infty e^{-x^2}\\,dx = \\frac{\\sqrt{\\pi}}{2}",
            "\\left( \\frac{1}{2} \\right)^2 \\le \\left[ \\frac{\\frac{a}{b}}{c} \\right] \\ne \\left\\{ x \\right\\}",
            "\\lim_{x \\to 0} \\frac{\\sin x}{x} = 1 \\Rightarrow \\alpha\\beta\\gamma\\Delta\\Omega \\approx \\pm\\infty",
            "A = \\begin{pmatrix} 1 & 0 \\\\ 0 & 1 \\end{pmatrix} \\quad |x| = \\begin{cases} x & x \\ge 0 \\\\ -x & \\text{sonst} \\end{cases}",
            "\\vec{v} \\cdot \\vec{w} \\quad \\bar{x} \\hat{y} \\prod_{i=1}^n x_i \\quad f'(x) \\quad \\foo + \\frac{a}",
        ];
        let fs: Vec<Formula> = srcs.iter().map(|s| layout(s, 36.0)).collect();
        let w = fs.iter().map(|f| f.w).fold(0.0, f32::max).ceil() as u32 + 20;
        let h = fs.iter().map(|f| f.h + 10.0).sum::<f32>().ceil() as u32 + 20;
        let mut pm = tiny_skia::Pixmap::new(w, h).unwrap();
        pm.fill(tiny_skia::Color::WHITE);
        {
            let mut c = Canvas::new(&mut pm);
            let mut y = 10.0;
            for f in &fs {
                c.save();
                c.translate(10.0, y);
                f.draw(&mut c, 0xFF111111);
                c.restore();
                y += f.h + 10.0;
            }
        }
        let out = std::env::var("OWNDRAW_SHOT").unwrap_or_else(|_| "/tmp/formeln.png".into());
        pm.save_png(out).unwrap();
    }
}
