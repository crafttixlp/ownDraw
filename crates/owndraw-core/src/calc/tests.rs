use super::*;

fn run(lines: &[&str]) -> Vec<String> {
    run_o(lines, &Opts::default())
}

fn run_o(lines: &[&str], o: &Opts) -> Vec<String> {
    page(lines, o).into_iter().map(|x| x.map(|o| o.text()).unwrap_or_default()).collect()
}

/// Letzte Zeile.
fn last(lines: &[&str]) -> String {
    run(lines).pop().unwrap()
}

#[test]
fn beispiel_le() {
    let r = run(&["a: 1 N", "b: 2 m", "M = a·b", "M = N·m", "M ="]);
    assert_eq!(r[3], "2 \\mathrm{N}\\cdot \\mathrm{m}");
    assert_eq!(r[4], "2\\,\\mathrm{N}\\cdot \\mathrm{m}");
    // LaTeX-Quelltext wie vom Formel-Werkzeug
    let r = run(&["a: 1\\,\\mathrm{N}", "b = 2\\,\\mathrm{m}", "M = a \\cdot b", "M = \\ldots\\,\\mathrm{kN}\\cdot\\mathrm{m}"]);
    assert_eq!(r[3], "0{,}002 \\mathrm{kN}\\cdot \\mathrm{m}");
}

#[test]
fn grundrechnen() {
    assert_eq!(last(&["3·4 + 2 ="]), "14");
    assert_eq!(last(&["(1 + 2)·3 ="]), "9");
    assert_eq!(last(&["2^10 ="]), "1024");
    assert_eq!(last(&["-2^2 ="]), "-4");
    assert_eq!(last(&["1/3 ="]), "0{,}3333");
    assert_eq!(last(&["\\frac{1}{4} + 0,5 ="]), "0{,}75");
    assert_eq!(last(&["sqrt(16) + √9 + \\sqrt[3]{27} ="]), "10");
    assert_eq!(last(&["5! ="]), "120");
    assert_eq!(last(&["|-3| ="]), "3");
    assert_eq!(last(&["2π ="]), "6{,}283");
    assert_eq!(last(&["1,5e3 + 2·10^3 ="]), "3500");
    assert_eq!(last(&["10 : 4 ="]), "2{,}5");
    assert_eq!(last(&["200 · 15 % ="]), "30");
    assert_eq!(last(&["3² + 4² ="]), "25");
    assert_eq!(last(&["x = 2", "3x ="]), "6");
    assert_eq!(last(&["123456789 ="]), "1{,}235\\cdot 10^{8}");
    assert_eq!(last(&["0,000012 ="]), "1{,}2\\cdot 10^{-5}");
}

#[test]
fn funktionen_und_winkel() {
    assert_eq!(last(&["sin(30) ="]), "0{,}5");
    assert_eq!(last(&["sin 30° ="]), "0{,}5");
    assert_eq!(last(&["\\cos\\left(60^{\\circ}\\right) ="]), "0{,}5");
    assert_eq!(last(&["arcsin(0,5) ="]), "30^{\\circ}");
    assert_eq!(last(&["α = atan(1)", "tan(α) ="]), "1");
    let rad = Opts { deg: false, ..Opts::default() };
    assert_eq!(run_o(&["sin(π/2) ="], &rad)[0], "1");
    assert_eq!(run_o(&["sin(90°) ="], &rad)[0], "1");
    assert_eq!(run_o(&["asin(1) ="], &rad)[0], "1{,}571");
    assert_eq!(last(&["ln(e) + log(1000) + log_2(8) ="]), "7");
    assert_eq!(last(&["\\log_{2}(1024) ="]), "10");
    assert_eq!(last(&["φ = 45°", "φ = rad"]), "0{,}7854 \\mathrm{rad}");
}

