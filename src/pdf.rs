//! Écriture de PDF minimale, sans dépendance : pages A4, rectangles, traits, polygones
//! et texte en Helvetica (polices standard de tout lecteur PDF, encodage WinAnsi :
//! les accents français sont pris en charge).
//!
//! Les coordonnées sont en points depuis le coin haut-gauche de la page ; `y` du texte
//! désigne sa ligne de base.

use std::fmt::Write as _;

/// Taille d'une page A4, en points.
pub const PAGE_W: f32 = 595.28;
pub const PAGE_H: f32 = 841.89;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub const fn rgb(hex: u32) -> Rgb {
    Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Rgb {
    fn ops(self) -> String {
        format!("{:.3} {:.3} {:.3}", self.0 as f32 / 255.0, self.1 as f32 / 255.0, self.2 as f32 / 255.0)
    }
}

#[derive(Default)]
pub struct Page {
    ops: Vec<u8>,
}

/// Nombre avec deux décimales au plus, sans zéros inutiles.
fn n(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

impl Page {
    fn push(&mut self, s: &str) {
        self.ops.extend_from_slice(s.as_bytes());
        self.ops.push(b'\n');
    }

    /// Rectangle plein ; (`x`, `y`) est son coin haut-gauche.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Rgb) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.push(&format!("{} rg {} {} {} {} re f", fill.ops(), n(x), n(PAGE_H - y - h), n(w), n(h)));
    }

    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32) {
        self.push(&format!("{} RG {} w {} {} m {} {} l S", color.ops(), n(width), n(x1), n(PAGE_H - y1), n(x2), n(PAGE_H - y2)));
    }

    /// Polygone plein (secteurs de graphiques circulaires…).
    pub fn polygon(&mut self, pts: &[(f32, f32)], fill: Rgb) {
        if pts.len() < 3 {
            return;
        }
        let mut s = format!("{} rg {} {} m", fill.ops(), n(pts[0].0), n(PAGE_H - pts[0].1));
        for &(x, y) in &pts[1..] {
            let _ = write!(s, " {} {} l", n(x), n(PAGE_H - y));
        }
        s.push_str(" h f");
        self.push(&s);
    }

    /// Secteur d'anneau entre les angles `from` et `to` (en tours, 0 = midi, sens horaire).
    pub fn ring_sector(&mut self, cx: f32, cy: f32, r_out: f32, r_in: f32, from: f32, to: f32, fill: Rgb) {
        if to - from <= 0.0005 {
            return;
        }
        let steps = (((to - from) * 120.0).ceil() as usize).max(2);
        let at = |r: f32, t: f32| {
            let a = t * std::f32::consts::TAU;
            (cx + r * a.sin(), cy - r * a.cos())
        };
        let mut pts: Vec<(f32, f32)> = (0..=steps).map(|k| at(r_out, from + (to - from) * k as f32 / steps as f32)).collect();
        pts.extend((0..=steps).rev().map(|k| at(r_in, from + (to - from) * k as f32 / steps as f32)));
        self.polygon(&pts, fill);
    }

    /// Texte sur une ligne, à partir de `x`, ligne de base en `y`.
    pub fn text(&mut self, x: f32, y: f32, size: f32, bold: bool, color: Rgb, s: &str) {
        if s.is_empty() {
            return;
        }
        let font = if bold { "F2" } else { "F1" };
        let mut hex = String::with_capacity(s.len() * 2);
        for b in encode(s) {
            let _ = write!(hex, "{b:02X}");
        }
        self.push(&format!("BT {} rg /{font} {} Tf {} {} Td <{hex}> Tj ET", color.ops(), n(size), n(x), n(PAGE_H - y)));
    }

    /// Texte aligné à droite sur `x`.
    pub fn text_right(&mut self, x: f32, y: f32, size: f32, bold: bool, color: Rgb, s: &str) {
        self.text(x - width(s, size, bold), y, size, bold, color, s);
    }

    /// Texte centré sur `x`.
    pub fn text_center(&mut self, x: f32, y: f32, size: f32, bold: bool, color: Rgb, s: &str) {
        self.text(x - width(s, size, bold) / 2.0, y, size, bold, color, s);
    }
}

// ---------------------------------------------------------------------------
// Encodage et largeurs (métriques Adobe de Helvetica et Helvetica-Bold)
// ---------------------------------------------------------------------------

/// Texte en WinAnsi (Windows-1252). Les caractères absents sont remplacés.
pub fn encode(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c as u32 {
            0x20..=0x7E | 0xA0..=0xFF => c as u32 as u8,
            0x20AC => 0x80,
            0x201A => 0x82,
            0x201E => 0x84,
            0x2026 => 0x85,
            0x2030 => 0x89,
            0x0152 => 0x8C,
            0x2018 => 0x91,
            0x2019 => 0x92,
            0x201C => 0x93,
            0x201D => 0x94,
            0x2022 => 0x95,
            0x2013 => 0x96,
            0x2014 => 0x97,
            0x2122 => 0x99,
            0x0153 => 0x9C,
            0x0178 => 0x9F,
            0x2212 => b'-',
            0x202F | 0x2009 => 0xA0,
            _ => b'?',
        })
        .collect()
}

