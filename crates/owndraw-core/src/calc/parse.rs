//! Zerlegen von Rechenzeilen: getippte Schreibweise (`M = a·b`, `sqrt(2)`, `30°`) und LaTeX-Teilmenge
//! (`\frac{a}{b}`, `\sqrt[3]{x}`, `\cdot`, `\mathrm{kN}`). Ausgabe als LaTeX oder Klartext.

use super::units;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64, String),
    Id(String),
    /// + - * / ^ ( ) [ ] { } = : ; ! | ,  (Mal = '*', Geteilt = '/', implizites Mal = '.')
    Sym(char),
    Frac,
    Sqrt,
    /// √ bzw. ∛ (Wurzelgrad)
    Root(u8),
    /// Platzhalter in Ergebniszeilen: ? … □
    Hole,
}

const GREEK: [(&str, char); 40] = [
    ("alpha", 'α'),
    ("beta", 'β'),
    ("gamma", 'γ'),
    ("delta", 'δ'),
    ("epsilon", 'ε'),
    ("varepsilon", 'ε'),
    ("zeta", 'ζ'),
    ("eta", 'η'),
    ("theta", 'θ'),
    ("vartheta", 'θ'),
    ("iota", 'ι'),
    ("kappa", 'κ'),
    ("lambda", 'λ'),
    ("mu", 'μ'),
    ("nu", 'ν'),
    ("xi", 'ξ'),
    ("pi", 'π'),
    ("rho", 'ρ'),
    ("varrho", 'ρ'),
    ("sigma", 'σ'),
    ("tau", 'τ'),
    ("upsilon", 'υ'),
    ("phi", 'φ'),
    ("varphi", 'φ'),
    ("chi", 'χ'),
    ("psi", 'ψ'),
    ("omega", 'ω'),
    ("Gamma", 'Γ'),
    ("Delta", 'Δ'),
    ("Theta", 'Θ'),
    ("Lambda", 'Λ'),
    ("Xi", 'Ξ'),
    ("Pi", 'Π'),
    ("Sigma", 'Σ'),
    ("Upsilon", 'Υ'),
    ("Phi", 'Φ'),
    ("Psi", 'Ψ'),
    ("Omega", 'Ω'),
    ("eta", 'η'),
    ("ell", 'l'),
];

/// Funktionsnamen (Klartext und LaTeX ohne Backslash).
pub const FUNCS: [&str; 26] = [
    "sin", "cos", "tan", "cot", "arcsin", "arccos", "arctan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "ln", "log", "lg", "ld", "exp", "sqrt",
    "cbrt", "abs", "sgn", "round", "floor", "ceil", "arccot",
];

fn greek_cmd(c: char) -> Option<&'static str> {
    GREEK.iter().find(|g| g.1 == c && g.0 != "varepsilon" && g.0 != "vartheta" && g.0 != "varrho" && g.0 != "varphi" && g.0 != "ell").map(|g| g.0)
}

fn is_letter(c: char) -> bool {
    c.is_alphabetic() || c == '°' || c == '%' || c == '‰' || c == 'Ω' || c == 'µ'
}

