//! Formfaktor nach logischer Breite (egui-Punkte) und Eingabeart (WP0; WP6 nutzt es).

use crate::settings::BarPos;

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

/// Umgebung der Oberfläche für einen Frame (Formfaktor, Eingabeart, Größe der Bedienelemente).
#[derive(Clone, Copy, Debug)]
pub struct Env {
    pub ff: FormFactor,
    pub touch: bool,
    /// Kantenlänge runder Knöpfe in Leisten (≥ `min_touch`).
    pub btn: f32,
}

impl Env {
    pub fn new(width: f32, touch: bool) -> Env {
        let ff = FormFactor::from_width(width);
        let min = min_touch(touch);
        // Auf dem Handy (schmal) etwas größer als das Minimum, auf dem PC kompakter.
        let want: f32 = match ff {
            FormFactor::Compact => 44.0,
            FormFactor::Medium => 44.0,
            FormFactor::Expanded => 40.0,
        };
        Env { ff, touch, btn: want.max(min) }
    }

    /// Tastaturkürzel-Hinweise nur bei großer Oberfläche.
    pub fn hints(&self) -> bool {
        self.ff == FormFactor::Expanded
    }

    /// Tooltip-Text: Name, bei Expanded mit Tastaturkürzel.
    pub fn tip(&self, name: &str, key: &str) -> String {
        if self.hints() && !key.is_empty() {
            format!("{name} ({key})")
        } else {
            name.to_string()
        }
    }
}

/// Position der Werkzeugleiste: Einstellung überschreibt, sonst nach Formfaktor
/// (Handy unten, Tablet links, PC oben).
pub fn toolbar_pos(ff: FormFactor, setting: Option<BarPos>) -> BarPos {
    setting.unwrap_or(match ff {
        FormFactor::Compact => BarPos::Bottom,
        FormFactor::Medium => BarPos::Left,
        FormFactor::Expanded => BarPos::Top,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoints() {
        assert_eq!(FormFactor::from_width(390.0), FormFactor::Compact);
        assert_eq!(FormFactor::from_width(599.9), FormFactor::Compact);
        assert_eq!(FormFactor::from_width(600.0), FormFactor::Medium);
        assert_eq!(FormFactor::from_width(1199.0), FormFactor::Medium);
        assert_eq!(FormFactor::from_width(1200.0), FormFactor::Expanded);
    }

    #[test]
    fn touch_targets() {
        for w in [390.0, 820.0, 1920.0] {
            assert!(Env::new(w, true).btn >= min_touch(true));
            assert!(Env::new(w, false).btn >= min_touch(false));
        }
    }

    #[test]
    fn toolbar_defaults() {
        assert_eq!(toolbar_pos(FormFactor::Compact, None), BarPos::Bottom);
        assert_eq!(toolbar_pos(FormFactor::Medium, None), BarPos::Left);
        assert_eq!(toolbar_pos(FormFactor::Expanded, None), BarPos::Top);
        assert_eq!(toolbar_pos(FormFactor::Compact, Some(BarPos::Right)), BarPos::Right);
    }

    #[test]
    fn hints_only_expanded() {
        assert_eq!(Env::new(390.0, false).tip("Stift", "P"), "Stift");
        assert_eq!(Env::new(1920.0, false).tip("Stift", "P"), "Stift (P)");
    }
}