/// Largeurs (millièmes de cadratin) des caractères ASCII 32 à 126.
const REGULAR: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // espace à /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // chiffres
    278, 278, 584, 584, 584, 556, 1015, // : à @
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, // A-Z
    278, 278, 278, 469, 556, 333, // [ à `
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, // a-z
    334, 260, 334, 584, // { à ~
];
const BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, //
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, //
    333, 333, 584, 584, 584, 611, 975, //
    722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, //
    333, 278, 333, 584, 556, 333, //
    556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, //
    389, 280, 389, 584,
];

fn glyph_width(b: u8, bold: bool) -> u16 {
    let table = if bold { &BOLD } else { &REGULAR };
    let base = |c: u8| table[(c - 32) as usize];
    match b {
        32..=126 => base(b),
        0xA0 => 278,
        0x85 | 0x97 | 0x89 => 1000,
        0x96 | 0x80 | 0xAB | 0xBB => 556,
        0x91 | 0x92 => if bold { 278 } else { 222 },
        0x93 | 0x94 => if bold { 500 } else { 333 },
        0x95 => 350,
        0xB0 => 400,
        0xB7 => 278,
        0xC0..=0xC5 => base(b'A'),
        0xC6 | 0x8C => 1000,
        0xC7 => base(b'C'),
        0xC8..=0xCB => base(b'E'),
        0xCC..=0xCF => 278,
        0xD1 => base(b'N'),
        0xD2..=0xD6 | 0xD8 => base(b'O'),
        0xD9..=0xDC => base(b'U'),
        0xDD | 0x9F => base(b'Y'),
        0xDF => 611,
        0xE0..=0xE5 => base(b'a'),
        0xE6 => if bold { 889 } else { 889 },
        0x9C => 944,
        0xE7 => base(b'c'),
        0xE8..=0xEB => base(b'e'),
        0xEC..=0xEF => 278,
        0xF1 => base(b'n'),
        0xF2..=0xF6 | 0xF8 => base(b'o'),
        0xF9..=0xFC => base(b'u'),
        0xFD | 0xFF => base(b'y'),
        _ => 556,
    }
}

/// Largeur d'un texte en points.
pub fn width(s: &str, size: f32, bold: bool) -> f32 {
    encode(s).into_iter().map(|b| glyph_width(b, bold) as f32).sum::<f32>() * size / 1000.0
}