#[test]
fn einheiten() {
    assert_eq!(last(&["s = 100 m", "t = 9,58 s", "v = s/t", "v = km/h"]), "37{,}58 \\mathrm{km}/\\mathrm{h}");
    assert_eq!(last(&["F = 10 kN", "A = 20 mm²", "σ = F/A", "σ = N/mm^2"]), "500 \\mathrm{N}/\\mathrm{mm}^{2}");
    assert_eq!(last(&["F = 10 kN", "A = 20 mm²", "σ = F/A", "σ = MPa"]), "500 \\mathrm{MPa}");
    assert_eq!(last(&["p = 2,5 bar", "p = kPa"]), "250 \\mathrm{kPa}");
    assert_eq!(last(&["U = 230 V", "I = 2 A", "P = U·I", "P ="]), "460\\,\\mathrm{W}");
    assert_eq!(last(&["U = 12 V", "R = 4,7 kΩ", "I = U/R", "I = mA"]), "2{,}553 \\mathrm{mA}");
    assert_eq!(last(&["P = 2 kW", "t = 3 h", "W = P·t", "W = kWh"]), "6 \\mathrm{kWh}");
    assert_eq!(last(&["n = 1500 1/min", "n = 1/s"]), "25 1/\\mathrm{s}");
    assert_eq!(last(&["n = 3000 min^-1", "ω = 2π·n", "ω = 1/s"]), "314{,}2 1/\\mathrm{s}");
    assert_eq!(last(&["m = 2 kg", "c = 4190 J/(kg·K)", "ΔT = 30 K", "Q = m·c·ΔT", "Q = kJ"]), "251{,}4 \\mathrm{kJ}");
    assert_eq!(last(&["T = 20 °C", "T = K"]), "293{,}1 \\mathrm{K}");
    assert_eq!(last(&["T_1 = 20 °C", "T_2 = 80 °C", "ΔT = T_2 - T_1", "ΔT ="]), "60\\,\\mathrm{K}");
    assert_eq!(last(&["T = 300 K", "T = °C"]), "26{,}85 ^{\\circ}\\mathrm{C}");
    assert_eq!(last(&["m = 5 kg", "F = m·g", "F ="]), "49{,}03\\,\\mathrm{kg}\\cdot \\mathrm{m}/\\mathrm{s}^{2}".replace("\\mathrm{kg}\\cdot \\mathrm{m}/\\mathrm{s}^{2}", "\\mathrm{N}"));
    assert_eq!(last(&["v = 72 km/h", "v = m/s"]), "20 \\mathrm{m}/\\mathrm{s}");
    assert_eq!(last(&["V = 2 l", "V = cm³"]), "2000 \\mathrm{cm}^{3}");
}

#[test]
fn warnungen() {
    assert!(last(&["x = 1 N + 1 m"]).contains("Einheiten passen nicht: N + m"));
    assert!(last(&["a = 3 N", "a = m"]).contains("Einheit passt nicht"));
    assert!(last(&["sin(2 m) ="]).contains("sin braucht"));
    assert!(last(&["sqrt(-4) ="]).contains("Wurzel aus negativer Zahl"));
    assert!(last(&["1/0 ="]).contains("Division durch 0"));
    assert!(last(&["q ="]).contains("unbekannt: q"));
}

#[test]
fn loesen() {
    // Le: Unbekannte an beliebiger Stelle
    assert_eq!(last(&["p = 5 MPa", "F = 10 kN", "p = F/A", "A = mm²"]), "2000 \\mathrm{mm}^{2}");
    // Formel vor den Werten
    assert_eq!(last(&["M = F·r", "F = 20 N", "r = 0,5 m", "M ="]), "10\\,\\mathrm{N}\\cdot \\mathrm{m}");
    assert_eq!(last(&["M = F·r", "M = 10 N·m", "F = 20 N", "r = cm"]), "50 \\mathrm{cm}");
    assert_eq!(last(&["2x + 3 = 11", "x ="]), "4");
    assert_eq!(last(&["U = R·I", "U = 230 V", "I = 10 A", "R ="]), "23\\,\\mathrm{Ω}");
    // quadratisch
    assert_eq!(last(&["x² - 5x + 6 = 0", "x ="]), "2\\;\\text{oder}\\;3");
    assert!(last(&["x^2 + 1 = 0", "x ="]).contains("keine reelle Lösung"));
    assert_eq!(last(&["x^2 = 9 m^2", "x = m"]), "-3\\;\\text{oder}\\;3 \\mathrm{m}");
    // nichtlinear numerisch
    assert_eq!(last(&["e^x = 10", "x ="]), "2{,}303");
    assert_eq!(last(&["v = sqrt(2·g·h)", "v = 10 m/s", "h = m"]), "5{,}099 \\mathrm{m}");
    // Systeme
    let r = run(&["x + y = 10", "x - y = 2", "x =", "y ="]);
    assert_eq!(r[2], "6");
    assert_eq!(r[3], "4");
    let r = run(&["2a + 3b + c = 1", "a - b + 2c = 6", "3a + b - c = 0", "a =", "b =", "c ="]);
    assert_eq!(&r[3..], &["1".to_string(), "-1".into(), "2".into()][..].iter().map(|s| s.to_string()).collect::<Vec<_>>()[..]);
    // Strom-/Spannungsteiler mit Einheiten
    let r = run(&["U = 12 V", "U = I·(R_1 + R_2)", "U_2 = I·R_2", "R_1 = 1 kΩ", "U_2 = 4 V", "R_2 = kΩ", "I = mA"]);
    assert_eq!(r[5], "0{,}5 \\mathrm{kΩ}");
    assert_eq!(r[6], "8 \\mathrm{mA}");
}

