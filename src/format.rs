//! Mise en forme à la française : tailles, pourcentages, dates, nombres, CSV.

use chrono::{Local, TimeZone};
use std::time::Duration;

/// Espace insécable (séparateur de milliers, entre le nombre et son unité).
const NBSP: char = '\u{a0}';

/// « 1 234 567 ».
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(NBSP);
        }
        out.push(c);
    }
    out
}

/// Taille découpée en nombre et unité : (« 1,23 », « Go »). Unités binaires (1 Ko = 1 024 o).
pub fn size_parts(bytes: u64) -> (String, &'static str) {
    const UNITS: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];
    if bytes < 1024 {
        return (bytes.to_string(), UNITS[0]);
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    // Trois chiffres significatifs ; 999,95 Ko s'affiche « 1 000 Ko » plutôt que de déborder.
    let text = if v < 9.995 {
        format!("{v:.2}")
    } else if v < 99.95 {
        format!("{v:.1}")
    } else {
        thousands(v.round() as u64)
    };
    (text.replace('.', ","), UNITS[u])
}

/// « 1,23 Go ».
pub fn size(bytes: u64) -> String {
    let (v, u) = size_parts(bytes);
    format!("{v}{NBSP}{u}")
}

/// Écart signé : « +1,23 Go », « −300 Mo », « 0 o ».
pub fn delta(d: i64) -> String {
    match d {
        0 => size(0),
        d if d > 0 => format!("+{}", size(d as u64)),
        d => format!("−{}", size(d.unsigned_abs())),
    }
}

/// Part d'un total : « 12,3 % », « < 0,1 % », « 100 % ».
pub fn percent(frac: f64) -> String {
    let pc = frac * 100.0;
    if !pc.is_finite() || pc <= 0.0 {
        format!("0{NBSP}%")
    } else if pc < 0.1 {
        format!("< 0,1{NBSP}%")
    } else if pc >= 99.95 {
        format!("100{NBSP}%")
    } else {
        format!("{pc:.1}{NBSP}%").replace('.', ",")
    }
}

/// Date d'un horodatage Unix : « 13/11/2026 » (« — » si inconnue).
pub fn date(unix: i64) -> String {
    if unix <= 0 {
        return "—".into();
    }
    match Local.timestamp_opt(unix, 0).single() {
        Some(t) => t.format("%d/%m/%Y").to_string(),
        None => "—".into(),
    }
}

/// « 13/11/2026 à 14:31 ».
pub fn date_time(unix: i64) -> String {
    if unix <= 0 {
        return "—".into();
    }
    match Local.timestamp_opt(unix, 0).single() {
        Some(t) => t.format("%d/%m/%Y à %H:%M").to_string(),
        None => "—".into(),
    }
}

/// Date pour un tableur : « 2026-11-13 14:31:58 » (vide si inconnue).
pub fn date_iso(unix: i64) -> String {
    if unix <= 0 {
        return String::new();
    }
    match Local.timestamp_opt(unix, 0).single() {
        Some(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => String::new(),
    }
}

/// Durée en secondes avec une décimale : « 12,3 ».
pub fn secs(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f32()).replace('.', ",")
}

/// « 1 dossier », « 12 345 dossiers ».
pub fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", thousands(n), if n > 1 { many } else { one })
}

/// Fichier CSV pour Excel : BOM UTF-8, séparateur « ; », champs entre guillemets, fins de ligne CRLF.
pub fn csv(headers: &[&str], rows: &[Vec<String>]) -> String {
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut out = String::from("\u{feff}");
    out.push_str(&headers.iter().map(|h| quote(h)).collect::<Vec<_>>().join(";"));
    out.push_str("\r\n");
    for r in rows {
        out.push_str(&r.iter().map(|f| quote(f)).collect::<Vec<_>>().join(";"));
        out.push_str("\r\n");
    }
    out
}

/// Tableau séparé par des tabulations, à coller dans Excel.
pub fn tsv(headers: &[&str], rows: &[Vec<String>]) -> String {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let mut out = headers.join("\t");
    out.push('\n');
    for r in rows {
        out.push_str(&r.iter().map(|f| clean(f)).collect::<Vec<_>>().join("\t"));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milliers() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1\u{a0}000");
        assert_eq!(thousands(1234567), "1\u{a0}234\u{a0}567");
    }

    #[test]
    fn tailles() {
        assert_eq!(size(0), "0\u{a0}o");
        assert_eq!(size(1023), "1023\u{a0}o");
        assert_eq!(size(1024), "1,00\u{a0}Ko");
        assert_eq!(size(1536), "1,50\u{a0}Ko");
        assert_eq!(size(10 * 1024 * 1024), "10,0\u{a0}Mo");
        assert_eq!(size(123 * 1024 * 1024), "123\u{a0}Mo");
        assert_eq!(size(1024 * 1024 * 1024), "1,00\u{a0}Go");
        assert_eq!(size(5 * 1024u64.pow(4)), "5,00\u{a0}To");
        // Au-delà du To, on reste en To.
        assert_eq!(size(2048 * 1024u64.pow(4)), "2\u{a0}048\u{a0}To");
        // Juste sous le Mo : on arrondit sans changer d'unité.
        assert_eq!(size(1023 * 1024 + 1023), "1\u{a0}024\u{a0}Ko");
        assert_eq!(size_parts(1024 * 1024), ("1,00".to_string(), "Mo"));
    }

    #[test]
    fn ecarts() {
        assert_eq!(delta(0), "0\u{a0}o");
        assert_eq!(delta(2048), "+2,00\u{a0}Ko");
        assert_eq!(delta(-3 * 1024 * 1024), "−3,00\u{a0}Mo");
    }

    #[test]
    fn pourcentages() {
        assert_eq!(percent(0.0), "0\u{a0}%");
        assert_eq!(percent(0.0005), "< 0,1\u{a0}%");
        assert_eq!(percent(0.1234), "12,3\u{a0}%");
        assert_eq!(percent(1.0), "100\u{a0}%");
        assert_eq!(percent(f64::NAN), "0\u{a0}%");
    }

    #[test]
    fn dates_et_pluriels() {
        assert_eq!(date(0), "—");
        assert_eq!(date_iso(0), "");
        // 2001-09-09 01:46:40 UTC : la date locale est le 8 ou le 9 selon le fuseau.
        let d = date(1_000_000_000);
        assert!(d == "09/09/2001" || d == "08/09/2001", "{d}");
        assert_eq!(plural(1, "fichier", "fichiers"), "1 fichier");
        assert_eq!(plural(2500, "fichier", "fichiers"), "2\u{a0}500 fichiers");
        assert_eq!(secs(Duration::from_millis(12340)), "12,3");
    }

    #[test]
    fn export() {
        let rows = vec![vec!["a;b".to_string(), "dit \"oui\"".to_string()]];
        let c = csv(&["Nom", "Note"], &rows);
        assert_eq!(c, "\u{feff}\"Nom\";\"Note\"\r\n\"a;b\";\"dit \"\"oui\"\"\"\r\n");
        let t = tsv(&["Nom", "Note"], &[vec!["a\tb".to_string(), "c".to_string()]]);
        assert_eq!(t, "Nom\tNote\na b\tc\n");
    }
}