/// Zerlegt eine Zeile in Zeichen. Fehler = nicht rechenbar (z. B. unbekannter LaTeX-Befehl).
pub fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out: Vec<Tok> = Vec::new();
    // Name fortsetzen (Δ T, Index _x)
    fn push_id(out: &mut Vec<Tok>, s: String) {
        if let Some(Tok::Id(p)) = out.last_mut() {
            if p == "Δ" || p.ends_with('_') {
                p.push_str(&s);
                return;
            }
        }
        out.push(Tok::Id(s));
    }
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() || ch == '~' || ch == '&' {
            i += 1;
            continue;
        }
        // Zahl
        if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let mut s = String::new();
            let mut seen_dot = false;
            while i < c.len() {
                let d = c[i];
                if d.is_ascii_digit() {
                    s.push(d);
                    i += 1;
                } else if !seen_dot && (d == '.' || d == ',') && c.get(i + 1).is_some_and(|x| x.is_ascii_digit()) {
                    s.push('.');
                    seen_dot = true;
                    i += 1;
                } else if !seen_dot && d == '{' && c.get(i + 1) == Some(&',') && c.get(i + 2) == Some(&'}') && c.get(i + 3).is_some_and(|x| x.is_ascii_digit()) {
                    s.push('.');
                    seen_dot = true;
                    i += 3;
                } else {
                    break;
                }
            }
            // Zehnerpotenz 1.5e3 / 2E-4
            if c.get(i) == Some(&'e') || c.get(i) == Some(&'E') {
                let mut j = i + 1;
                if matches!(c.get(j), Some('+') | Some('-')) {
                    j += 1;
                }
                if c.get(j).is_some_and(|x| x.is_ascii_digit()) {
                    let mut e = String::from("e");
                    e.extend(&c[i + 1..j]);
                    i = j;
                    while i < c.len() && c[i].is_ascii_digit() {
                        e.push(c[i]);
                        i += 1;
                    }
                    s.push_str(&e);
                }
            }
            let v: f64 = s.parse().map_err(|_| "Zahl nicht lesbar".to_string())?;
            out.push(Tok::Num(v, s));
            continue;
        }
        if ch == '\\' {
            i += 1;
            let st = i;
            if i < c.len() && !c[i].is_ascii_alphabetic() {
                // \, \; \! \  \% \{ \} \|
                let x = c[i];
                i += 1;
                match x {
                    '%' => push_id(&mut out, "%".into()),
                    '{' | '}' => {}
                    '|' => out.push(Tok::Sym('|')),
                    _ => {}
                }
                continue;
            }
            while i < c.len() && c[i].is_ascii_alphabetic() {
                i += 1;
            }
            let name: String = c[st..i].iter().collect();
            match name.as_str() {
                "frac" | "dfrac" | "tfrac" => out.push(Tok::Frac),
                "sqrt" => out.push(Tok::Sqrt),
                "cdot" | "times" | "ast" => out.push(Tok::Sym('*')),
                "div" => out.push(Tok::Sym('/')),
                "approx" | "simeq" | "equiv" => out.push(Tok::Sym('=')),
                "left" | "right" | "bigl" | "bigr" | "Bigl" | "Bigr" | "big" | "Big" => {
                    while i < c.len() && c[i] == ' ' {
                        i += 1;
                    }
                    if c.get(i) == Some(&'.') {
                        i += 1;
                    } else if c.get(i) == Some(&'\\') && c.get(i + 1) == Some(&'{') {
                        i += 2;
                        out.push(Tok::Sym('('));
                    } else if c.get(i) == Some(&'\\') && c.get(i + 1) == Some(&'}') {
                        i += 2;
                        out.push(Tok::Sym(')'));
                    } else if c.get(i) == Some(&'\\') && c.get(i + 1) == Some(&'|') {
                        i += 2;
                        out.push(Tok::Sym('|'));
                    } else if c.get(i) == Some(&'\\') {
                        // \left\lvert …
                        let s2 = i + 1;
                        let mut j = s2;
                        while j < c.len() && c[j].is_ascii_alphabetic() {
                            j += 1;
                        }
                        let n2: String = c[s2..j].iter().collect();
                        i = j;
                        match n2.as_str() {
                            "lvert" | "rvert" | "vert" => out.push(Tok::Sym('|')),
                            "lbrace" => out.push(Tok::Sym('(')),
                            "rbrace" => out.push(Tok::Sym(')')),
                            _ => return Err(format!("\\{n2}")),
                        }
                    }
                }
                "lvert" | "rvert" | "vert" | "mid" => out.push(Tok::Sym('|')),
                "ldots" | "dots" | "cdots" | "square" | "Box" => out.push(Tok::Hole),
                "circ" | "degree" => {
                    // ^{\circ} wird beim '^' behandelt; einzeln = Grad
                    push_id(&mut out, "°".into());
                    if matches!(c.get(i), Some('C') | Some('F')) && !c.get(i + 1).is_some_and(|x| x.is_alphanumeric()) {
                        if let Some(Tok::Id(p)) = out.last_mut() {
                            p.push(c[i]);
                        }
                        i += 1;
                    }
                }
                "quad" | "qquad" | "displaystyle" | "textstyle" | "limits" | "nolimits" => {}
                "mathrm" | "text" | "textrm" | "mathit" | "operatorname" | "mbox" | "mathbf" | "textit" => {
                    // Inhalt als ein Name
                    while i < c.len() && c[i] == ' ' {
                        i += 1;
                    }
                    if c.get(i) != Some(&'{') {
                        continue;
                    }
                    let mut depth = 0;
                    let mut s = String::new();
                    i += 1;
                    while i < c.len() {
                        let x = c[i];
                        i += 1;
                        match x {
                            '{' => depth += 1,
                            '}' if depth == 0 => break,
                            '}' => depth -= 1,
                            '\\' => {
                                let s2 = i;
                                while i < c.len() && c[i].is_ascii_alphabetic() {
                                    i += 1;
                                }
                                let n2: String = c[s2..i].iter().collect();
                                if n2.is_empty() {
                                    if let Some(&y) = c.get(i) {
                                        s.push(y);
                                        i += 1;
                                    }
                                } else if let Some(g) = GREEK.iter().find(|g| g.0 == n2) {
                                    s.push(g.1);
                                } else if n2 == "circ" {
                                    s.push('°');
                                } else if n2 == "cdot" {
                                    s.push('·');
                                }
                            }
                            _ => s.push(x),
                        }
                    }
                    let s = s.trim().to_string();
                    if s.contains(['·', '/', '^', ' ']) {
                        // \mathrm{N\cdot m} oder \mathrm{m/s} → erneut zerlegen
                        let inner = lex(&s)?;
                        out.extend(inner);
                    } else if !s.is_empty() {
                        push_id(&mut out, s);
                    }
                }
                n => {
                    if let Some(g) = GREEK.iter().find(|g| g.0 == n) {
                        push_id(&mut out, g.1.to_string());
                    } else if FUNCS.contains(&n) {
                        out.push(Tok::Id(n.to_string()));
                    } else {
                        return Err(format!("\\{n}"));
                    }
                }
            }
            continue;
        }
        // Hochzahlen-Zeichen ² ³ ⁻¹
        if "⁰¹²³⁴⁵⁶⁷⁸⁹⁻".contains(ch) {
            let mut s = String::new();
            while i < c.len() && "⁰¹²³⁴⁵⁶⁷⁸⁹⁻".contains(c[i]) {
                s.push(match c[i] {
                    '⁻' => '-',
                    '⁰' => '0',
                    '¹' => '1',
                    '²' => '2',
                    '³' => '3',
                    '⁴' => '4',
                    '⁵' => '5',
                    '⁶' => '6',
                    '⁷' => '7',
                    '⁸' => '8',
                    _ => '9',
                });
                i += 1;
            }
            let v: f64 = s.parse().map_err(|_| "Hochzahl nicht lesbar".to_string())?;
            out.push(Tok::Sym('^'));
            out.push(Tok::Sym('{'));
            out.push(Tok::Num(v, s));
            out.push(Tok::Sym('}'));
            continue;
        }
        // ^{\circ} / ^\circ → Grad (°, °C)
        if ch == '^' {
            let rest: String = c[i + 1..].iter().take(16).collect();
            let pat = ["{\\circ}", "\\circ", "{°}", "{\\degree}"].into_iter().find(|p| rest.starts_with(p));
            if let Some(pat) = pat {
                i += 1 + pat.chars().count();
                let mut s = String::from("°");
                while i < c.len() && c[i] == ' ' {
                    i += 1;
                }
                let r2: String = c[i..].iter().take(12).collect();
                if r2.starts_with("\\mathrm{C}") {
                    s.push('C');
                    i += 10;
                } else if r2.starts_with("\\text{C}") {
                    s.push('C');
                    i += 8;
                } else if c.get(i) == Some(&'C') && !c.get(i + 1).is_some_and(|x| x.is_alphanumeric()) {
                    s.push('C');
                    i += 1;
                }
                out.push(Tok::Id(s));
                continue;
            }
        }
        if ch == '_' {
            // Index an Namen hängen: F_1, F_{res}, F_{\mathrm{res}}
            if let Some(Tok::Id(p)) = out.last_mut() {
                i += 1;
                let mut s = String::new();
                if c.get(i) == Some(&'{') {
                    let mut depth = 0;
                    i += 1;
                    while i < c.len() {
                        let x = c[i];
                        i += 1;
                        match x {
                            '{' => depth += 1,
                            '}' if depth == 0 => break,
                            '}' => depth -= 1,
                            '\\' => {
                                let s2 = i;
                                while i < c.len() && c[i].is_ascii_alphabetic() {
                                    i += 1;
                                }
                                let n2: String = c[s2..i].iter().collect();
                                if let Some(g) = GREEK.iter().find(|g| g.0 == n2) {
                                    s.push(g.1);
                                }
                            }
                            ' ' => {}
                            _ => s.push(x),
                        }
                    }
                } else if c.get(i) == Some(&'\\') {
                    let s2 = i + 1;
                    let mut j = s2;
                    while j < c.len() && c[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    let n2: String = c[s2..j].iter().collect();
                    if let Some(g) = GREEK.iter().find(|g| g.0 == n2) {
                        s.push(g.1);
                    }
                    i = j;
                } else {
                    // Klartext: Buchstaben/Ziffern; LaTeX ohne Klammer: ein Zeichen (unterscheidet sich nur bei längeren Indizes)
                    while i < c.len() && (c[i].is_alphanumeric()) {
                        s.push(c[i]);
                        i += 1;
                    }
                }
                if s.is_empty() {
                    return Err("Index fehlt".into());
                }
                p.push('_');
                p.push_str(&s);
                continue;
            }
            return Err("_".into());
        }
        if is_letter(ch) {
            let mut s = String::new();
            while i < c.len() && (is_letter(c[i]) || (c[i].is_ascii_digit() && !s.is_empty() && !s.starts_with('°') && s != "%")) {
                s.push(c[i]);
                i += 1;
            }
            push_id(&mut out, s);
            continue;
        }
        i += 1;
        match ch {
            '·' | '×' | '*' | '⋅' | '∙' => out.push(Tok::Sym('*')),
            '÷' => out.push(Tok::Sym('/')),
            '−' | '–' => out.push(Tok::Sym('-')),
            '≈' => out.push(Tok::Sym('=')),
            '√' => out.push(Tok::Root(2)),
            '∛' => out.push(Tok::Root(3)),
            '∜' => out.push(Tok::Root(4)),
            '?' | '…' | '□' | '▢' | '_' => out.push(Tok::Hole),
            '.' if c.get(i) == Some(&'.') => {
                while c.get(i) == Some(&'.') {
                    i += 1;
                }
                out.push(Tok::Hole);
            }
            '+' | '-' | '/' | '^' | '(' | ')' | '[' | ']' | '{' | '}' | '=' | ':' | ';' | '!' | '|' | ',' => out.push(Tok::Sym(ch)),
            '′' | '\'' => return Err("'".into()),
            _ => return Err(ch.to_string()),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- Syntaxbaum

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    /// Mal ohne Zeichen (2x, 2 π r)
    IMul,
    /// Zahl mal Einheit (1,5 kN)
    UMul,
    Div,
    /// Bruch \frac{a}{b}
    Frac,
    Pow,
}

#[derive(Clone, Debug, PartialEq)]
pub enum E {
    Num(f64, String),
    Var(String),
    Unit(units::Unit),
    Hole,
    Neg(Box<E>),
    Bin(Op, Box<E>, Box<E>),
    /// Funktion, Argument, Basis (log_2)
    Call(String, Box<E>, Option<Box<E>>),
    /// Wurzel: Radikand, Grad (None = 2)
    Root(Box<E>, Option<Box<E>>),
    Abs(Box<E>),
    Fact(Box<E>),
    /// Klammer (sichtbar)
    Par(Box<E>),
}

impl E {
    fn bin(o: Op, a: E, b: E) -> E {
        E::Bin(o, Box::new(a), Box::new(b))
    }
    /// Nur Einheiten (für Wunsch-Einheiten).
    pub fn is_unit(&self) -> bool {
        match self {
            E::Unit(_) => true,
            E::Num(v, _) => *v == 1.0,
            E::Bin(Op::Mul | Op::IMul | Op::Div | Op::Frac, a, b) => a.is_unit() && b.is_unit() && !matches!(**b, E::Num(..)),
            E::Bin(Op::Pow, a, b) => matches!(**a, E::Unit(_) | E::Par(_)) && a.is_unit() && matches!(**b, E::Num(..) | E::Neg(_)),
            E::Par(a) => a.is_unit(),
            E::Neg(a) => matches!(**a, E::Num(..)),
            _ => false,
        }
    }
    /// Alle Variablennamen.
    pub fn vars(&self, out: &mut Vec<String>) {
        match self {
            E::Var(n) => {
                if !out.contains(n) {
                    out.push(n.clone())
                }
            }
            E::Neg(a) | E::Abs(a) | E::Fact(a) | E::Par(a) => a.vars(out),
            E::Bin(_, a, b) => {
                a.vars(out);
                b.vars(out);
            }
            E::Call(_, a, b) | E::Root(a, b) => {
                a.vars(out);
                if let Some(b) = b {
                    b.vars(out);
                }
            }
            _ => {}
        }
    }
    pub fn has_hole(&self) -> bool {
        match self {
            E::Hole => true,
            E::Neg(a) | E::Abs(a) | E::Fact(a) | E::Par(a) => a.has_hole(),
            E::Bin(_, a, b) => a.has_hole() || b.has_hole(),
            E::Call(_, a, b) | E::Root(a, b) => a.has_hole() || b.as_ref().is_some_and(|b| b.has_hole()),
            _ => false,
        }
    }
}

// ---------------------------------------------------------------- Parser

struct P<'a> {
    t: &'a [Tok],
    i: usize,
    abs: usize,
}

