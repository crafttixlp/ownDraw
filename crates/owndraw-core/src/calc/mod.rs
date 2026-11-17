//! Rechenkern (Werkzeug „Rechnen“): rechnet Formelzeilen einer Seite mit Variablen und Einheiten aus.

/// Einstellungen des Rechenkerns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opts {
    /// Rechnen eingeschaltet (aus = keine Ergebnisse, z. B. für Prüfungen).
    pub on: bool,
    /// Winkel ohne Einheit in Grad (sonst Bogenmaß).
    pub deg: bool,
    /// Gültige Stellen im Ergebnis (2–10).
    pub digits: u8,
    /// Dezimalkomma statt Punkt.
    pub comma: bool,
    /// Rechenweg unter Ergebniszeilen anzeigen.
    pub steps: bool,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts { on: true, deg: true, digits: 4, comma: true, steps: false }
    }
}

/// Getippte Zeile (z. B. `a: 1,5 kN`, `M = a·b`, `x = sqrt(2)/3`) → LaTeX-Quelltext für ein Formel-Objekt.
pub fn to_latex(plain: &str) -> String {
    plain.to_string()
}

/// LaTeX-Quelltext eines Formel-Objekts → getippte Schreibweise (zum Bearbeiten).
pub fn to_plain(latex: &str) -> String {
    latex.to_string()
}
