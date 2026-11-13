//! Formfaktor nach logischer Breite (egui-Punkte) und Eingabeart (WP0; WP6 nutzt es).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormFactor {
    /// < 600 pt: Handy (Werkzeugleiste unten, Bottom-Sheets).
    Compact,
    /// 600–1199 pt: Tablet (schwebende Seitenleiste, Popover).
    Medium,
    /// ≥ 1200 pt: Laptop/PC (Leisten oben/links, Ebenen angedockt).
    Expanded,
}

impl FormFactor {
    pub fn from_width(w: f32) -> FormFactor {
        if w < 600.0 {
            FormFactor::Compact
        } else if w < 1200.0 {
            FormFactor::Medium
        } else {
            FormFactor::Expanded
        }
    }
}

/// Mindestgröße für Bedienelemente: 44 pt bei Touch, sonst 32 pt.
pub fn min_touch(touch: bool) -> f32 {
    if touch {
        44.0
    } else {
        32.0
    }
}