/// Texte raccourci avec « … » pour tenir dans `max` points.
pub fn truncate(s: &str, size: f32, bold: bool, max: f32) -> String {
    if width(s, size, bold) <= max {
        return s.to_string();
    }
    let ell = width("…", size, bold);
    let mut out = String::new();
    let mut w = 0.0;
    for c in s.chars() {
        let cw = width(c.encode_utf8(&mut [0; 4]), size, bold);
        if w + cw + ell > max {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.push('…');
    out
}

/// Chemin raccourci par le milieu (« C:\Users\…\fichier.txt »), plus parlant qu'une fin coupée.
pub fn truncate_middle(s: &str, size: f32, bold: bool, max: f32) -> String {
    if width(s, size, bold) <= max {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut a, mut b) = (chars.len() / 2, chars.len() / 2);
    loop {
        let cand: String = chars[..a].iter().chain(['…'].iter()).chain(chars[b..].iter()).collect();
        if width(&cand, size, bold) <= max || a == 0 {
            return cand;
        }
        // On retire alternativement au début et à la fin, en gardant plus de fin (le nom).
        if a * 2 > chars.len() - b {
            a -= 1;
        } else {
            b += 1;
            if b > chars.len() {
                return truncate(s, size, bold, max);
            }
        }
    }
}

/// Découpe un texte en lignes de `max` points au plus (coupures aux espaces).
pub fn wrap(s: &str, size: f32, bold: bool, max: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split(' ') {
        let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if width(&cand, size, bold) <= max {
            cur = cand;
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        // Mot plus long que la ligne (chemin…) : coupé où il le faut.
        for c in word.chars() {
            let mut next = cur.clone();
            next.push(c);
            if width(&next, size, bold) > max && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                next = c.to_string();
            }
            cur = next;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

// ---------------------------------------------------------------------------
// Assemblage du fichier
// ---------------------------------------------------------------------------

/// Chaîne PDF littérale (métadonnées), en UTF-16 avec marque d'ordre.
fn text_string(s: &str) -> String {
    let mut hex = String::from("<FEFF");
    for u in s.encode_utf16() {
        let _ = write!(hex, "{u:04X}");
    }
    hex.push('>');
    hex
}

/// Assemble les pages en un fichier PDF 1.4.
pub fn document(pages: &[Page], title: &str) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let count = 5 + 2 * pages.len();
    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    // 1 catalogue, 2 arbre des pages, 3-4 polices, 5 informations, puis (page, contenu) par page.
    obj(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 6 + 2 * i)).collect();
    obj(&mut out, &mut offsets, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).as_bytes());
    obj(&mut out, &mut offsets, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    obj(&mut out, &mut offsets, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>");
    let date = chrono::Local::now().format("D:%Y%m%d%H%M%S").to_string();
    obj(&mut out, &mut offsets, format!("<< /Title {} /Creator (DiskScan) /Producer (DiskScan) /CreationDate ({date}) >>", text_string(title)).as_bytes());
    for (i, p) in pages.iter().enumerate() {
        let page = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
            n(PAGE_W),
            n(PAGE_H),
            7 + 2 * i
        );
        obj(&mut out, &mut offsets, page.as_bytes());
        let mut stream = format!("<< /Length {} >>\nstream\n", p.ops.len()).into_bytes();
        stream.extend_from_slice(&p.ops);
        stream.extend_from_slice(b"\nendstream");
        obj(&mut out, &mut offsets, &stream);
    }
    debug_assert_eq!(offsets.len(), count);
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", count + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n", count + 1).as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodage() {
        assert_eq!(encode("Été à ça"), vec![0xC9, b't', 0xE9, b' ', 0xE0, b' ', 0xE7, 0x61]);
        assert_eq!(encode("€ … — −"), vec![0x80, b' ', 0x85, b' ', 0x97, b' ', b'-']);
        assert_eq!(encode("日"), vec![b'?']);
    }

    #[test]
    fn largeurs() {
        // « Hello » en Helvetica 10 pt : H 722 + e 556 + l 222 + l 222 + o 556 = 2 278.
        assert!((width("Hello", 10.0, false) - 22.78).abs() < 1e-3);
        assert!(width("Hello", 10.0, true) > width("Hello", 10.0, false));
        assert_eq!(width("é", 10.0, false), width("e", 10.0, false));
        let t = truncate("un texte assez long pour être coupé", 10.0, false, 60.0);
        assert!(t.ends_with('…') && width(&t, 10.0, false) <= 60.0);
        assert_eq!(truncate("court", 10.0, false, 60.0), "court");
        let m = truncate_middle(r"C:\Users\Utilisateur\AppData\Local\Google\fichier.img", 10.0, false, 120.0);
        assert!(m.contains('…') && m.ends_with("img") && width(&m, 10.0, false) <= 120.0, "{m}");
        let lines = wrap("un deux trois quatre cinq six sept huit neuf dix", 10.0, false, 60.0);
        assert!(lines.len() > 1 && lines.iter().all(|l| width(l, 10.0, false) <= 60.0));
        assert_eq!(lines.join(" "), "un deux trois quatre cinq six sept huit neuf dix");
        // Un chemin sans espace plus long que la ligne est coupé, sans rien perdre.
        let path = r"C:\Users\Utilisateur\AppData\Local\Temp\un\chemin\très\long\sans\espace";
        let lines = wrap(&format!("Voir {path} ici"), 10.0, false, 80.0);
        assert!(lines.len() > 2 && lines.iter().all(|l| width(l, 10.0, false) <= 80.0), "{lines:?}");
        assert_eq!(lines.concat().replace(' ', ""), format!("Voir{path}ici"));
    }

    /// Structure du fichier : chaque entrée de la table de références pointe sur son objet.
    #[test]
    fn structure() {
        let mut p1 = Page::default();
        p1.rect(10.0, 10.0, 100.0, 20.0, rgb(0x1E63D6));
        p1.text(20.0, 50.0, 12.0, true, rgb(0x000000), "Rapport d'analyse (é)");
        p1.ring_sector(200.0, 200.0, 50.0, 30.0, 0.0, 0.7, rgb(0xFF0000));
        let p2 = Page::default();
        let pdf = document(&[p1, p2], "Essai é");
        // Positions en octets : l'en-tête contient des octets binaires (non UTF-8).
        let find = |from: usize, pat: &[u8]| pdf[from..].windows(pat.len()).position(|w| w == pat).map(|i| i + from);
        let num = |at: usize| -> usize { pdf[at..].iter().take_while(|b| b.is_ascii_digit()).fold(0, |a, &b| a * 10 + (b - b'0') as usize) };
        assert!(pdf.starts_with(b"%PDF-1.4") && pdf.ends_with(b"%%EOF\n"));
        let start = num(find(0, b"startxref\n").unwrap() + 10);
        assert!(pdf[start..].starts_with(b"xref\n0 10\n0000000000 65535 f \n"));
        for k in 0..9 {
            let entry = start + 10 + 20 * (k + 1);
            let off = num(entry);
            assert!(pdf[off..].starts_with(format!("{} 0 obj", k + 1).as_bytes()), "objet {}", k + 1);
        }
        // Longueur des flux exacte.
        let i = find(0, b"/Length ").unwrap();
        let len = num(i + 8);
        let s = find(i, b"stream\n").unwrap() + 7;
        assert!(pdf[s + len..].starts_with(b"\nendstream"));
        assert_eq!(n(1.0), "1");
        assert_eq!(n(-0.001), "0");
        assert_eq!(n(2.5), "2.5");
    }
}
