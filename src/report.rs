//! Rapport PDF d'une analyse : chiffres clés, occupation du lecteur, résumé rédigé,
//! graphiques (premier niveau, extensions, ancienneté) et tableaux (plus gros fichiers,
//! évolution depuis l'analyse précédente).

use crate::format as fmt;
use crate::pdf::{self, rgb, Page, Rgb, PAGE_H, PAGE_W};
use crate::tree::{AgeStat, ExtStat, AGE_LABELS};

/// Élément du premier niveau du dossier analysé.
pub struct Item {
    pub name: String,
    pub size: u64,
    pub alloc: u64,
    pub is_dir: bool,
}

pub struct BigFile {
    pub path: String,
    pub size: u64,
    pub alloc: u64,
    pub mtime: i64,
}

/// Dossier dont la taille a changé depuis l'analyse précédente.
pub struct Change {
    pub path: String,
    pub before: i64,
    pub now: i64,
    pub delta: i64,
    pub new: bool,
}

pub struct Drive {
    pub label: String,
    pub total: u64,
    pub free: u64,
}

/// Tout ce que contient le rapport, préparé par l'application.
pub struct ReportData {
    pub root: String,
    pub when: i64,
    pub elapsed_secs: f32,
    pub cancelled: bool,
    /// Analyse rechargée : dossiers seulement.
    pub loaded: bool,
    /// Graphiques et classements en taille occupée sur le disque.
    pub on_disk: bool,
    pub size: u64,
    pub alloc: u64,
    pub files: u64,
    pub dirs: u64,
    pub denied: u64,
    pub drive: Option<Drive>,
    /// Trié du plus gros au plus petit.
    pub top_level: Vec<Item>,
    pub big_files: Vec<BigFile>,
    pub exts: Vec<ExtStat>,
    pub ages: [AgeStat; 6],
    pub compared: bool,
    /// Trié par écart absolu décroissant.
    pub changes: Vec<Change>,
}