#[test]
fn abschnitte_und_ueberschreiben() {
    let r = run(&["a = 2", "b = a·3", "b =", "a = 5", "a·3 =", "b ="]);
    assert_eq!(r[2], "6");
    assert_eq!(r[4], "15");
    assert_eq!(r[5], "6", "b bleibt, Zuweisungen gelten von oben nach unten");
}

#[test]
fn rechenweg() {
    let o = Opts { steps: true, ..Opts::default() };
    let out = page(&["a: 1 N", "b: 2 m", "M = a·b", "M ="], &o);
    let Some(Out::Result { steps, .. }) = &out[3] else { panic!() };
    assert_eq!(steps[0], "M = a \\cdot b");
    assert!(steps[1].contains("1\\,\\mathrm{N} \\cdot 2\\,\\mathrm{m}"), "{steps:?}");
    let out = page(&["p = F/A", "p = 2 bar", "F = 100 N", "A = cm²"], &o);
    let Some(Out::Result { steps, .. }) = &out[3] else { panic!() };
    assert!(steps.iter().any(|s| s.contains("gelöst nach")), "{steps:?}");
}

#[test]
fn einstellungen() {
    let o = Opts { comma: false, digits: 6, ..Opts::default() };
    assert_eq!(run_o(&["1/3 ="], &o)[0], "0.333333");
}

#[test]
fn keine_rechenzeilen() {
    let r = run(&["\\int_a^b f(x)\\,dx", "E = mc^2", "\\sum_{i=1}^n i", "Hallo"]);
    assert!(r.iter().all(|s| s.is_empty()), "{r:?}");
}

#[test]
fn latex_umwandeln() {
    assert_eq!(to_latex("a: 1,5 kN"), "a: 1{,}5\\,\\mathrm{kN}");
    assert_eq!(to_latex("M = a·b"), "M = a \\cdot b");
    assert_eq!(to_latex("M ="), "M =");
    assert_eq!(to_latex("M = kN·m"), "M = \\mathrm{kN}\\cdot \\mathrm{m}");
    assert_eq!(to_latex("x = sqrt(2)/3"), "x = \\frac{\\sqrt{2}}{3}");
    assert_eq!(to_latex("F_res = 2 F_1"), "F_{\\mathrm{res}} = 2\\,F_{1}");
    assert_eq!(to_latex("α = 30°"), "\\alpha = 30^{\\circ}");
    assert_eq!(to_latex("ΔT = 5 K"), "\\Delta T = 5\\,\\mathrm{K}");
    assert_eq!(to_latex("v = 3 m/s^2"), "v = 3\\,\\mathrm{m}/\\mathrm{s}^{2}");
    assert_eq!(to_latex("x = ?"), "x = \\ldots");
    assert_eq!(to_latex("T = 20 °C"), "T = 20\\,^{\\circ}\\mathrm{C}");
    assert_eq!(to_latex("c = 4190 J/(kg·K)"), "c = 4190\\,\\mathrm{J}/\\left(\\mathrm{kg}\\cdot \\mathrm{K}\\right)");
    for s in ["a: 1,5 kN", "M = a·b", "M =", "M = kN·m", "x = sqrt(2)/3", "F_res = 2 F_1", "α = 30°", "v = 3 m/s²", "T = 20 °C", "y = sin(x) + 2^(x+1)", "c = 4190 J/(kg·K)", "x = ?"] {
        let l = to_latex(s);
        assert_eq!(to_plain(&l), s.replace("2^(x+1)", "2^(x + 1)"), "Rundlauf {s} → {l}");
        // gleich gerechnet
        assert_eq!(to_latex(&to_plain(&l)), l);
    }
    // gesetzt ohne Fehler
    for s in ["a: 1,5 kN", "M = a·b", "α = 30°", "T = 20 °C", "c = 4190 J/(kg·K)", "x = ?", "F_res = 2 F_1"] {
        let f = crate::math::layout(&to_latex(s), 40.0);
        assert!(f.errors.is_empty(), "{s}: {:?}", f.errors);
    }
}