type R<T> = Result<T, String>;

impl<'a> P<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is(&self, c: char) -> bool {
        self.peek() == Some(&Tok::Sym(c))
    }
    fn eat(&mut self, c: char) -> bool {
        if self.is(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, c: char) -> R<()> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(format!("„{c}“ fehlt"))
        }
    }

    fn expr(&mut self) -> R<E> {
        let mut a = self.term()?;
        loop {
            if self.eat('+') {
                a = E::bin(Op::Add, a, self.term()?);
            } else if self.eat('-') {
                a = E::bin(Op::Sub, a, self.term()?);
            } else {
                return Ok(a);
            }
        }
    }

    /// Beginnt hier ein Faktor (für Mal ohne Zeichen)?
    fn starts_factor(&self) -> bool {
        match self.peek() {
            Some(Tok::Num(..) | Tok::Id(_) | Tok::Frac | Tok::Sqrt | Tok::Root(_)) => true,
            Some(Tok::Sym('(' | '{' | '[')) => true,
            Some(Tok::Sym('|')) => self.abs == 0,
            _ => false,
        }
    }

    fn term(&mut self) -> R<E> {
        let mut a = self.unary()?;
        loop {
            if self.eat('*') {
                a = E::bin(Op::Mul, a, self.unary()?);
            } else if self.eat('/') || self.eat(':') {
                a = E::bin(Op::Div, a, self.unary()?);
            } else if self.starts_factor() {
                a = E::bin(Op::IMul, a, self.power()?);
            } else {
                return Ok(a);
            }
        }
    }

    fn unary(&mut self) -> R<E> {
        if self.eat('-') {
            let a = self.unary()?;
            // -10 °C: Vorzeichen gehört zur Zahl
            if let E::Bin(Op::UMul, n, u) = &a {
                if let E::Num(v, s) = &**n {
                    return Ok(E::bin(Op::UMul, E::Num(-v, format!("-{s}")), (**u).clone()));
                }
            }
            return Ok(E::Neg(Box::new(a)));
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> R<E> {
        let b = self.postfix()?;
        if self.eat('^') {
            let e = self.exponent()?;
            return Ok(E::bin(Op::Pow, b, e));
        }
        Ok(b)
    }

    fn exponent(&mut self) -> R<E> {
        if self.eat('-') {
            return Ok(E::Neg(Box::new(self.exponent()?)));
        }
        if self.is('{') {
            self.i += 1;
            let e = self.expr()?;
            self.expect('}')?;
            return Ok(e);
        }
        // 2^3, 2^x, 2^(…), LaTeX 2^3 = ein Zeichen
        let e = self.primary(false)?;
        if self.eat('^') {
            let f = self.exponent()?;
            return Ok(E::bin(Op::Pow, e, f));
        }
        Ok(e)
    }

    fn postfix(&mut self) -> R<E> {
        let mut a = self.primary(true)?;
        while self.eat('!') {
            a = E::Fact(Box::new(a));
        }
        Ok(a)
    }

    /// Einheit mit Hochzahl: kN, mm^2, mm², s^{-1}
    fn unit_atom(&mut self) -> Option<E> {
        let st = self.i;
        let u = match self.peek() {
            Some(Tok::Id(s)) => {
                let u = units::lookup(s)?;
                self.i += 1;
                E::Unit(u)
            }
            Some(Tok::Sym('(')) => {
                self.i += 1;
                let Some(e) = self.unit_expr() else {
                    self.i = st;
                    return None;
                };
                if !self.eat(')') {
                    self.i = st;
                    return None;
                }
                E::Par(Box::new(e))
            }
            _ => return None,
        };
        if self.is('^') {
            let s2 = self.i;
            self.i += 1;
            let br = self.eat('{');
            let neg = self.eat('-');
            if let Some(Tok::Num(v, s)) = self.peek().cloned() {
                self.i += 1;
                if !br || self.eat('}') {
                    let n = E::Num(v, s);
                    return Some(E::bin(Op::Pow, u, if neg { E::Neg(Box::new(n)) } else { n }));
                }
            }
            self.i = s2;
        }
        Some(u)
    }

    /// Einheitenausdruck: kN·m, m/s², J/(kg·K), 1/min
    fn unit_expr(&mut self) -> Option<E> {
        let st = self.i;
        let mut a = if let Some(Tok::Num(v, s)) = self.peek().cloned() {
            if v != 1.0 || !self.t.get(self.i + 1).is_some_and(|t| *t == Tok::Sym('/')) {
                return None;
            }
            self.i += 1;
            E::Num(v, s)
        } else {
            match self.unit_atom() {
                Some(u) => u,
                None => {
                    self.i = st;
                    return None;
                }
            }
        };
        loop {
            let s2 = self.i;
            let op = if self.eat('*') {
                Op::Mul
            } else if self.eat('/') {
                Op::Div
            } else if matches!(self.peek(), Some(Tok::Id(_))) {
                Op::IMul
            } else {
                break;
            };
            match self.unit_atom() {
                Some(b) => a = E::bin(op, a, b),
                None => {
                    self.i = s2;
                    break;
                }
            }
        }
        Some(a)
    }

    fn group(&mut self) -> R<E> {
        if self.eat('{') {
            let e = self.expr()?;
            self.expect('}')?;
            return Ok(e);
        }
        self.primary(false)
    }

    fn primary(&mut self, units_ok: bool) -> R<E> {
        let Some(t) = self.peek().cloned() else { return Err("Ausdruck fehlt".into()) };
        self.i += 1;
        match t {
            Tok::Num(v, s) => {
                let n = E::Num(v, s);
                if units_ok {
                    // [m] oder Einheitenkette direkt nach der Zahl
                    if self.is('[') {
                        let st = self.i;
                        self.i += 1;
                        if let Some(u) = self.unit_expr() {
                            if self.eat(']') {
                                return Ok(E::bin(Op::UMul, n, u));
                            }
                        }
                        self.i = st;
                    }
                    let st = self.i;
                    if let Some(u) = self.unit_expr() {
                        // „2 m^2“ ok; „2 (m)“ nur, wenn danach nichts Rechenbares folgt – einfach: Klammer allein nicht
                        if !matches!(u, E::Par(_)) {
                            return Ok(E::bin(Op::UMul, n, u));
                        }
                        self.i = st;
                    }
                }
                Ok(n)
            }
            Tok::Hole => Ok(E::Hole),
            Tok::Sym('(') => {
                let e = self.expr()?;
                self.expect(')')?;
                Ok(E::Par(Box::new(e)))
            }
            Tok::Sym('{') => {
                let e = self.expr()?;
                self.expect('}')?;
                Ok(e)
            }
            Tok::Sym('[') => {
                let Some(u) = self.unit_expr() else { return Err("Einheit in [ ] erwartet".into()) };
                self.expect(']')?;
                Ok(u)
            }
            Tok::Sym('|') => {
                self.abs += 1;
                let e = self.expr()?;
                self.abs -= 1;
                self.expect('|')?;
                Ok(E::Abs(Box::new(e)))
            }
            Tok::Frac => {
                let a = self.group()?;
                let b = self.group()?;
                Ok(E::bin(Op::Frac, a, b))
            }
            Tok::Sqrt => {
                let n = if self.eat('[') {
                    let e = self.expr()?;
                    self.expect(']')?;
                    Some(Box::new(e))
                } else {
                    None
                };
                let a = self.group()?;
                Ok(E::Root(Box::new(a), n))
            }
            Tok::Root(k) => {
                let a = self.power()?;
                let a = match a {
                    E::Par(x) => *x,
                    x => x,
                };
                Ok(E::Root(Box::new(a), if k == 2 { None } else { Some(Box::new(E::Num(k as f64, k.to_string()))) }))
            }
            Tok::Id(s) => {
                // Funktion?
                let (base, sub) = match s.split_once('_') {
                    Some((b, x)) => (b.to_string(), Some(x.to_string())),
                    None => (s.clone(), None),
                };
                if FUNCS.contains(&base.as_str()) && (sub.is_none() || base == "log") {
                    let bs = match sub {
                        Some(x) => Some(Box::new(match x.parse::<f64>() {
                            Ok(v) => E::Num(v, x),
                            Err(_) => E::Var(x),
                        })),
                        None => None,
                    };
                    // Basis in LaTeX: \log_{2} → wurde an den Namen gehängt
                    let arg = if self.is('(') || self.is('{') {
                        let br = self.is('{');
                        self.i += 1;
                        let e = self.expr()?;
                        self.expect(if br { '}' } else { ')' })?;
                        e
                    } else {
                        let a = self.power()?;
                        match a {
                            E::Par(x) => *x,
                            x => x,
                        }
                    };
                    if base == "sqrt" {
                        return Ok(E::Root(Box::new(arg), None));
                    }
                    if base == "cbrt" {
                        return Ok(E::Root(Box::new(arg), Some(Box::new(E::Num(3.0, "3".into())))));
                    }
                    if base == "abs" {
                        return Ok(E::Abs(Box::new(arg)));
                    }
                    return Ok(E::Call(base, Box::new(arg), bs));
                }
                // reine Einheitenzeichen (%, °, °C) sind nie Variablen
                if matches!(s.as_str(), "%" | "‰" | "°" | "°C") {
                    return Ok(E::Unit(units::lookup(&s).unwrap()));
                }
                if s.starts_with('°') {
                    return Err(s);
                }
                Ok(E::Var(s))
            }
            Tok::Sym(c) => Err(format!("„{c}“ an dieser Stelle")),
        }
    }
}

/// Ausdruck aus Zeichen (komplett).
pub fn expr(t: &[Tok]) -> R<E> {
    let mut p = P { t, i: 0, abs: 0 };
    let e = p.expr()?;
    if p.i < t.len() {
        return Err(format!("unerwartet: {:?}", t[p.i]));
    }
    Ok(e)
}

/// Reiner Einheitenausdruck (komplett), optional mit Platzhalter davor: `kN·m`, `? m/s`.
pub fn unit_only(t: &[Tok]) -> Option<E> {
    let mut p = P { t, i: 0, abs: 0 };
    while p.peek() == Some(&Tok::Hole) {
        p.i += 1;
    }
    if p.i == t.len() {
        return None;
    }
    let e = if p.is('[') {
        p.i += 1;
        let e = p.unit_expr()?;
        if !p.eat(']') {
            return None;
        }
        e
    } else {
        p.unit_expr()?
    };
    while p.peek() == Some(&Tok::Hole) {
        p.i += 1;
    }
    (p.i == t.len() && !matches!(e, E::Num(..))).then_some(e)
}

/// Teile einer Zeile: `name:` am Anfang zählt wie `name =`. Getrennt an `=` außerhalb von Klammern.
pub fn split_eq(t: &[Tok]) -> Vec<&[Tok]> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut st = 0;
    for (i, x) in t.iter().enumerate() {
        match x {
            Tok::Sym('(' | '{' | '[') => depth += 1,
            Tok::Sym(')' | '}' | ']') => depth -= 1,
            Tok::Sym('=') if depth == 0 => {
                parts.push(&t[st..i]);
                st = i + 1;
            }
            Tok::Sym(':') if depth == 0 && i == 1 && matches!(t[0], Tok::Id(_)) => {
                parts.push(&t[st..i]);
                st = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&t[st..]);
    parts
}

// ---------------------------------------------------------------- Ausgabe

/// Name als LaTeX: x, \alpha, \Delta T, F_{\mathrm{res}}, \mathit{abc}.
pub fn name_latex(n: &str) -> String {
    let (b, sub) = match n.split_once('_') {
        Some((b, s)) => (b, Some(s)),
        None => (n, None),
    };
    let chars: Vec<char> = b.chars().collect();
    let mut s = String::new();
    let conv = |c: char| match greek_cmd(c) {
        Some(g) => format!("\\{g} "),
        None => c.to_string(),
    };
    if chars.len() == 1 || (chars.len() == 2 && chars[0] == 'Δ') {
        for c in &chars {
            s.push_str(&conv(*c));
        }
        s = s.trim_end().to_string();
    } else {
        s = format!("\\mathit{{{b}}}");
    }
    if let Some(x) = sub {
        let xs: String = x.chars().map(conv).collect::<String>();
        let xs = xs.trim_end();
        if x.chars().count() == 1 || x.chars().all(|c| c.is_ascii_digit()) {
            s.push_str(&format!("_{{{xs}}}"));
        } else {
            s.push_str(&format!("_{{\\mathrm{{{x}}}}}"));
        }
    }
    s
}

fn prec(e: &E) -> u8 {
    match e {
        E::Bin(Op::Add | Op::Sub, ..) => 1,
        E::Bin(Op::Mul | Op::IMul | Op::Div, ..) => 2,
        E::Neg(_) => 2,
        E::Bin(Op::UMul, ..) => 3,
        E::Bin(Op::Pow, ..) => 4,
        E::Num(v, _) if *v < 0.0 => 2,
        _ => 5,
    }
}

/// Ersatz beim Ausgeben (Rechenweg: Variablen durch Werte ersetzen). Liefert (Text, Vorrang).
pub type Subst<'a> = &'a dyn Fn(&str) -> Option<(String, u8)>;

/// Zahl im LaTeX-Quelltext (Komma als {,}).
pub fn num_latex(raw: &str) -> String {
    let raw = raw.replace('.', "{,}");
    match raw.split_once(['e', 'E']) {
        Some((m, e)) => format!("{m}\\cdot 10^{{{}}}", e.trim_start_matches('+')),
        None => raw,
    }
}

pub fn latex(e: &E, sub: Option<Subst>) -> String {
    let p = |x: &E, min: u8| -> String {
        let (s, pr) = latex_p(x, sub);
        if pr < min {
            format!("\\left({s}\\right)")
        } else {
            s
        }
    };
    let _ = p;
    latex_p(e, sub).0
}

fn latex_p(e: &E, sub: Option<Subst>) -> (String, u8) {
    let wrap = |x: &E, min: u8| -> String {
        let (s, pr) = latex_p(x, sub);
        if pr < min {
            format!("\\left({s}\\right)")
        } else {
            s
        }
    };
    let s = match e {
        E::Num(_, raw) => num_latex(raw),
        E::Var(n) => {
            if let Some(f) = sub {
                if let Some(r) = f(n) {
                    return r;
                }
            }
            name_latex(n)
        }
        E::Unit(_) => unit_latex(e),
        E::Hole => "\\ldots".into(),
        E::Neg(a) => format!("-{}", wrap(a, 3)),
        E::Bin(o, a, b) => match o {
            Op::Add => format!("{} + {}", wrap(a, 1), wrap(b, 2)),
            Op::Sub => format!("{} - {}", wrap(a, 1), wrap(b, 2)),
            Op::Mul => format!("{} \\cdot {}", wrap(a, 2), wrap(b, 3)),
            Op::IMul => {
                let l = wrap(a, 2);
                let r = wrap(b, 3);
                // 2x, 2\pi r; zwei Zahlen hintereinander bräuchten einen Punkt
                if matches!(**b, E::Num(..)) || (sub.is_some() && matches!(**b, E::Var(_))) {
                    format!("{l} \\cdot {r}")
                } else {
                    format!("{l}\\,{r}")
                }
            }
            Op::UMul => {
                let u = unit_latex(b);
                if u == "^{\\circ}" {
                    format!("{}^{{\\circ}}", wrap(a, 3))
                } else if u.starts_with("^{\\circ}") {
                    format!("{}\\,{u}", wrap(a, 3))
                } else {
                    format!("{}\\,{}", wrap(a, 3), u)
                }
            }
            Op::Div if a.is_unit() && b.is_unit() => format!("{}/{}", unit_latex(a), unit_latex(b)),
            Op::Div | Op::Frac => {
                let strip = |x: &E| match x {
                    E::Par(i) => latex_p(i, sub).0,
                    x => latex_p(x, sub).0,
                };
                format!("\\frac{{{}}}{{{}}}", strip(a), strip(b))
            }
            Op::Pow => {
                let ex = match &**b {
                    E::Par(i) => latex_p(i, sub).0,
                    x => latex_p(x, sub).0,
                };
                format!("{}^{{{}}}", wrap(a, 5), ex)
            }
        },
        E::Call(f, a, base) => {
            let name = match f.as_str() {
                "asin" => "arcsin",
                "acos" => "arccos",
                "atan" => "arctan",
                x => x,
            };
            let cmd = if ["sin", "cos", "tan", "cot", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "ln", "log", "lg", "exp"].contains(&name) {
                format!("\\{name}")
            } else {
                format!("\\operatorname{{{name}}}")
            };
            let b = base.as_ref().map(|b| format!("_{{{}}}", latex_p(b, sub).0)).unwrap_or_default();
            format!("{cmd}{b}\\left({}\\right)", latex_p(a, sub).0)
        }
        E::Root(a, n) => match n {
            Some(n) => format!("\\sqrt[{}]{{{}}}", latex_p(n, sub).0, latex_p(a, sub).0),
            None => format!("\\sqrt{{{}}}", latex_p(a, sub).0),
        },
        E::Abs(a) => format!("\\left|{}\\right|", latex_p(a, sub).0),
        E::Fact(a) => format!("{}!", wrap(a, 5)),
        E::Par(a) => format!("\\left({}\\right)", latex_p(a, sub).0),
    };
    (s, prec(e))
}

/// Einheitenausdruck als LaTeX (alles in einem \mathrm, Exponenten außen).
pub fn unit_latex(e: &E) -> String {
    match e {
        E::Unit(u) if u.sym == "°" => "^{\\circ}".into(),
        E::Unit(u) if u.sym == "°C" => "^{\\circ}\\mathrm{C}".into(),
        E::Unit(u) => format!("\\mathrm{{{}}}", if u.sym == "%" { "\\%" } else { &u.sym }),
        E::Num(_, r) => r.clone(),
        E::Bin(Op::Pow, a, b) => {
            let ex = match &**b {
                E::Neg(n) => format!("-{}", unit_latex(n)),
                x => unit_latex(x),
            };
            format!("{}^{{{}}}", unit_latex(a), ex)
        }
        E::Bin(Op::Div | Op::Frac, a, b) => format!("{}/{}", unit_latex(a), unit_latex(b)),
        E::Bin(_, a, b) => format!("{}\\cdot {}", unit_latex(a), unit_latex(b)),
        E::Par(a) => format!("\\left({}\\right)", unit_latex(a)),
        E::Neg(a) => format!("-{}", unit_latex(a)),
        _ => String::new(),
    }
}

/// Klartext (zum Bearbeiten): a·b, sqrt(x), x^2, 1,5 kN, 30°.
pub fn plain(e: &E) -> String {
    plain_p(e).0
}

fn plain_p(e: &E) -> (String, u8) {
    let wrap = |x: &E, min: u8| -> String {
        let (s, pr) = plain_p(x);
        if pr < min {
            format!("({s})")
        } else {
            s
        }
    };
    let s = match e {
        E::Num(_, raw) => raw.replace('.', ","),
        E::Var(n) => n.clone(),
        E::Unit(u) => u.sym.clone(),
        E::Hole => "?".into(),
        E::Neg(a) => format!("-{}", wrap(a, 3)),
        E::Bin(o, a, b) => match o {
            Op::Add => format!("{} + {}", wrap(a, 1), wrap(b, 2)),
            Op::Sub => format!("{} - {}", wrap(a, 1), wrap(b, 2)),
            Op::Mul => format!("{}·{}", wrap(a, 2), wrap(b, 3)),
            Op::IMul => {
                if matches!(**b, E::Num(..)) {
                    format!("{}·{}", wrap(a, 2), wrap(b, 3))
                } else {
                    format!("{} {}", wrap(a, 2), wrap(b, 3))
                }
            }
            Op::UMul => {
                let u = unit_plain(b);
                if u == "°" {
                    format!("{}{}", wrap(a, 3), u)
                } else {
                    format!("{} {}", wrap(a, 3), u)
                }
            }
            Op::Div if a.is_unit() && b.is_unit() => format!("{}/{}", unit_plain(a), unit_plain(b)),
            Op::Div | Op::Frac => format!("{}/{}", wrap(a, 2), wrap(b, 3)),
            Op::Pow => {
                let ex = plain_p(b);
                let ex = if ex.1 >= 5 { ex.0 } else { format!("({})", ex.0) };
                format!("{}^{}", wrap(a, 5), ex)
            }
        },
        E::Call(f, a, base) => {
            let b = base.as_ref().map(|b| format!("_{}", plain(b))).unwrap_or_default();
            format!("{f}{b}({})", plain(a))
        }
        E::Root(a, n) => match n {
            Some(n) if plain(n) == "3" => format!("cbrt({})", plain(a)),
            Some(n) => format!("{}√({})", plain(n), plain(a)),
            None => format!("sqrt({})", plain(a)),
        },
        E::Abs(a) => format!("|{}|", plain(a)),
        E::Fact(a) => format!("{}!", wrap(a, 5)),
        E::Par(a) => format!("({})", plain(a)),
    };
    (s, prec(e))
}

pub fn unit_plain(e: &E) -> String {
    match e {
        E::Unit(u) => u.sym.clone(),
        E::Num(_, r) => r.clone(),
        E::Bin(Op::Pow, a, b) => {
            let ex = match &**b {
                E::Neg(n) => format!("-{}", unit_plain(n)),
                x => unit_plain(x),
            };
            match ex.parse::<i32>() {
                Ok(n) => format!("{}{}", unit_plain(a), units::sup(n)),
                Err(_) => format!("{}^{}", unit_plain(a), ex),
            }
        }
        E::Bin(Op::Div | Op::Frac, a, b) => format!("{}/{}", unit_plain(a), unit_plain(b)),
        E::Bin(_, a, b) => format!("{}·{}", unit_plain(a), unit_plain(b)),
        E::Par(a) => format!("({})", unit_plain(a)),
        E::Neg(a) => format!("-{}", unit_plain(a)),
        _ => String::new(),
    }
}

/// Teil einer Zeile als LaTeX (Platzhalter bleiben).
pub fn part_latex(t: &[Tok], unit_ok: bool) -> Option<String> {
    if t.is_empty() {
        return Some(String::new());
    }
    if t.iter().all(|x| *x == Tok::Hole) {
        return Some("\\ldots".into());
    }
    if let Some(u) = unit_only(t).filter(|_| unit_ok) {
        let pre = if t[0] == Tok::Hole { "\\ldots\\," } else { "" };
        return Some(format!("{pre}{}", unit_latex(&u)));
    }
    expr(t).ok().map(|e| latex(&e, None))
}

pub fn part_plain(t: &[Tok], unit_ok: bool) -> Option<String> {
    if t.is_empty() {
        return Some(String::new());
    }
    if t.iter().all(|x| *x == Tok::Hole) {
        return Some("?".into());
    }
    if let Some(u) = unit_only(t).filter(|_| unit_ok) {
        let pre = if t[0] == Tok::Hole { "? " } else { "" };
        return Some(format!("{pre}{}", unit_plain(&u)));
    }
    expr(t).ok().map(|e| plain(&e))
}
