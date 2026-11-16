//! Oberfläche (WP6): drei Bildschirme – Start, Editor, Einstellungen – mit Formfaktor-abhängigem Layout.

#[cfg(target_arch = "wasm32")]
pub mod account;
pub mod dialogs;
pub mod editor;
pub mod settings;
pub mod start;
pub mod tutorial;
#[cfg(not(target_arch = "wasm32"))]
pub mod update;

use egui::Color32;
use std::path::Path;

/// „Fehler melden“ gibt es nur in der Web-App mit Server (nicht am Desktop, auf Android oder in der
/// statischen Web-App ohne `/api`).
pub fn bug_report_available() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        crate::platform::web::api::online()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

/// Öffnet den Dialog „Fehler melden“ (HTML über der App, siehe `crates/owndraw-web/index.html`).
pub fn open_bug_report(_sh: &mut crate::app::Shared) {
    #[cfg(target_arch = "wasm32")]
    if !crate::platform::web::open_bug_report() {
        _sh.toast("Fehler melden ist hier nicht verfügbar.");
    }
}

/// Knopf „Admin-Panel“ nur für angemeldete Admin-Konten in der Web-App mit Server.
/// `/admin` selbst bleibt serverseitig geschützt (Login + 2FA).
pub fn admin_available() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        crate::platform::web::api::online() && crate::platform::web::api::is_admin()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

/// Öffnet das Admin-Panel (`/admin` am Server) in einem neuen Tab.
pub fn open_admin(sh: &mut crate::app::Shared) {
    sh.platform.open_url("/admin");
}

/// Öffnet die Download-Seite der Apps (`/download` am Server; nur sinnvoll, wenn [`bug_report_available`]).
pub fn open_downloads(sh: &mut crate::app::Shared) {
    sh.platform.open_url("/download");
}

/// Kürzt Text mit „…“ auf höchstens `max_w` Breite.
pub fn elide(s: &str, size: f32, max_w: f32, ui: &egui::Ui) -> String {
    let font = egui::FontId::proportional(size);
    let w = |t: &str| ui.fonts_mut(|f| f.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x);
    if w(s) <= max_w {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let t: String = chars.iter().collect::<String>() + "…";
        if w(&t) <= max_w {
            return t;
        }
    }
    "…".into()
}

/// Lokale Zeit als "dd.MM.yyyy · HH:mm".
pub fn fmt_time(ms: i64) -> String {
    let off = local_offset_secs();
    let secs = ms / 1000 + off;
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    // Tage seit 1970 nach Datum (H. Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{d:02}.{m:02}.{y} · {:02}:{:02}", sod / 3600, sod % 3600 / 60)
}

/// Zeitzonen-Versatz aus /etc/localtime bzw. TZ (nur aktueller Versatz, ohne Abhängigkeit).
fn local_offset_secs() -> i64 {
    use std::sync::OnceLock;
    static OFF: OnceLock<i64> = OnceLock::new();
    *OFF.get_or_init(|| {
        // Browser: Versatz aus `Date` (getTimezoneOffset liefert Minuten westlich von UTC).
        #[cfg(target_arch = "wasm32")]
        return -(js_sys::Date::new_0().get_timezone_offset() as i64) * 60;
        #[allow(unreachable_code)]
        let now = owndraw_core::store::now_ms() / 1000;
        let data = std::env::var("TZ")
            .ok()
            .filter(|t| !t.is_empty())
            .and_then(|t| std::fs::read(Path::new("/usr/share/zoneinfo").join(t.trim_start_matches(':'))).ok())
            .or_else(|| std::fs::read("/etc/localtime").ok());
        data.and_then(|d| tzif_offset(&d, now)).unwrap_or(0)
    })
}

/// Liest den zum Zeitpunkt t gültigen UTC-Versatz aus einer TZif-Datei (v1-Block).
fn tzif_offset(d: &[u8], t: i64) -> Option<i64> {
    if d.len() < 44 || &d[0..4] != b"TZif" {
        return None;
    }
    let be = |o: usize| -> Option<i64> { Some(i32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?) as i64) };
    let (timecnt, typecnt) = (be(32)? as usize, be(36)? as usize);
    let trans = 44;
    let idx = trans + timecnt * 4;
    let types = idx + timecnt;
    let mut ti = 0usize;
    for i in 0..timecnt {
        if be(trans + i * 4)? <= t {
            ti = *d.get(idx + i)? as usize;
        }
    }
    if timecnt == 0 && typecnt == 0 {
        return None;
    }
    be(types + ti * 6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeitformat() {
        // 2026-11-13 12:34 UTC; Versatz hängt von der Zeitzone ab, daher nur das Datumsformat prüfen
        let s = fmt_time(1_794_573_240_000);
        assert_eq!(s.len(), "13.11.2026 · 12:34".len());
        assert!(s.contains("11.2026"));
    }
}