impl ReportData {
    fn w(&self, size: u64, alloc: u64) -> u64 {
        if self.on_disk {
            alloc
        } else {
            size
        }
    }
    fn total(&self) -> u64 {
        self.w(self.size, self.alloc)
    }
    /// Chemin relatif au dossier analysé.
    fn rel(&self, path: &str) -> String {
        let root = self.root.trim_end_matches('\\');
        match path.get(..root.len()) {
            Some(h) if h.eq_ignore_ascii_case(root) => {
                let rest = path[root.len()..].trim_start_matches('\\');
                if rest.is_empty() {
                    "(dossier analysé)".into()
                } else {
                    rest.to_string()
                }
            }
            _ => path.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Mise en page
// ---------------------------------------------------------------------------

const M: f32 = 42.0;
const CW: f32 = PAGE_W - 2.0 * M;
const BOTTOM: f32 = PAGE_H - 54.0;

const TEXT: Rgb = rgb(0x15181C);
const TEXT2: Rgb = rgb(0x454C56);
const TEXT3: Rgb = rgb(0x69717C);
const ACCENT: Rgb = rgb(0x1E63D6);
const TRACK: Rgb = rgb(0xE3E7EC);
const SURFACE2: Rgb = rgb(0xF4F5F7);
const ZEBRA: Rgb = rgb(0xF8F9FB);
const BORDER: Rgb = rgb(0xE1E4E9);
const OK: Rgb = rgb(0x14773A);
const WARN: Rgb = rgb(0xB26B00);
const ERR: Rgb = rgb(0xC02626);
const SERIES: [Rgb; 10] = [
    rgb(0x1E63D6),
    rgb(0x14A38B),
    rgb(0xE59A2F),
    rgb(0x8E5CC9),
    rgb(0xD9534F),
    rgb(0x3BA3D0),
    rgb(0xB9A12C),
    rgb(0x5C9E3F),
    rgb(0xC4569B),
    rgb(0x7A8796),
];
/// Ancienneté : du bleu (récent) à l'orangé (ancien).
const AGE_COLORS: [Rgb; 6] = [rgb(0x1E63D6), rgb(0x4F86DD), rgb(0x8FB0E8), rgb(0xE9C27D), rgb(0xE59A2F), rgb(0xC9711A)];

struct Doc<'a> {
    d: &'a ReportData,
    pages: Vec<Page>,
    y: f32,
}

impl<'a> Doc<'a> {
    fn p(&mut self) -> &mut Page {
        self.pages.last_mut().expect("au moins une page")
    }

    /// Nouvelle page avec son en-tête.
    fn new_page(&mut self) {
        let mut p = Page::default();
        p.rect(0.0, 0.0, PAGE_W, 4.0, ACCENT);
        p.text(M, 30.0, 8.5, true, ACCENT, "DiskScan");
        p.text(M + pdf::width("DiskScan", 8.5, true) + 6.0, 30.0, 8.5, false, TEXT3, "Rapport d'analyse d'espace disque");
        let root = pdf::truncate_middle(&self.d.root, 8.5, false, CW * 0.5);
        p.text_right(M + CW, 30.0, 8.5, false, TEXT3, &root);
        p.line(M, 38.0, M + CW, 38.0, BORDER, 0.6);
        self.pages.push(p);
        self.y = 58.0;
    }

    /// Passe à la page suivante si `h` points ne tiennent plus.
    fn ensure(&mut self, h: f32) {
        if self.y + h > BOTTOM {
            self.new_page();
        }
    }

    /// Titre de section (et la place pour au moins `min_after` points de contenu).
    fn section(&mut self, title: &str, min_after: f32) {
        self.ensure(34.0 + min_after);
        self.y += 10.0;
        let y = self.y;
        let p = self.p();
        p.rect(M, y, 3.0, 15.0, ACCENT);
        p.text(M + 10.0, y + 12.0, 13.0, true, TEXT, title);
        self.y += 26.0;
    }

    /// Paragraphe sur toute la largeur.
    fn paragraph(&mut self, text: &str, size: f32, color: Rgb) {
        for line in pdf::wrap(text, size, false, CW) {
            self.ensure(size * 1.45);
            let y = self.y + size;
            self.p().text(M, y, size, false, color, &line);
            self.y += size * 1.45;
        }
    }

    /// Liste à puces.
    fn bullets(&mut self, items: &[String]) {
        for it in items {
            let lines = pdf::wrap(it, 10.0, false, CW - 14.0);
            self.ensure(14.0 * lines.len() as f32 + 4.0);
            let y0 = self.y;
            self.p().rect(M + 2.0, y0 + 5.5, 3.5, 3.5, ACCENT);
            for (k, line) in lines.iter().enumerate() {
                let y = y0 + 10.0 + 14.0 * k as f32;
                self.p().text(M + 13.0, y, 10.0, false, TEXT, line);
            }
            self.y += 14.0 * lines.len() as f32 + 5.0;
        }
    }

    /// Ligne de graphique en barres : pastille, libellé, détail, barre, taille, part.
    #[allow(clippy::too_many_arguments)]
    fn bar_row(&mut self, color: Rgb, label: &str, detail: &str, value: u64, max: u64, total: u64) {
        self.ensure(18.0);
        let y = self.y;
        let (bx, bw) = (M + 236.0, 150.0);
        let p = self.p();
        p.rect(M, y + 4.0, 8.0, 8.0, color);
        let lw = if detail.is_empty() { 200.0 } else { 140.0 };
        let label = pdf::truncate(label, 9.5, false, lw);
        p.text(M + 14.0, y + 11.5, 9.5, false, TEXT, &label);
        if !detail.is_empty() {
            let det = pdf::truncate(detail, 8.0, false, 226.0 - 14.0 - lw - 6.0);
            p.text_right(M + 226.0, y + 11.5, 8.0, false, TEXT3, &det);
        }
        p.rect(bx, y + 5.0, bw, 6.0, TRACK);
        if max > 0 && value > 0 {
            p.rect(bx, y + 5.0, (bw * value as f32 / max as f32).max(1.5), 6.0, color);
        }
        p.text_right(M + CW - 48.0, y + 11.5, 9.5, true, TEXT, &fmt::size(value));
        let pct = if total == 0 { 0.0 } else { value as f64 / total as f64 };
        p.text_right(M + CW, y + 11.5, 9.0, false, TEXT2, &fmt::percent(pct));
        self.y += 18.0;
    }

    /// Tableau simple : en-tête, lignes alternées. `cols` : (titre, largeur, aligné à droite).
    fn table(&mut self, cols: &[(&str, f32, bool)], rows: &[Vec<(String, Rgb, bool)>]) {
        let header = |doc: &mut Doc| {
            doc.ensure(34.0);
            let y = doc.y;
            let p = doc.p();
            p.rect(M, y, CW, 17.0, SURFACE2);
            let mut x = M + 6.0;
            for &(title, w, right) in cols {
                if right {
                    p.text_right(x + w - 6.0, y + 12.0, 8.5, true, TEXT2, title);
                } else {
                    p.text(x, y + 12.0, 8.5, true, TEXT2, title);
                }
                x += w;
            }
            doc.y += 17.0;
        };
        header(self);
        for (k, row) in rows.iter().enumerate() {
            if self.y + 16.0 > BOTTOM {
                self.new_page();
                header(self);
            }
            let y = self.y;
            let p = self.p();
            if k % 2 == 1 {
                p.rect(M, y, CW, 16.0, ZEBRA);
            }
            let mut x = M + 6.0;
            for (i, &(_, w, right)) in cols.iter().enumerate() {
                let (text, color, bold) = &row[i];
                let max = w - 10.0;
                let t = if i == 1 || text.contains('\\') { pdf::truncate_middle(text, 8.5, *bold, max) } else { pdf::truncate(text, 8.5, *bold, max) };
                if right {
                    p.text_right(x + w - 6.0, y + 11.5, 8.5, *bold, *color, &t);
                } else {
                    p.text(x, y + 11.5, 8.5, *bold, *color, &t);
                }
                x += w;
            }
            self.y += 16.0;
        }
        let y = self.y;
        self.p().line(M, y, M + CW, y, BORDER, 0.6);
        self.y += 4.0;
    }
}

// ---------------------------------------------------------------------------
// Contenu
// ---------------------------------------------------------------------------

/// Phrases du résumé, construites à partir des chiffres de l'analyse.
pub fn summary_sentences(d: &ReportData) -> Vec<String> {
    let mut out = Vec::new();
    let total = d.total();
    // Le chemin figure déjà en titre : on ne le répète que s'il est court (lecteur).
    let what = if d.drive.is_some() || d.root.len() <= 3 { format!("Le lecteur {}", d.root) } else { "Le dossier analysé".to_string() };
    out.push(format!(
        "{what} contient {} répartis dans {}, pour {} en taille réelle et {} sur le disque.",
        fmt::plural(d.files, "fichier", "fichiers"),
        fmt::plural(d.dirs, "dossier", "dossiers"),
        fmt::size(d.size),
        fmt::size(d.alloc)
    ));
    if let Some(dr) = &d.drive {
        let used = dr.total.saturating_sub(dr.free);
        let pct = used as f64 / dr.total.max(1) as f64;
        let mut s = format!("Le lecteur est occupé à {} : il reste {} libres sur {}.", fmt::percent(pct), fmt::size(dr.free), fmt::size(dr.total));
        if pct >= 0.95 {
            s.push_str(" Il est presque plein : libérer de l'espace est urgent.");
        } else if pct >= 0.85 {
            s.push_str(" Il commence à manquer de place.");
        }
        out.push(s);
    }
    if let Some(first) = d.top_level.first() {
        let w = d.w(first.size, first.alloc);
        if total > 0 && w > 0 {
            let kind = if first.is_dir { "Le dossier" } else { "Le fichier" };
            out.push(format!("{kind} « {} » est l'élément le plus volumineux du premier niveau : {}, soit {} du total.", first.name, fmt::size(w), fmt::percent(w as f64 / total as f64)));
        }
        let k = d.top_level.len().min(3);
        if k >= 2 && total > 0 {
            let sum: u64 = d.top_level[..k].iter().map(|i| d.w(i.size, i.alloc)).sum();
            out.push(format!("Les {k} plus gros éléments du premier niveau représentent {} de l'espace analysé.", fmt::percent(sum as f64 / total as f64)));
        }
    }
    if let Some(f) = d.big_files.first() {
        let (dir, name) = f.path.rsplit_once('\\').unwrap_or(("", f.path.as_str()));
        let rel = d.rel(dir);
        let place = if rel == "(dossier analysé)" { "à la racine du dossier analysé".to_string() } else { format!("dans {rel}") };
        out.push(format!("Le plus gros fichier est « {name} » ({}), {place}.", fmt::size(d.w(f.size, f.alloc))));
    }
    if let Some(e) = d.exts.first() {
        let w = d.w(e.size, e.alloc);
        if total > 0 && w > 0 {
            out.push(format!(
                "Le type de fichier qui occupe le plus de place est {} : {} ({}) en {}.",
                e.ext,
                fmt::size(w),
                fmt::percent(w as f64 / total as f64),
                fmt::plural(e.files, "fichier", "fichiers")
            ));
        }
    }
    let ages_total: u64 = d.ages.iter().map(|a| d.w(a.size, a.alloc)).sum();
    let old: u64 = d.ages[3..].iter().map(|a| d.w(a.size, a.alloc)).sum();
    if ages_total > 0 && old > 0 {
        out.push(format!(
            "{} des données ({}) n'ont pas été modifiées depuis plus d'un an : c'est souvent là qu'il y a le plus à archiver ou à supprimer.",
            fmt::percent(old as f64 / ages_total as f64),
            fmt::size(old)
        ));
    }
    if d.size > 0 && d.alloc < d.size / 10 * 9 {
        out.push(format!(
            "La taille sur le disque est inférieure de {} à la taille réelle : fichiers compressés, creux ou disponibles en ligne seulement.",
            fmt::size(d.size - d.alloc)
        ));
    }
    if d.denied > 0 {
        out.push(format!(
            "{} pas pu être {} (accès refusé) : {} contenu n'est pas compté.",
            fmt::plural(d.denied, "dossier n'a", "dossiers n'ont"),
            if d.denied > 1 { "lus" } else { "lu" },
            if d.denied > 1 { "leur" } else { "son" }
        ));
    }
    if d.compared {
        let (up, down, new) = d.changes.iter().fold((0u64, 0u64, 0u64), |(u, dn, n), c| if c.new { (u, dn, n + 1) } else if c.delta > 0 { (u + 1, dn, n) } else { (u, dn + 1, n) });
        if up + down + new == 0 {
            out.push("Aucun dossier n'a changé de taille depuis l'analyse précédente.".into());
        } else {
            let mut parts = Vec::new();
            if up > 0 {
                parts.push(fmt::plural(up, "dossier a grossi", "dossiers ont grossi"));
            }
            if down > 0 {
                parts.push(fmt::plural(down, "a diminué", "ont diminué"));
            }
            if new > 0 {
                parts.push(fmt::plural(new, "est nouveau", "sont nouveaux"));
            }
            let mut s = format!("Depuis l'analyse précédente, {}.", parts.join(", "));
            if let Some(c) = d.changes.iter().find(|c| !c.new && c.delta > 0) {
                s.push_str(&format!(" La plus forte hausse : {} ({}).", d.rel(&c.path), fmt::delta(c.delta)));
            }
            out.push(s);
        }
    }
    out
}

/// Page de garde : titre, cible, dates, chiffres clés.
fn cover(doc: &mut Doc, generated: i64) {
    let d = doc.d;
    let mut p = Page::default();
    p.rect(0.0, 0.0, PAGE_W, 6.0, ACCENT);
    // Logo : carré accent et disque.
    p.rect(M, 34.0, 26.0, 26.0, ACCENT);
    p.ring_sector(M + 13.0, 47.0, 8.0, 2.6, 0.0, 0.61, rgb(0xFFFFFF));
    p.ring_sector(M + 13.0, 47.0, 8.0, 2.6, 0.61, 1.0, rgb(0x8FB0E8));
    p.text(M + 34.0, 45.0, 11.0, true, TEXT, "DiskScan");
    p.text(M + 34.0, 57.0, 8.5, false, TEXT3, "Analyseur d'espace disque");
    p.text(M, 100.0, 24.0, true, TEXT, "Rapport d'analyse");
    let root = pdf::truncate_middle(&d.root, 14.0, true, CW);
    p.text(M, 124.0, 14.0, true, ACCENT, &root);
    let mut info = format!("Analyse du {} · durée {} s", fmt::date_time(d.when), fmt::secs(std::time::Duration::from_secs_f32(d.elapsed_secs.max(0.0))));
    info.push_str(&format!(" · rapport généré le {}", fmt::date_time(generated)));
    p.text(M, 142.0, 9.5, false, TEXT2, &info);
    let mut y = 150.0;
    let mut notes = Vec::new();
    if d.cancelled {
        notes.push("Analyse interrompue : les chiffres sont partiels.");
    }
    if d.loaded {
        notes.push("Analyse rechargée : le détail par fichier se limite aux plus gros fichiers et aux répartitions enregistrées.");
    }
    for note in notes {
        y += 14.0;
        p.text(M, y, 9.0, true, WARN, note);
    }
    y += 18.0;

    // Chiffres clés.
    let cw = (CW - 3.0 * 10.0) / 4.0;
    let denied = if d.denied > 0 { format!("dont {}", fmt::plural(d.denied, "illisible", "illisibles")) } else { String::new() };
    let cards = [
        ("Taille réelle", fmt::size(d.size), String::new()),
        ("Sur le disque", fmt::size(d.alloc), String::new()),
        ("Fichiers", fmt::thousands(d.files), String::new()),
        ("Dossiers", fmt::thousands(d.dirs), denied),
    ];
    for (i, (label, value, sub)) in cards.iter().enumerate() {
        let x = M + i as f32 * (cw + 10.0);
        p.rect(x, y, cw, 58.0, SURFACE2);
        p.text(x + 10.0, y + 17.0, 8.5, false, TEXT3, label);
        p.text(x + 10.0, y + 40.0, 17.0, true, TEXT, value);
        if !sub.is_empty() {
            p.text(x + 10.0, y + 52.0, 7.5, false, ERR, sub);
        }
    }
    doc.pages.push(p);
    doc.y = y + 58.0 + 8.0;
    let mode = if d.on_disk { "Graphiques et classements : taille occupée sur le disque." } else { "Graphiques et classements : taille réelle des fichiers." };
    doc.y += 4.0;
    let yy = doc.y + 9.0;
    doc.p().text(M, yy, 8.5, false, TEXT3, mode);
    doc.y += 14.0;
}

/// Anneau d'occupation du lecteur et sa légende.
fn drive_section(doc: &mut Doc) {
    let d = doc.d;
    let Some(dr) = &d.drive else { return };
    doc.section(&format!("Occupation du lecteur {}", dr.label), 130.0);
    let used = dr.total.saturating_sub(dr.free);
    let pct = used as f64 / dr.total.max(1) as f64;
    let color = if pct >= 0.95 {
        ERR
    } else if pct >= 0.85 {
        rgb(0xE59A2F)
    } else {
        ACCENT
    };
    let (cx, cy) = (M + 62.0, doc.y + 58.0);
    let y0 = doc.y;
    let p = doc.p();
    p.ring_sector(cx, cy, 56.0, 36.0, 0.0, 1.0, TRACK);
    p.ring_sector(cx, cy, 56.0, 36.0, 0.0, pct as f32, color);
    p.text_center(cx, cy + 4.0, 15.0, true, TEXT, &fmt::percent(pct));
    p.text_center(cx, cy + 16.0, 8.0, false, TEXT3, "occupé");
    // Légende.
    let found = d.alloc.min(used);
    let hidden = used.saturating_sub(d.alloc);
    let mut rows: Vec<(Option<Rgb>, &str, String)> = vec![
        (Some(color), "Espace occupé", format!("{} ({})", fmt::size(used), fmt::percent(pct))),
        (Some(TRACK), "Espace libre", fmt::size(dr.free)),
        (None, "Capacité", fmt::size(dr.total)),
        (None, "Trouvé par l'analyse (sur le disque)", fmt::size(found)),
    ];
    if hidden > used / 100 {
        rows.push((None, "Non visible par l'analyse", fmt::size(hidden)));
    }
    let lx = M + 150.0;
    let mut y = y0 + 14.0;
    for (sw, label, value) in &rows {
        if let Some(c) = sw {
            p.rect(lx, y - 8.0, 9.0, 9.0, *c);
        }
        p.text(lx + 16.0, y, 10.0, false, TEXT2, label);
        p.text_right(M + CW, y, 10.0, true, TEXT, value);
        p.line(lx, y + 6.0, M + CW, y + 6.0, BORDER, 0.5);
        y += 21.0;
    }
    if hidden > used / 100 {
        let note = "Non visible : fichiers système protégés, clichés instantanés (points de restauration) et métadonnées NTFS, illisibles sans droits administrateur.";
        for (k, line) in pdf::wrap(note, 8.0, false, M + CW - lx).iter().enumerate() {
            p.text(lx, y + 2.0 + 10.0 * k as f32, 8.0, false, TEXT3, line);
        }
        y += 22.0;
    }
    doc.y = (y0 + 124.0).max(y + 4.0);
}

fn top_level_section(doc: &mut Doc) {
    let d = doc.d;
    if d.top_level.is_empty() {
        return;
    }
    doc.section("Répartition au premier niveau", 80.0);
    let total = d.total();
    // Barre empilée : les 9 plus gros et le reste.
    let y = doc.y;
    let mut x = M;
    let shown = d.top_level.len().min(9);
    let rest: u64 = d.top_level[shown..].iter().map(|i| d.w(i.size, i.alloc)).sum();
    {
        let p = doc.p();
        p.rect(M, y, CW, 16.0, TRACK);
        if total > 0 {
            for (k, it) in d.top_level[..shown].iter().enumerate() {
                let w = CW * d.w(it.size, it.alloc) as f32 / total as f32;
                p.rect(x, y, w, 16.0, SERIES[k]);
                x += w;
            }
            p.rect(x, y, CW * rest as f32 / total as f32, 16.0, rgb(0xB8C0CB));
        }
    }
    doc.y += 26.0;
    let max = d.top_level.first().map_or(0, |i| d.w(i.size, i.alloc));
    for (k, it) in d.top_level.iter().take(12).enumerate() {
        let color = if k < shown { SERIES[k] } else { rgb(0xB8C0CB) };
        let detail = if it.is_dir { "" } else { "fichier" };
        doc.bar_row(color, &it.name, detail, d.w(it.size, it.alloc), max, total);
    }
    if d.top_level.len() > 12 {
        let others: u64 = d.top_level[12..].iter().map(|i| d.w(i.size, i.alloc)).sum();
        let text = format!("… et {} ({}).", fmt::plural((d.top_level.len() - 12) as u64, "autre élément", "autres éléments"), fmt::size(others));
        doc.paragraph(&text, 8.5, TEXT3);
    }
}

fn ext_section(doc: &mut Doc) {
    let d = doc.d;
    if d.exts.is_empty() {
        return;
    }
    doc.section("Types de fichiers (extensions)", 80.0);
    let total: u64 = d.exts.iter().map(|e| d.w(e.size, e.alloc)).sum();
    let max = d.exts.iter().map(|e| d.w(e.size, e.alloc)).max().unwrap_or(0);
    let mut exts: Vec<&ExtStat> = d.exts.iter().collect();
    exts.sort_by(|a, b| d.w(b.size, b.alloc).cmp(&d.w(a.size, a.alloc)));
    for (k, e) in exts.iter().take(12).enumerate() {
        doc.bar_row(SERIES[k % 10], &e.ext, &fmt::plural(e.files, "fichier", "fichiers"), d.w(e.size, e.alloc), max, total);
    }
    if exts.len() > 12 {
        let others: u64 = exts[12..].iter().map(|e| d.w(e.size, e.alloc)).sum();
        let text = format!("… et {} ({}).", fmt::plural((exts.len() - 12) as u64, "autre extension", "autres extensions"), fmt::size(others));
        doc.paragraph(&text, 8.5, TEXT3);
    }
}

/// Colonnes par tranche d'ancienneté.
fn age_section(doc: &mut Doc) {
    let d = doc.d;
    let values: Vec<u64> = d.ages.iter().map(|a| d.w(a.size, a.alloc)).collect();
    let total: u64 = values.iter().sum();
    if total == 0 {
        return;
    }
    doc.section("Ancienneté des fichiers (dernière modification)", 185.0);
    let max = *values.iter().max().unwrap_or(&1);
    let (top, h) = (doc.y + 22.0, 110.0);
    let col = CW / 6.0;
    let p = doc.p();
    p.line(M, top + h, M + CW, top + h, BORDER, 0.8);
    for (i, v) in values.iter().enumerate() {
        let cx = M + col * (i as f32 + 0.5);
        let bh = if max == 0 { 0.0 } else { (h * *v as f32 / max as f32).max(if *v > 0 { 1.5 } else { 0.0 }) };
        p.rect(cx - 22.0, top + h - bh, 44.0, bh, AGE_COLORS[i]);
        p.text_center(cx, top + h - bh - 14.0, 9.0, true, TEXT, &fmt::size(*v));
        p.text_center(cx, top + h - bh - 4.0, 7.5, false, TEXT3, &fmt::percent(*v as f64 / total as f64));
        p.text_center(cx, top + h + 13.0, 8.5, false, TEXT2, AGE_LABELS[i]);
        p.text_center(cx, top + h + 24.0, 7.5, false, TEXT3, &fmt::plural(d.ages[i].files, "fichier", "fichiers"));
    }
    doc.y = top + h + 34.0;
}

fn big_files_section(doc: &mut Doc) {
    let d = doc.d;
    if d.big_files.is_empty() {
        return;
    }
    let n = d.big_files.len().min(25);
    doc.section(&format!("Les {n} plus gros fichiers"), 60.0);
    let cols = [("#", 22.0, false), ("Fichier", 170.0, false), ("Dossier", CW - 22.0 - 170.0 - 72.0 - 64.0, false), ("Taille", 72.0, true), ("Modifié le", 64.0, false)];
    let rows: Vec<Vec<(String, Rgb, bool)>> = d
        .big_files
        .iter()
        .take(n)
        .enumerate()
        .map(|(k, f)| {
            let (dir, name) = f.path.rsplit_once('\\').unwrap_or(("", f.path.as_str()));
            vec![
                ((k + 1).to_string(), TEXT3, false),
                (name.to_string(), TEXT, true),
                (d.rel(dir), TEXT2, false),
                (fmt::size(d.w(f.size, f.alloc)), TEXT, true),
                (fmt::date(f.mtime), TEXT2, false),
            ]
        })
        .collect();
    doc.table(&cols, &rows);
}

fn changes_section(doc: &mut Doc) {
    let d = doc.d;
    doc.section("Évolution depuis l'analyse précédente", 40.0);
    if !d.compared {
        doc.paragraph("Pas d'analyse précédente de cette cible : relancez l'analyse plus tard pour voir quels dossiers grossissent ou diminuent.", 9.5, TEXT2);
        return;
    }
    if d.changes.is_empty() {
        doc.paragraph("Aucun dossier n'a changé de taille depuis l'analyse précédente.", 9.5, TEXT2);
        return;
    }
    let n = d.changes.len().min(20);
    let cols = [("Dossier", CW - 80.0 - 80.0 - 90.0, false), ("Avant", 80.0, true), ("Maintenant", 80.0, true), ("Écart", 90.0, true)];
    let rows: Vec<Vec<(String, Rgb, bool)>> = d
        .changes
        .iter()
        .take(n)
        .map(|c| {
            let (delta, color) = if c.new {
                ("nouveau".to_string(), ACCENT)
            } else if c.delta > 0 {
                (fmt::delta(c.delta), WARN)
            } else {
                (fmt::delta(c.delta), OK)
            };
            vec![
                (d.rel(&c.path), TEXT, false),
                (if c.new { "—".into() } else { fmt::size(c.before.max(0) as u64) }, TEXT2, false),
                (fmt::size(c.now.max(0) as u64), TEXT2, false),
                (delta, color, true),
            ]
        })
        .collect();
    doc.table(&cols, &rows);
    if d.changes.len() > n {
        doc.paragraph(&format!("… et {} (liste complète dans l'onglet Évolution).", fmt::plural((d.changes.len() - n) as u64, "autre dossier", "autres dossiers")), 8.5, TEXT3);
    }
}

fn notes_section(doc: &mut Doc) {
    doc.section("Méthode", 50.0);
    let notes = [
        "Taille réelle : somme des tailles des fichiers. Taille sur le disque : espace effectivement réservé (clusters), plus faible pour les fichiers compressés, creux ou disponibles en ligne seulement.",
        "Les liens symboliques et jonctions sont listés mais jamais suivis ; un fichier présent sous plusieurs noms (lien physique) n'est compté qu'une fois.",
        "Les dossiers illisibles (accès refusé) sont signalés et leur contenu n'est pas compté. L'analyse ne demande aucun droit administrateur.",
    ];
    for n in notes {
        doc.paragraph(n, 8.5, TEXT3);
        doc.y += 3.0;
    }
}

/// Construit le rapport PDF complet.
pub fn render(d: &ReportData, generated: i64) -> Vec<u8> {
    let mut doc = Doc { d, pages: Vec::new(), y: 0.0 };
    cover(&mut doc, generated);
    drive_section(&mut doc);
    doc.section("Résumé", 60.0);
    let sentences = summary_sentences(d);
    doc.bullets(&sentences);
    top_level_section(&mut doc);
    ext_section(&mut doc);
    age_section(&mut doc);
    big_files_section(&mut doc);
    changes_section(&mut doc);
    notes_section(&mut doc);

    // Pied de page, une fois le nombre de pages connu.
    let total = doc.pages.len();
    let foot = format!("Rapport généré par DiskScan le {}", fmt::date_time(generated));
    for (i, p) in doc.pages.iter_mut().enumerate() {
        p.line(M, PAGE_H - 36.0, M + CW, PAGE_H - 36.0, BORDER, 0.6);
        p.text(M, PAGE_H - 24.0, 8.0, false, TEXT3, &foot);
        p.text_right(M + CW, PAGE_H - 24.0, 8.0, false, TEXT3, &format!("Page {} / {total}", i + 1));
    }
    pdf::document(&doc.pages, &format!("DiskScan — {}", d.root))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn sample() -> ReportData {
        let item = |name: &str, size: u64, is_dir: bool| Item { name: name.into(), size, alloc: size / 2, is_dir };
        ReportData {
            root: r"C:\".into(),
            when: 1_700_000_000,
            elapsed_secs: 19.9,
            cancelled: false,
            loaded: false,
            on_disk: false,
            size: 700 << 30,
            alloc: 420 << 30,
            files: 1_386_284,
            dirs: 233_371,
            denied: 351,
            drive: Some(Drive { label: "Disque local (C:)".into(), total: 464 << 30, free: 30 << 30 }),
            top_level: (0..15).map(|k| item(&format!("Dossier {k}"), (600 >> k.min(9)) << 28, k != 3)).collect(),
            big_files: (0..30).map(|k| BigFile { path: format!(r"C:\Users\Utilisateur\AppData\Local\Google\Play Games\fichier_{k}.img"), size: (80 - k) << 30, alloc: 6 << 30, mtime: 1_700_000_000 }).collect(),
            exts: (0..20).map(|k| ExtStat { ext: format!(".e{k}"), files: 100 - k, size: (200 - 9 * k) << 28, alloc: (100 - 4 * k) << 28 }).collect(),
            ages: [
                AgeStat { files: 10, size: 50 << 30, alloc: 20 << 30 },
                AgeStat { files: 20, size: 80 << 30, alloc: 70 << 30 },
                AgeStat { files: 30, size: 120 << 30, alloc: 100 << 30 },
                AgeStat { files: 40, size: 200 << 30, alloc: 150 << 30 },
                AgeStat { files: 50, size: 180 << 30, alloc: 60 << 30 },
                AgeStat { files: 60, size: 70 << 30, alloc: 20 << 30 },
            ],
            compared: true,
            changes: vec![
                Change { path: r"C:\Users".into(), before: 500 << 30, now: 516 << 30, delta: 16 << 30, new: false },
                Change { path: r"C:\Temp".into(), before: 9 << 30, now: 1 << 30, delta: -(8 << 30), new: false },
                Change { path: r"C:\Nouveau".into(), before: 0, now: 2 << 30, delta: 2 << 30, new: true },
            ],
        }
    }

    #[test]
    fn resume() {
        let d = sample();
        let s = summary_sentences(&d);
        assert!(s[0].starts_with("Le lecteur C:\\ contient 1\u{a0}386\u{a0}284 fichiers"), "{}", s[0]);
        let mut folder = sample();
        folder.drive = None;
        folder.root = r"C:\Users\Utilisateur\Documents".into();
        folder.big_files[0].path = r"C:\Users\Utilisateur\Documents\gros.iso".into();
        let s2 = summary_sentences(&folder);
        assert!(s2[0].starts_with("Le dossier analysé contient"), "{}", s2[0]);
        assert!(s2.iter().any(|x| x.contains("« gros.iso »") && x.contains("à la racine du dossier analysé")));
        assert!(s.iter().any(|x| x.contains("Le lecteur est occupé à 93,5") && x.contains("commence à manquer de place")));
        assert!(s.iter().any(|x| x.contains("« Dossier 0 »")));
        assert!(s.iter().any(|x| x.contains("« fichier_0.img »") && x.contains(r"Users\Utilisateur\AppData")));
        assert!(s.iter().any(|x| x.contains("351 dossiers n'ont pas pu être lus")));
        assert!(s.iter().any(|x| x.contains("1 dossier a grossi, 1 a diminué, 1 est nouveau") && x.contains("La plus forte hausse : Users (+16,0\u{a0}Go)")));
        assert!(s.iter().any(|x| x.contains("depuis plus d'un an")));
        // Un seul dossier illisible : accords au singulier.
        let mut d1 = sample();
        d1.denied = 1;
        assert!(summary_sentences(&d1).iter().any(|x| x.contains("1 dossier n'a pas pu être lu (accès refusé) : son contenu")));
    }

    #[test]
    fn rendu() {
        let d = sample();
        let pdf = render(&d, 1_700_000_100);
        assert!(pdf.starts_with(b"%PDF-1.4") && pdf.ends_with(b"%%EOF\n"));
        let pages = pdf.windows(11).filter(|w| w == b"/Type /Page").count() - 1;
        assert!((2..=6).contains(&pages), "{pages} pages");
        // Sans lecteur, sans comparaison, rechargé : le rapport se construit aussi.
        let mut d = sample();
        d.drive = None;
        d.compared = false;
        d.loaded = true;
        d.big_files.clear();
        d.top_level.clear();
        assert!(render(&d, 1).starts_with(b"%PDF"));
    }
}
