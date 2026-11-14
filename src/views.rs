//! Zone principale : cartes de statistiques, progression, onglets, barre d'outils,
//! arborescence, carte proportionnelle, tableaux simples, panneau de détails,
//! menu contextuel et confirmation d'envoi à la corbeille.

use crate::app::{Act, DiskScanApp, Row, ScanEnd, Tab, TreeSort, MAX_HITS};
use crate::format as fmt;
use crate::icons::{self, Icon};
use crate::theme::{mono, regular, semibold, stroke, Palette};
use crate::tree::{cmp_names, flag, Node, NodeId, SortKey, Tree, AGE_LABELS, NONE};
use crate::treemap;
use crate::widgets::{self as w, Button, Input, Kind};
use eframe::egui::{self, pos2, vec2, Align, Align2, Color32, Frame, Label, Layout, Margin, Rect, RichText, Rounding, Sense, Ui, UiBuilder, Vec2};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DETAILS_W: f32 = 316.0;
const ROW_H: f32 = 30.0;

// ---------------------------------------------------------------------------
// Colonnes de l'arborescence
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
pub enum ColKind {
    Name,
    Size,
    Percent,
    Files,
    Dirs,
    Modified,
    Delta,
}

impl ColKind {
    /// Ordre par défaut, modifiable par glisser-déposer des en-têtes.
    pub const DEFAULT_ORDER: [ColKind; 7] = [ColKind::Name, ColKind::Size, ColKind::Percent, ColKind::Files, ColKind::Dirs, ColKind::Modified, ColKind::Delta];

    fn title(self) -> &'static str {
        match self {
            ColKind::Name => "Nom",
            ColKind::Size => "Taille",
            ColKind::Percent => "Part du parent",
            ColKind::Files => "Fichiers",
            ColKind::Dirs => "Dossiers",
            ColKind::Modified => "Modifié le",
            ColKind::Delta => "Évolution",
        }
    }
    fn sort_key(self) -> TreeSort {
        match self {
            ColKind::Name => TreeSort::Name,
            ColKind::Size | ColKind::Percent => TreeSort::Size,
            ColKind::Files => TreeSort::Files,
            ColKind::Dirs => TreeSort::Dirs,
            ColKind::Modified => TreeSort::Modified,
            ColKind::Delta => TreeSort::Delta,
        }
    }
    /// Colonne de nombres, alignée à droite.
    fn right(self) -> bool {
        matches!(self, ColKind::Size | ColKind::Files | ColKind::Dirs | ColKind::Delta)
    }
}

struct Col {
    kind: ColKind,
    x: f32,
    w: f32,
}

/// Colonnes dans l'ordre choisi. Si la place manque, on retire les dossiers, puis la date,
/// puis les fichiers, en resserrant la barre de pourcentage ; le nom prend la largeur restante.
fn layout_columns(left: f32, width: f32, order: &[ColKind], show_delta: bool) -> Vec<Col> {
    use ColKind::*;
    const TIERS: [(&[ColKind], f32, f32); 4] = [(&[], 150.0, 240.0), (&[Dirs], 130.0, 210.0), (&[Dirs, Modified], 110.0, 180.0), (&[Dirs, Modified, Files], 100.0, 0.0)];
    let mut cols = Vec::new();
    for (hidden, pct_w, min_name) in TIERS {
        let defs: Vec<(ColKind, f32)> = order
            .iter()
            .filter(|k| !hidden.contains(k) && (show_delta || **k != Delta))
            .map(|&k| {
                let cw = match k {
                    Name => 0.0,
                    Size => 84.0,
                    Percent => pct_w,
                    Files | Dirs => 72.0,
                    Modified => 84.0,
                    Delta => 96.0,
                };
                (k, cw)
            })
            .collect();
        let fixed: f32 = defs.iter().map(|d| d.1).sum();
        let flex = width - 24.0 - 14.0 * (defs.len() as f32 - 1.0) - fixed;
        let mut x = left + 12.0;
        cols.clear();
        for (kind, cw) in defs {
            let cw = if kind == Name { flex.max(80.0) } else { cw };
            cols.push(Col { kind, x, w: cw });
            x += cw + 14.0;
        }
        if flex >= min_name {
            break;
        }
    }
    cols
}

// ---------------------------------------------------------------------------
// Tableaux simples (plus gros fichiers, extensions, ancienneté, évolution)
// ---------------------------------------------------------------------------

pub struct FlatRow {
    pub cells: Vec<String>,
    /// Valeur de tri numérique de chaque colonne (`None` : tri par le texte).
    pub nums: Vec<Option<i64>>,
    /// Champs exportés (tailles en octets, dates complètes).
    pub export: Vec<String>,
    /// Nœud de l'arbre (`NONE` s'il n'y en a pas).
    pub node: NodeId,
    /// Chemin pour les actions (vide : pas de menu).
    pub path: String,
    /// Part du total (colonne à barre).
    pub frac: f32,
    /// Écart : 1 a grossi, -1 a diminué, 2 nouveau, 0 sinon.
    pub tone: i8,
}

struct FlatCol {
    title: &'static str,
    /// Largeur fixe ; négative : part de la largeur restante.
    w: f32,
    right: bool,
    mono: bool,
    bar: bool,
    tone: bool,
}

const fn fc(title: &'static str, w: f32, right: bool, mono: bool, bar: bool, tone: bool) -> FlatCol {
    FlatCol { title, w, right, mono, bar, tone }
}

static FILES_COLS: [FlatCol; 4] = [fc("Nom", -1.0, false, false, false, false), fc("Dans le dossier", -1.3, false, true, false, false), fc("Taille", 90.0, true, false, false, false), fc("Modifié le", 90.0, false, false, false, false)];
static EXTS_COLS: [FlatCol; 4] = [fc("Extension", -1.0, false, true, false, false), fc("Fichiers", 90.0, true, false, false, false), fc("Taille", 100.0, true, false, false, false), fc("Part du total", 200.0, false, false, true, false)];
static AGES_COLS: [FlatCol; 4] = [fc("Dernière modification", -1.0, false, false, false, false), fc("Fichiers", 90.0, true, false, false, false), fc("Taille", 100.0, true, false, false, false), fc("Part du total", 200.0, false, false, true, false)];
static CHANGES_COLS: [FlatCol; 4] = [fc("Dossier", -1.0, false, true, false, false), fc("Avant", 100.0, true, false, false, false), fc("Maintenant", 100.0, true, false, false, false), fc("Écart", 110.0, true, false, false, true)];

fn flat_cols(tab: Tab) -> &'static [FlatCol] {
    match tab {
        Tab::Files => &FILES_COLS,
        Tab::Exts => &EXTS_COLS,
        Tab::Ages => &AGES_COLS,
        _ => &CHANGES_COLS,
    }
}

fn flat_export_headers(tab: Tab) -> &'static [&'static str] {
    match tab {
        Tab::Files => &["Chemin", "Taille (octets)", "Sur le disque (octets)", "Modifié le"],
        Tab::Exts => &["Extension", "Fichiers", "Taille (octets)", "Sur le disque (octets)", "Part du total (%)"],
        Tab::Ages => &["Dernière modification", "Fichiers", "Taille (octets)", "Sur le disque (octets)", "Part du total (%)"],
        _ => &["Dossier", "Avant (octets)", "Maintenant (octets)", "Écart (octets)", "Nouveau"],
    }
}

const TREE_EXPORT: [&str; 10] = ["Chemin", "Type", "Taille (octets)", "Sur le disque (octets)", "Taille", "Part du parent (%)", "Fichiers", "Dossiers", "Modifié le", "Écart (octets)"];

/// Positions (x, largeur) des colonnes d'un tableau simple.
fn flat_layout(left: f32, width: f32, defs: &[FlatCol]) -> Vec<(f32, f32)> {
    let gap = 14.0;
    let fixed: f32 = defs.iter().filter(|c| c.w > 0.0).map(|c| c.w).sum();
    let parts: f32 = defs.iter().filter(|c| c.w < 0.0).map(|c| -c.w).sum();
    let flex = (width - 24.0 - gap * (defs.len() as f32 - 1.0) - fixed).max(60.0 * parts);
    let mut x = left + 12.0;
    defs.iter()
        .map(|c| {
            let cw = if c.w > 0.0 { c.w } else { flex * -c.w / parts };
            let r = (x, cw);
            x += cw + gap;
            r
        })
        .collect()
}

fn pct_number(frac: f64) -> String {
    format!("{:.1}", frac * 100.0).replace('.', ",")
}

fn kind_label(n: &Node) -> &'static str {
    if n.has(flag::GROUP) {
        "Fichiers regroupés"
    } else if n.has(flag::LINK) && n.is_dir() {
        "Jonction ou lien"
    } else if n.has(flag::LINK) {
        "Lien symbolique"
    } else if n.has(flag::DENIED) {
        "Dossier illisible"
    } else if n.is_dir() {
        "Dossier"
    } else {
        "Fichier"
    }
}

/// Teinte d'une case de la carte (fichiers, selon l'extension).
fn hue(p: &Palette, i: u8) -> Color32 {
    const HUES: [u32; 8] = [0x8FB3EA, 0x93CFA8, 0xEDC284, 0xC8A6DD, 0xE9A3A3, 0x86CDCB, 0xDDD07E, 0xB3BCCB];
    let c = HUES[i as usize % 8];
    let c = Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    if p.dark {
        w::lerp_color(c, Color32::from_rgb(0x10, 0x12, 0x15), 0.45)
    } else {
        c
    }
}

/// Zone dessinée de la carte, dans la carte (cadre) de la zone principale.
fn map_area(rect: Rect) -> Rect {
    Rect::from_min_max(pos2(rect.left() + 10.0, rect.top() + 44.0), pos2(rect.right() - 10.0, rect.bottom() - 10.0))
}

impl DiskScanApp {
    /// Le nœud existe-t-il encore dans l'arbre (pas envoyé à la corbeille) ?
    pub fn valid(&self, id: NodeId) -> bool {
        (id as usize) < self.tree.nodes.len() && self.tree.is_under(id, Tree::ROOT)
    }

    /// Chemin relatif au dossier analysé (« Projets\app »), plus lisible dans les tableaux.
    fn relative(&self, path: &str) -> String {
        let root = self.tree.root_path.trim_end_matches('\\');
        match path.get(..root.len()) {
            Some(head) if head.eq_ignore_ascii_case(root) => {
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

    fn display_name(&self, id: NodeId) -> String {
        let n = self.tree.node(id);
        if id == Tree::ROOT {
            self.tree.root_path.clone()
        } else if n.has(flag::GROUP) {
            format!("{} (sans détail)", fmt::plural(n.files as u64, "fichier", "fichiers"))
        } else {
            n.name.to_string()
        }
    }

    /// Champs d'une ligne de l'arborescence, pour la copie et l'export.
    pub fn node_fields(&self, id: NodeId) -> Vec<String> {
        let n = self.tree.node(id);
        let d = self.s.on_disk;
        let delta = self.deltas.get(&id).map_or(String::new(), |x| x.get(d).to_string());
        vec![
            self.tree.path(id),
            kind_label(n).to_string(),
            if n.has(flag::DUP) { "0".into() } else { n.size.to_string() },
            if n.has(flag::DUP) { "0".into() } else { n.alloc.to_string() },
            fmt::size(n.weight(d)),
            pct_number(self.share(id)),
            n.files.to_string(),
            if n.is_dir() { n.dirs.to_string() } else { String::new() },
            fmt::date_iso(n.mtime),
            delta,
        ]
    }

    /// En-têtes et lignes du tableau affiché, pour l'export CSV et la copie.
    pub fn export_table(&self) -> (Vec<&'static str>, Vec<Vec<String>>) {
        match self.s.tab {
            Tab::Tree => (TREE_EXPORT.to_vec(), self.rows.iter().map(|r| self.node_fields(r.id)).collect()),
            Tab::Map => {
                let mut ids = vec![self.map_root];
                ids.extend(self.tree.sorted_children(self.map_root, SortKey::Size, false, self.s.on_disk, true));
                (TREE_EXPORT.to_vec(), ids.into_iter().map(|id| self.node_fields(id)).collect())
            }
            tab => (flat_export_headers(tab).to_vec(), self.flat.iter().map(|r| r.export.clone()).collect()),
        }
    }

    // -- Calcul des tableaux simples ------------------------------------------

    fn filter_applies(&self) -> bool {
        matches!(self.s.tab, Tab::Tree | Tab::Files | Tab::Exts | Tab::Changes)
    }

    pub fn ensure_flat(&mut self) {
        let tab = self.s.tab;
        if !matches!(tab, Tab::Files | Tab::Exts | Tab::Ages | Tab::Changes) {
            return;
        }
        let (col, asc) = self.flat_sort.get(&tab).copied().unwrap_or((usize::MAX, false));
        let filter = if self.filter_applies() { self.filter.trim().to_lowercase() } else { String::new() };
        let key = (tab, self.data_rev, filter, col, asc, self.s.on_disk);
        if self.flat_key.as_ref() == Some(&key) {
            return;
        }
        let mut rows = self.build_flat(tab);
        if !key.2.is_empty() {
            rows.retain(|r| r.cells.iter().any(|c| c.to_lowercase().contains(&key.2)));
        }
        if col != usize::MAX {
            rows.sort_by(|a, b| {
                let o = match (a.nums.get(col).copied().flatten(), b.nums.get(col).copied().flatten()) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    _ => cmp_names(&a.cells[col], &b.cells[col]),
                };
                if asc {
                    o
                } else {
                    o.reverse()
                }
            });
        }
        self.flat = rows;
        self.flat_key = Some(key);
    }

    /// Lignes d'un tableau simple, dans leur ordre naturel.
    fn build_flat(&self, tab: Tab) -> Vec<FlatRow> {
        if !self.has_data {
            return Vec::new();
        }
        let d = self.s.on_disk;
        let pick = |size: u64, alloc: u64| if d { alloc } else { size };
        match tab {
            Tab::Files => {
                let mut rows: Vec<FlatRow> = self
                    .summary
                    .top
                    .iter()
                    .map(|f| {
                        let wt = pick(f.size, f.alloc);
                        let (dir, name) = f.path.rsplit_once('\\').unwrap_or(("", f.path.as_str()));
                        let dir = self.relative(dir);
                        let node = if f.node != NONE && self.valid(f.node) { f.node } else { NONE };
                        FlatRow {
                            cells: vec![name.to_string(), dir, fmt::size(wt), fmt::date(f.mtime)],
                            nums: vec![None, None, Some(wt as i64), Some(f.mtime)],
                            export: vec![f.path.clone(), f.size.to_string(), f.alloc.to_string(), fmt::date_iso(f.mtime)],
                            node,
                            path: f.path.clone(),
                            frac: 0.0,
                            tone: 0,
                        }
                    })
                    .collect();
                rows.sort_by(|a, b| b.nums[2].cmp(&a.nums[2]));
                rows
            }
            Tab::Exts => {
                let total: u64 = self.summary.exts.iter().map(|e| pick(e.size, e.alloc)).sum();
                let mut rows: Vec<FlatRow> = self
                    .summary
                    .exts
                    .iter()
                    .map(|e| {
                        let wt = pick(e.size, e.alloc);
                        let frac = if total == 0 { 0.0 } else { wt as f64 / total as f64 };
                        FlatRow {
                            cells: vec![e.ext.clone(), fmt::thousands(e.files), fmt::size(wt), fmt::percent(frac)],
                            nums: vec![None, Some(e.files as i64), Some(wt as i64), Some(wt as i64)],
                            export: vec![e.ext.clone(), e.files.to_string(), e.size.to_string(), e.alloc.to_string(), pct_number(frac)],
                            node: NONE,
                            path: String::new(),
                            frac: frac as f32,
                            tone: 0,
                        }
                    })
                    .collect();
                rows.sort_by(|a, b| b.nums[2].cmp(&a.nums[2]));
                rows
            }
            Tab::Ages => {
                let total: u64 = self.summary.ages.iter().map(|a| pick(a.size, a.alloc)).sum();
                self.summary
                    .ages
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let wt = pick(a.size, a.alloc);
                        let frac = if total == 0 { 0.0 } else { wt as f64 / total as f64 };
                        FlatRow {
                            cells: vec![AGE_LABELS[i].to_string(), fmt::thousands(a.files), fmt::size(wt), fmt::percent(frac)],
                            nums: vec![Some(i as i64), Some(a.files as i64), Some(wt as i64), Some(wt as i64)],
                            export: vec![AGE_LABELS[i].to_string(), a.files.to_string(), a.size.to_string(), a.alloc.to_string(), pct_number(frac)],
                            node: NONE,
                            path: String::new(),
                            frac: frac as f32,
                            tone: 0,
                        }
                    })
                    .collect()
            }
            _ => {
                let mut rows: Vec<(i64, FlatRow)> = self
                    .deltas
                    .iter()
                    .filter(|(&id, x)| self.valid(id) && (x.new || x.get(d) != 0))
                    .map(|(&id, x)| {
                        let now = self.tree.node(id).weight(d) as i64;
                        let dv = if x.new { now } else { x.get(d) };
                        let before = now - dv;
                        let path = self.tree.path(id);
                        let tone = if x.new {
                            2
                        } else if dv > 0 {
                            1
                        } else {
                            -1
                        };
                        let row = FlatRow {
                            cells: vec![
                                self.relative(&path),
                                if x.new { "—".into() } else { fmt::size(before.max(0) as u64) },
                                fmt::size(now as u64),
                                if x.new { "nouveau".into() } else { fmt::delta(dv) },
                            ],
                            nums: vec![None, Some(before), Some(now), Some(dv)],
                            export: vec![path.clone(), if x.new { String::new() } else { before.to_string() }, now.to_string(), dv.to_string(), if x.new { "oui".into() } else { String::new() }],
                            node: id,
                            path,
                            frac: 0.0,
                            tone,
                        };
                        (dv.abs(), row)
                    })
                    .collect();
                rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| cmp_names(&a.1.path, &b.1.path)));
                rows.into_iter().map(|r| r.1).collect()
            }
        }
    }

    /// Calcule les cases de la carte si le dossier, les données ou la taille ont changé.
    fn ensure_map(&mut self, rect: Rect) {
        if !self.has_data {
            return;
        }
        if !self.valid(self.map_root) {
            self.map_root = Tree::ROOT;
        }
        let a = map_area(rect);
        let key = (self.map_root, self.data_rev, self.s.on_disk, [a.left() as i32, a.top() as i32, a.right() as i32, a.bottom() as i32]);
        if self.map_cache.as_ref().is_some_and(|c| c.0 == key) {
            return;
        }
        let items = treemap::layout(&self.tree, self.map_root, a, self.s.on_disk);
        self.map_cache = Some((key, items));
    }

    // =======================================================================
    // Zone principale
    // =======================================================================

    pub fn main_area(&mut self, ui: &mut Ui, p: &'static Palette, acts: &mut Vec<Act>) {
        self.ensure_flat();
        ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
        self.stats(ui, p);
        self.progress(ui, p);
        self.tabs(ui, p, acts);
        self.toolbar(ui, p, acts);

        let rect = ui.available_rect_before_wrap();
        let tab = self.s.tab;
        let selected = self.selected.filter(|&s| self.has_data && self.valid(s) && !matches!(tab, Tab::Exts | Tab::Ages));
        let (main_rect, panel_rect) = if selected.is_some() && rect.width() > DETAILS_W + 380.0 {
            (Rect::from_min_max(rect.min, pos2(rect.right() - DETAILS_W - 12.0, rect.bottom())), Some(Rect::from_min_max(pos2(rect.right() - DETAILS_W, rect.top()), rect.max)))
        } else {
            (rect, None)
        };
        if tab == Tab::Map {
            self.ensure_map(main_rect);
        }
        ui.allocate_new_ui(UiBuilder::new().max_rect(main_rect), |ui| match tab {
            Tab::Tree => self.tree_table(ui, p, acts),
            Tab::Map => self.map_view(ui, p, acts),
            _ => self.flat_table(ui, p, acts),
        });
        if let (Some(id), Some(pr)) = (selected, panel_rect) {
            ui.allocate_new_ui(UiBuilder::new().max_rect(pr), |ui| self.details(ui, p, id, acts));
        }
    }

    fn stats(&self, ui: &mut Ui, p: &'static Palette) {
        let width = ui.available_width();
        let cw = ((width - 36.0) / 4.0).floor();
        let (row, _) = ui.allocate_exact_size(vec2(width, 72.0), Sense::hover());
        let on_disk = self.s.on_disk;
        let label = if on_disk { "Taille sur le disque" } else { "Taille analysée" };
        let cards: [(&str, String, String); 4] = if self.has_data {
            let r = self.tree.node(Tree::ROOT);
            let (v, u) = fmt::size_parts(r.weight(on_disk));
            let sub = match self.drive_used {
                Some(used) if self.rescan.is_none() => format!("{u} / {} occupés", fmt::size(used)),
                _ => u.to_string(),
            };
            let dirs_sub = if r.denied > 0 { format!("dont {}", fmt::plural(r.denied as u64, "illisible", "illisibles")) } else { String::new() };
            [(label, v, sub), ("Fichiers", fmt::thousands(r.files as u64), String::new()), ("Dossiers", fmt::thousands(r.dirs as u64), dirs_sub), ("Durée de l'analyse", fmt::secs(self.elapsed), "s".into())]
        } else {
            [(label, "—".into(), String::new()), ("Fichiers", "—".into(), String::new()), ("Dossiers", "—".into(), String::new()), ("Durée de l'analyse", "—".into(), String::new())]
        };
        for (i, (label, value, sub)) in cards.iter().enumerate() {
            let x = row.left() + i as f32 * (cw + 12.0);
            let cw = if i == 3 { row.right() - x } else { cw };
            let r = Rect::from_min_size(pos2(x, row.top()), vec2(cw, 72.0));
            paint_stat(ui, p, r, label, value, sub);
        }
    }

    fn progress(&self, ui: &mut Ui, p: &'static Palette) {
        let width = ui.available_width();
        let (row, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
        let painter = ui.painter();
        let cy = row.center().y;
        let track = Rect::from_min_max(pos2(row.left(), cy - 3.0), pos2(row.right() - 56.0, cy + 3.0));
        painter.rect_filled(track, 3.0, p.track);
        let label = |text: String| {
            let g = w::galley(ui, text, regular(12.0), p.text2);
            w::text_at(painter, g.clone(), row.right() - g.size().x, cy);
        };
        if self.running() {
            let known = self.drive_used.filter(|&u| u > 0 && self.rescan.is_none());
            if let Some(used) = known {
                // Lecteur entier : l'espace occupé donne une estimation de l'avancement.
                let frac = (self.tree.node(Tree::ROOT).alloc as f64 / used as f64).clamp(0.0, 0.99) as f32;
                painter.rect_filled(Rect::from_min_size(track.min, vec2((track.width() * frac).max(6.0), 6.0)), 3.0, p.accent);
                label(format!("{} %", (frac * 100.0).floor() as u32));
            } else {
                // Dossier : avancement inconnu, segment qui défile.
                let t = ui.input(|i| i.time) as f32;
                let seg = track.width() * 0.22;
                let x = track.left() + ((t * 0.55) % 1.3 - 0.25) * track.width();
                let r = Rect::from_min_max(pos2(x.max(track.left()), track.top()), pos2((x + seg).min(track.right()), track.bottom()));
                if r.width() > 0.0 {
                    painter.rect_filled(r, 3.0, p.accent);
                }
                ui.ctx().request_repaint_after(Duration::from_millis(33));
                label("…".into());
            }
        } else if self.has_data {
            let (color, text) = match self.end {
                Some(ScanEnd::Cancelled) => (p.warn, "partiel"),
                _ => (p.ok, "100 %"),
            };
            painter.rect_filled(track, 3.0, color);
            label(text.into());
        }
    }

    fn tabs(&self, ui: &mut Ui, p: &'static Palette, acts: &mut Vec<Act>) {
        let width = ui.available_width();
        let (row, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
        let changes = self.deltas.values().filter(|d| d.new || d.get(self.s.on_disk) != 0).count();
        let tabs: [(Tab, &str, &Icon); 6] = [
            (Tab::Tree, "Arborescence", &icons::LIST),
            (Tab::Map, "Carte", &icons::TREEMAP),
            (Tab::Files, "Plus gros fichiers", &icons::FILE),
            (Tab::Exts, "Extensions", &icons::FILES),
            (Tab::Ages, "Ancienneté", &icons::CLOCK),
            (Tab::Changes, "Évolution", &icons::ACTIVITY),
        ];
        let font = regular(12.5);
        let items: Vec<(Tab, std::sync::Arc<egui::Galley>, &Icon, Option<std::sync::Arc<egui::Galley>>)> = tabs
            .iter()
            .map(|&(t, label, ic)| {
                let selected = self.s.tab == t;
                let g = w::galley(ui, label, if selected { semibold(12.5) } else { font.clone() }, if selected { p.text } else { p.text2 });
                let badge = (t == Tab::Changes && changes > 0).then(|| w::galley(ui, fmt::thousands(changes as u64), semibold(11.0), p.accent_text));
                (t, g, ic, badge)
            })
            .collect();
        let widths: Vec<f32> = items.iter().map(|(_, g, _, b)| 12.0 + 14.0 + 7.0 + g.size().x + b.as_ref().map_or(0.0, |b| b.size().x + 16.0) + 12.0).collect();
        let total: f32 = widths.iter().sum::<f32>() + 6.0;
        let container = Rect::from_min_size(row.min, vec2(total.min(width), 34.0));
        ui.painter().rect(container.shrink(0.5), 8.0, p.surface2, stroke(1.0, p.border));
        let mut x = container.left() + 3.0;
        for ((t, g, ic, badge), tw) in items.into_iter().zip(widths) {
            let r = Rect::from_min_size(pos2(x, row.top() + 3.0), vec2(tw, 28.0));
            x += tw;
            let resp = ui.interact(r, ui.id().with(("tab", t as u8)), Sense::click());
            let selected = self.s.tab == t;
            let painter = ui.painter();
            if selected {
                painter.rect(r, 6.0, p.surface, stroke(1.0, p.border_strong));
            } else if resp.hovered() {
                painter.rect_filled(r, 6.0, p.hover);
            }
            let cy = r.center().y;
            icons::paint(ui, ic, w::icon_rect(r.left() + 12.0, cy, 14.0), if selected { p.accent_text } else { p.text3 });
            let gw = w::text_at(painter, g, r.left() + 33.0, cy);
            if let Some(b) = badge {
                let br = Rect::from_min_size(pos2(r.left() + 33.0 + gw + 7.0, cy - 9.0), vec2(b.size().x + 10.0, 18.0));
                painter.rect_filled(br, 9.0, p.accent_soft);
                w::text_at(painter, b, br.left() + 5.0, cy);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !selected {
                acts.push(Act::SetTab(t));
            }
        }
    }

    fn count_label(&self) -> String {
        if !self.has_data {
            return String::new();
        }
        let searching = !self.filter.trim().is_empty();
        match self.s.tab {
            Tab::Tree if searching => {
                if self.matches > MAX_HITS {
                    format!("{} premiers résultats sur {}", fmt::thousands(MAX_HITS as u64), fmt::thousands(self.matches as u64))
                } else {
                    fmt::plural(self.matches as u64, "résultat", "résultats")
                }
            }
            Tab::Tree => fmt::plural(self.rows.len() as u64, "ligne affichée", "lignes affichées"),
            Tab::Map => "Surface proportionnelle à la taille · double-clic : entrer dans un dossier".into(),
            Tab::Files => fmt::plural(self.flat.len() as u64, "fichier", "fichiers"),
            Tab::Exts => fmt::plural(self.flat.len() as u64, "extension", "extensions"),
            Tab::Ages => "Fichiers classés par date de dernière modification".into(),
            Tab::Changes => fmt::plural(self.flat.len() as u64, "dossier a changé", "dossiers ont changé"),
        }
    }

    fn toolbar(&mut self, ui: &mut Ui, p: &'static Palette, acts: &mut Vec<Act>) {
        let tab = self.s.tab;
        let n = match tab {
            Tab::Tree => self.rows.len(),
            Tab::Map => usize::from(self.has_data),
            _ => self.flat.len(),
        };
        let has_rows = self.has_data && n > 0;
        let can_clear = self.has_data && !self.running();
        let filter_tab = self.filter_applies();
        let filter_on = !self.filter.trim().is_empty();
        let count = self.count_label();
        let hint = match tab {
            Tab::Tree => "Filtrer : nom de fichier ou de dossier",
            Tab::Files => "Filtrer : nom, dossier",
            Tab::Exts => "Filtrer : extension",
            _ => "Filtrer : chemin",
        };
        let (mut export, mut clear, mut copy) = (false, false, false);
        let width = ui.available_width();
        ui.allocate_ui_with_layout(vec2(width, 34.0), Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_size(vec2(width, 34.0));
            ui.spacing_mut().item_spacing.x = 8.0;
            let r = Button::new(Kind::Secondary, "").icon(&icons::TRASH).size(30.0, 30.0).enabled(can_clear).show(ui, p);
            if r.on_hover_text("Effacer les résultats (et la dernière analyse enregistrée)").clicked() {
                clear = true;
            }
            let r = Button::new(Kind::Secondary, "Exporter CSV").icon(&icons::DOWNLOAD).enabled(has_rows).show(ui, p);
            if r.on_hover_text("Exporter le tableau affiché (séparateur « ; », compatible Excel)").clicked() {
                export = true;
            }
            let r = Button::new(Kind::Secondary, "Copier").icon(&icons::COPY).enabled(has_rows).show(ui, p);
            if r.on_hover_text("Copier le tableau affiché, prêt à coller dans Excel").clicked() {
                copy = true;
            }
            ui.add_space(4.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                if filter_tab {
                    let out = w::input(ui, p, Input { text: &mut self.filter, id: "filter", hint, width: 260.0, height: 34.0, mono: false, error: false, search: true, clearable: true, highlight: filter_on });
                    if out.cleared {
                        self.filter.clear();
                    }
                    if out.cleared || out.response.changed() {
                        self.rows_dirty = true;
                    }
                }
                ui.add(Label::new(RichText::new(count).font(regular(12.0)).color(p.text3)).truncate());
            });
        });
        if copy {
            let (h, rows) = self.export_table();
            let n = rows.len() as u64;
            acts.push(Act::Copy(fmt::tsv(&h, &rows), format!("Tableau copié dans le presse-papiers : {}, prêt à coller dans Excel.", fmt::plural(n, "ligne", "lignes"))));
        }
        self.toolbar_actions(export, clear);
    }

    // -- Arborescence ---------------------------------------------------------

    fn tree_table(&self, ui: &mut Ui, p: &'static Palette, acts: &mut Vec<Act>) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 8.0, p.surface);
        let inner = rect.shrink(1.0);
        let cols = layout_columns(inner.left(), inner.width(), &self.s.col_order, !self.deltas.is_empty());
        // Colonne en cours de déplacement : (trait d'insertion, étiquette qui suit la souris).
        let mut drag_mark: Option<(f32, &'static str, egui::Pos2)> = None;

        let header = Rect::from_min_size(inner.min, vec2(inner.width(), 32.0));
        let painter = ui.painter();
        painter.rect_filled(header, Rounding { nw: 7.0, ne: 7.0, sw: 0.0, se: 0.0 }, p.surface2);
        painter.hline(header.x_range(), header.bottom() - 0.5, stroke(1.0, p.border));
        let cy = header.center().y;
        let col_at = |x: f32| cols.iter().position(|t| x >= t.x - 7.0 && x < t.x + t.w + 7.0);
        for (ci, c) in cols.iter().enumerate() {
            let key = c.kind.sort_key();
            let sorted = self.sort == key && c.kind != ColKind::Percent;
            let hit = Rect::from_min_size(pos2(c.x - 7.0, header.top()), vec2(c.w + 14.0, 32.0));
            // Clic : trier. Glisser : déplacer la colonne. Clic droit : ordre par défaut.
            let resp = ui.interact(hit, ui.id().with(("col", c.kind)), Sense::click_and_drag());
            let pointer = ui.input(|i| i.pointer.interact_pos());
            let target = pointer.and_then(|pos| col_at(pos.x)).filter(|&ti| ti != ci);
            if resp.dragged() {
                if let Some(pos) = pointer {
                    let line_x = target.map(|ti| {
                        let t = &cols[ti];
                        if ti < ci {
                            t.x - 7.0
                        } else {
                            t.x + t.w + 7.0
                        }
                    });
                    drag_mark = Some((line_x.unwrap_or(f32::NAN), c.kind.title(), pos));
                }
            }
            if resp.drag_stopped() {
                if let Some(ti) = target {
                    acts.push(Act::MoveCol(c.kind, cols[ti].kind));
                }
            }
            resp.context_menu(|ui| {
                ui.set_min_width(230.0);
                if w::menu_item(ui, p, Some(&icons::REFRESH), "Rétablir l'ordre des colonnes", None).clicked() {
                    acts.push(Act::ResetCols);
                    ui.close_menu();
                }
            });
            let color = if sorted || resp.dragged() {
                p.accent_text
            } else if resp.hovered() {
                p.text
            } else {
                p.text2
            };
            let g = w::galley(ui, c.kind.title(), semibold(12.0), color);
            let gw = g.size().x;
            let full = gw + if sorted { 16.0 } else { 0.0 };
            let x = if c.kind.right() { c.x + c.w - full } else { c.x };
            w::text_at(ui.painter(), g, x, cy);
            if sorted {
                let ic = if self.sort_asc { &icons::CHEVRON_UP } else { &icons::CHEVRON_DOWN_BOLD };
                icons::paint(ui, ic, w::icon_rect(x + gw + 4.0, cy, 12.0), color);
            }
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Cliquer pour trier · glisser pour déplacer la colonne");
            if resp.clicked() {
                acts.push(Act::Sort(key));
            }
        }
        if let Some((line_x, title, pos)) = drag_mark {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let top = egui::LayerId::new(egui::Order::Tooltip, ui.id().with("col_drag"));
            let fg = ui.ctx().layer_painter(top);
            if line_x.is_finite() {
                fg.vline(line_x, rect.y_range(), stroke(2.0, p.accent));
            }
            let g = w::galley(ui, title, semibold(12.0), p.text);
            let tag = Rect::from_min_size(pos2(pos.x + 12.0, header.top() + 3.0), vec2(g.size().x + 20.0, 26.0));
            fg.rect(tag, 6.0, p.menu, stroke(1.0, p.accent));
            w::text_at(&fg, g, tag.left() + 10.0, tag.center().y);
        }

        let body = Rect::from_min_max(pos2(inner.left(), header.bottom()), inner.max);
        ui.allocate_new_ui(UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(body.intersect(ui.clip_rect()));
            if !self.has_data {
                self.empty_state(ui, p, body);
                return;
            }
            if self.rows.is_empty() {
                self.no_match(ui, p, acts, body);
                return;
            }
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut area = egui::ScrollArea::vertical().id_salt("rows").auto_shrink(false);
            if let Some(k) = self.scroll_to {
                let id = ui.make_persistent_id("rows");
                let cur = egui::scroll_area::State::load(ui.ctx(), id).map_or(0.0, |s| s.offset.y);
                let (top, bottom) = (k as f32 * ROW_H, k as f32 * ROW_H + ROW_H);
                if top < cur {
                    area = area.vertical_scroll_offset(top);
                } else if bottom > cur + body.height() {
                    area = area.vertical_scroll_offset(bottom - body.height());
                }
            }
            area.show_rows(ui, ROW_H, self.rows.len(), |ui, range| {
                for i in range {
                    self.tree_row(ui, p, &cols, self.rows[i], acts);
                }
            });
        });
        ui.painter().rect_stroke(rect.shrink(0.5), 8.0, stroke(1.0, p.border));
    }

    fn tree_row(&self, ui: &mut Ui, p: &'static Palette, cols: &[Col], row: Row, acts: &mut Vec<Act>) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
        let id = row.id;
        let n = self.tree.node(id);
        let on_disk = self.s.on_disk;
        let searching = !self.filter.trim().is_empty();
        let is_dir = n.is_dir();
        let expandable = is_dir && !searching && !n.has(flag::LINK) && (self.tree.has_children(id) || n.has(flag::PENDING));
        let open = self.expanded.contains(&id);
        let mut chevron_clicked = false;
        if ui.is_rect_visible(rect) {
            let selected = self.selected == Some(id);
            let painter = ui.painter();
            if selected {
                painter.rect_filled(rect, 0.0, p.selected);
            } else if resp.hovered() || resp.context_menu_opened() {
                painter.rect_filled(rect, 0.0, p.hover);
            }
            painter.hline(rect.x_range(), rect.bottom() - 0.5, stroke(1.0, p.row_line));
            if selected {
                painter.rect_stroke(rect.shrink(0.5), 0.0, stroke(1.0, p.selected_border));
            }
            let cy = rect.center().y;
            let pending = n.has(flag::PENDING) && is_dir && !n.has(flag::LINK);
            let dim = n.has(flag::GROUP) || n.has(flag::DUP) || n.has(flag::LINK) || (pending && !self.running());
            let main = if dim { p.text3 } else { p.text };
            for c in cols {
                let cell = Rect::from_min_max(pos2(c.x, rect.top()), pos2(c.x + c.w, rect.bottom()));
                let cp = painter.with_clip_rect(cell.intersect(painter.clip_rect()));
                match c.kind {
                    ColKind::Name => {
                        let indent = if searching { 0.0 } else { (row.depth as f32 * 18.0).min((c.w - 150.0).max(0.0)) };
                        let x = c.x + indent;
                        if expandable {
                            let ic = if open { &icons::CHEVRON_DOWN_TREE } else { &icons::CHEVRON_RIGHT };
                            icons::paint(ui, ic, w::icon_rect(x, cy, 13.0), p.text3);
                            let hit = Rect::from_min_size(pos2(x - 5.0, rect.top()), vec2(22.0, ROW_H));
                            if ui.interact(hit, ui.id().with(("chev", id)), Sense::click()).clicked() {
                                chevron_clicked = true;
                            }
                        }
                        let (icon, ic_color) = if n.has(flag::GROUP) {
                            (&icons::FILES, p.text3)
                        } else if n.has(flag::LINK) {
                            (&icons::LINK, p.text3)
                        } else if n.has(flag::DENIED) {
                            (&icons::LOCK, p.err)
                        } else if id == Tree::ROOT {
                            (&icons::HARD_DRIVE, p.accent_text)
                        } else if is_dir {
                            (&icons::FOLDER, p.accent_text)
                        } else {
                            (&icons::FILE, p.text3)
                        };
                        icons::paint(ui, icon, w::icon_rect(x + 18.0, cy, 15.0), ic_color);
                        let tx = x + 40.0;
                        // Badges à droite du nom.
                        let mut right = c.x + c.w;
                        let mut badges: Vec<(&str, Color32, Color32)> = Vec::new();
                        if n.has(flag::DENIED) {
                            badges.push(("accès refusé", p.err, p.err_soft));
                        }
                        if n.has(flag::LINK) {
                            badges.push((if is_dir { "non suivi" } else { "lien" }, p.badge_text, p.badge));
                        }
                        if n.has(flag::DUP) {
                            badges.push(("lien physique", p.badge_text, p.badge));
                        }
                        if self.deltas.get(&id).is_some_and(|d| d.new) {
                            badges.push(("nouveau", p.accent_text, p.accent_soft));
                        }
                        if pending && !self.running() {
                            badges.push(("non parcouru", p.warn, p.warn_soft));
                        }
                        for (t, fg, bg) in badges {
                            let g = w::galley(ui, t, semibold(11.0), fg);
                            let bw = g.size().x + 12.0;
                            let br = Rect::from_min_size(pos2(right - bw, cy - 9.0), vec2(bw, 18.0));
                            cp.rect_filled(br, 4.0, bg);
                            w::text_at(&cp, g, br.left() + 6.0, cy);
                            right -= bw + 6.0;
                        }
                        if pending && self.running() {
                            egui::Spinner::new().size(12.0).color(p.accent_text).paint_at(ui, w::icon_rect(right - 14.0, cy, 12.0));
                            right -= 20.0;
                        }
                        let name = self.display_name(id);
                        let font = if id == Tree::ROOT { semibold(13.0) } else { regular(13.0) };
                        let g = w::galley_trunc(ui, &name, font, main, right - 8.0 - tx);
                        let used = w::text_at(&cp, g, tx, cy);
                        if searching && n.parent != NONE {
                            let px = tx + used + 10.0;
                            let parent = self.relative(&self.tree.path(n.parent));
                            let g = w::galley_trunc(ui, &parent, mono(11.5), p.text3, right - 8.0 - px);
                            w::text_at(&cp, g, px, cy);
                        }
                    }
                    ColKind::Size => {
                        let col = if pending && self.running() { p.text3 } else { main };
                        let g = w::galley(ui, fmt::size(n.weight(on_disk)), regular(12.5), col);
                        let gw = g.size().x;
                        w::text_at(&cp, g, c.x + c.w - gw, cy);
                    }
                    ColKind::Percent => {
                        let frac = self.share(id) as f32;
                        let tw = c.w - 58.0;
                        let track = Rect::from_min_size(pos2(c.x, cy - 3.0), vec2(tw, 6.0));
                        cp.rect_filled(track, 3.0, p.track);
                        if frac > 0.0 {
                            let color = if is_dir { p.accent } else { w::lerp_color(p.accent, p.track, 0.45) };
                            cp.rect_filled(Rect::from_min_size(track.min, vec2((tw * frac).max(3.0), 6.0)), 3.0, color);
                        }
                        let g = w::galley(ui, fmt::percent(frac as f64), regular(12.0), p.text2);
                        let gw = g.size().x;
                        w::text_at(&cp, g, c.x + c.w - gw, cy);
                    }
                    ColKind::Files | ColKind::Dirs => {
                        let v = if c.kind == ColKind::Files { n.files } else { n.dirs };
                        let shown = is_dir || (c.kind == ColKind::Files && n.has(flag::GROUP));
                        let (t, col) = if shown { (fmt::thousands(v as u64), p.text2) } else { ("—".to_string(), p.text3) };
                        let g = w::galley(ui, t, regular(12.5), col);
                        let gw = g.size().x;
                        w::text_at(&cp, g, c.x + c.w - gw, cy);
                    }
                    ColKind::Modified => {
                        w::text_at(&cp, w::galley(ui, fmt::date(n.mtime), regular(12.5), p.text2), c.x, cy);
                    }
                    ColKind::Delta => {
                        let (t, col) = match self.deltas.get(&id) {
                            Some(d) if d.new => ("nouveau".to_string(), p.accent_text),
                            Some(d) if d.get(on_disk) > 0 => (fmt::delta(d.get(on_disk)), p.warn),
                            Some(d) if d.get(on_disk) < 0 => (fmt::delta(d.get(on_disk)), p.ok),
                            _ => ("—".to_string(), p.text3),
                        };
                        let g = w::galley(ui, t, regular(12.5), col);
                        let gw = g.size().x;
                        w::text_at(&cp, g, c.x + c.w - gw, cy);
                    }
                }
            }
        }
        if chevron_clicked {
            acts.push(Act::Toggle(id));
        } else if resp.double_clicked() {
            if searching {
                acts.push(Act::ShowInTree(id));
            } else if expandable {
                acts.push(Act::Toggle(id));
            }
        } else if resp.clicked() {
            acts.push(Act::Select(id));
        }
        if resp.secondary_clicked() {
            acts.push(Act::MenuTarget(id));
        }
        resp.context_menu(|ui| item_menu(self, ui, p, id, &self.tree.path(id), acts));
        if resp.context_menu_opened() {
            self.menu_open.set(true);
        }
    }

    // -- Tableaux simples -----------------------------------------------------

    fn flat_table(&self, ui: &mut Ui, p: &'static Palette, acts: &mut Vec<Act>) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 8.0, p.surface);
        let inner = rect.shrink(1.0);
        let tab = self.s.tab;
        let defs = flat_cols(tab);
        let cols = flat_layout(inner.left(), inner.width(), defs);
        let sort = self.flat_sort.get(&tab).copied();

        let header = Rect::from_min_size(inner.min, vec2(inner.width(), 32.0));
        let painter = ui.painter();
        painter.rect_filled(header, Rounding { nw: 7.0, ne: 7.0, sw: 0.0, se: 0.0 }, p.surface2);
        painter.hline(header.x_range(), header.bottom() - 0.5, stroke(1.0, p.border));
        let cy = header.center().y;
        for (ci, (def, &(x, cw))) in defs.iter().zip(&cols).enumerate() {
            let hit = Rect::from_min_size(pos2(x - 7.0, header.top()), vec2(cw + 14.0, 32.0));
            let resp = ui.interact(hit, ui.id().with(("fcol", tab as u8, ci)), Sense::click());
            let sorted = sort.is_some_and(|s| s.0 == ci);
            let color = if sorted {
                p.accent_text
            } else if resp.hovered() {
                p.text
            } else {
                p.text2
            };
            let g = w::galley(ui, def.title, semibold(12.0), color);
            let gw = g.size().x;
            let full = gw + if sorted { 16.0 } else { 0.0 };
            let tx = if def.right { x + cw - full } else { x };
            w::text_at(ui.painter(), g, tx, cy);
            if let Some((_, asc)) = sort.filter(|s| s.0 == ci) {
                let ic = if asc { &icons::CHEVRON_UP } else { &icons::CHEVRON_DOWN_BOLD };
                icons::paint(ui, ic, w::icon_rect(tx + gw + 4.0, cy, 12.0), color);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Cliquer pour trier").clicked() {
                acts.push(Act::SortFlat(ci));
            }
        }

        let body = Rect::from_min_max(pos2(inner.left(), header.bottom()), inner.max);
        ui.allocate_new_ui(UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(body.intersect(ui.clip_rect()));
            if !self.has_data {
                self.empty_state(ui, p, body);
                return;
            }
            if self.flat.is_empty() {
                self.no_match(ui, p, acts, body);
                return;
            }
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical().id_salt(("flat", tab as u8)).auto_shrink(false).show_rows(ui, ROW_H, self.flat.len(), |ui, range| {
                for i in range {
                    self.flat_row(ui, p, defs, &cols, &self.flat[i], acts);
                }
            });
        });
        ui.painter().rect_stroke(rect.shrink(0.5), 8.0, stroke(1.0, p.border));
    }

    fn flat_row(&self, ui: &mut Ui, p: &'static Palette, defs: &[FlatCol], cols: &[(f32, f32)], r: &FlatRow, acts: &mut Vec<Act>) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
        let has_node = r.node != NONE;
        if ui.is_rect_visible(rect) {
            let selected = has_node && self.selected == Some(r.node);
            let painter = ui.painter();
            if selected {
                painter.rect_filled(rect, 0.0, p.selected);
            } else if (has_node || !r.path.is_empty()) && (resp.hovered() || resp.context_menu_opened()) {
                painter.rect_filled(rect, 0.0, p.hover);
            }
            painter.hline(rect.x_range(), rect.bottom() - 0.5, stroke(1.0, p.row_line));
            if selected {
                painter.rect_stroke(rect.shrink(0.5), 0.0, stroke(1.0, p.selected_border));
            }
            let cy = rect.center().y;
            for (ci, (def, &(x, cw))) in defs.iter().zip(cols).enumerate() {
                let cell = Rect::from_min_max(pos2(x, rect.top()), pos2(x + cw, rect.bottom()));
                let cp = painter.with_clip_rect(cell.intersect(painter.clip_rect()));
                let text = &r.cells[ci];
                if def.bar {
                    let tw = cw - 62.0;
                    let track = Rect::from_min_size(pos2(x, cy - 3.0), vec2(tw, 6.0));
                    cp.rect_filled(track, 3.0, p.track);
                    if r.frac > 0.0 {
                        cp.rect_filled(Rect::from_min_size(track.min, vec2((tw * r.frac).max(3.0), 6.0)), 3.0, p.accent);
                    }
                    let g = w::galley(ui, text.as_str(), regular(12.0), p.text2);
                    let gw = g.size().x;
                    w::text_at(&cp, g, x + cw - gw, cy);
                    continue;
                }
                let color = if def.tone {
                    match r.tone {
                        1 => p.warn,
                        -1 => p.ok,
                        2 => p.accent_text,
                        _ => p.text3,
                    }
                } else if ci == 0 {
                    p.text
                } else {
                    p.text2
                };
                let font = if def.mono { mono(12.0) } else { regular(12.5) };
                let g = w::galley_trunc(ui, text, font, color, cw);
                let gw = g.size().x;
                w::text_at(&cp, g, if def.right { x + cw - gw } else { x }, cy);
            }
        }
        if has_node {
            if resp.double_clicked() {
                acts.push(Act::ShowInTree(r.node));
            } else if resp.clicked() {
                acts.push(Act::Select(r.node));
            }
            if resp.secondary_clicked() {
                acts.push(Act::MenuTarget(r.node));
            }
        }
        if !r.path.is_empty() {
            let resp = if has_node { resp.on_hover_text("Double-clic : afficher dans l'arborescence") } else { resp };
            resp.context_menu(|ui| item_menu(self, ui, p, r.node, &r.path, acts));
            if resp.context_menu_opened() {
                self.menu_open.set(true);
            }
        }
    }

    // -- Carte proportionnelle ------------------------------------------------

    fn map_view(&self, ui: &mut Ui, p: &'static Palette, acts: &mut Vec<Act>) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 8.0, p.surface);
        if !self.has_data {
            self.empty_state(ui, p, rect);
            ui.painter().rect_stroke(rect.shrink(0.5), 8.0, stroke(1.0, p.border));
            return;
        }
        let crumb = Rect::from_min_max(rect.min + vec2(10.0, 8.0), pos2(rect.right() - 10.0, rect.top() + 38.0));
        ui.allocate_new_ui(UiBuilder::new().max_rect(crumb).layout(Layout::left_to_right(Align::Center)), |ui| self.breadcrumb(ui, p, acts));
        let area = map_area(rect);
        let items: &[treemap::Item] = self.map_cache.as_ref().map_or(&[], |c| c.1.as_slice());
        if items.is_empty() {
            ui.allocate_new_ui(UiBuilder::new().max_rect(area).layout(Layout::top_down(Align::Center)), |ui| {
                ui.add_space((area.height() / 2.0 - 30.0).max(10.0));
                w::label(ui, "Rien à afficher", semibold(15.0), p.text);
                centered_text(ui, "Ce dossier est vide, ou son contenu n'a pas encore été parcouru.", 360.0, p.text3);
            });
            ui.painter().rect_stroke(rect.shrink(0.5), 8.0, stroke(1.0, p.border));
            return;
        }

        let resp = ui.interact(area, ui.id().with("treemap"), Sense::click());
        let hovered = resp.hover_pos().and_then(|pos| items.iter().rposition(|it| it.rect.contains(pos)));
        let painter = ui.painter().with_clip_rect(area.intersect(ui.clip_rect()));
        let on_disk = self.s.on_disk;
        let tile_text = if p.dark { Color32::from_white_alpha(225) } else { Color32::from_rgb(0x15, 0x18, 0x1C) };
        for (k, it) in items.iter().enumerate() {
            let n = self.tree.node(it.node);
            let r = it.rect;
            let hot = hovered == Some(k);
            if it.leaf {
                let base = if it.hue != 255 {
                    hue(p, it.hue)
                } else if n.has(flag::GROUP) {
                    w::lerp_color(p.track, p.idle, 0.25)
                } else {
                    w::lerp_color(p.surface2, p.idle, 0.4)
                };
                let fill = if hot { w::lerp_color(base, if p.dark { Color32::WHITE } else { Color32::BLACK }, 0.12) } else { base };
                let round = if r.width() > 12.0 && r.height() > 12.0 { 2.0 } else { 0.0 };
                painter.rect(r, round, fill, stroke(1.0, p.surface));
                if r.width() >= 44.0 && r.height() >= 18.0 {
                    let name = self.display_name(it.node);
                    let g = w::galley_trunc(ui, &name, regular(11.5), tile_text, r.width() - 8.0);
                    painter.galley(pos2(r.left() + 4.0, r.top() + 3.0), g, tile_text);
                    if r.height() >= 34.0 {
                        let g = w::galley_trunc(ui, &fmt::size(n.weight(on_disk)), regular(11.0), tile_text.gamma_multiply(0.75), r.width() - 8.0);
                        painter.galley(pos2(r.left() + 4.0, r.top() + 18.0), g, tile_text);
                    }
                }
            } else {
                let shade = (it.depth as f32 * 0.12).min(0.5);
                let fill = w::lerp_color(p.surface2, p.border, shade);
                painter.rect(r, 3.0, fill, stroke(1.0, if hot { p.text3 } else { p.border_strong }));
                if it.header {
                    let size = w::galley(ui, fmt::size(n.weight(on_disk)), regular(11.0), p.text3);
                    let sw = size.size().x;
                    let name_w = r.width() - 12.0 - if r.width() > sw + 80.0 { sw + 8.0 } else { 0.0 };
                    let g = w::galley_trunc(ui, &self.display_name(it.node), semibold(11.5), p.text, name_w);
                    let hy = r.top() + treemap::HEADER_H / 2.0 + 1.0;
                    w::text_at(&painter, g, r.left() + 6.0, hy);
                    if r.width() > sw + 80.0 {
                        w::text_at(&painter, size, r.right() - 6.0 - sw, hy);
                    }
                }
            }
        }
        // Sélection et survol par-dessus tout le reste.
        if let Some(s) = self.selected {
            if let Some(it) = items.iter().find(|it| it.node == s) {
                painter.rect_stroke(it.rect.shrink(1.0), 2.0, stroke(2.0, p.accent));
            }
        }
        if let Some(k) = hovered {
            painter.rect_stroke(items[k].rect, 2.0, stroke(1.5, p.text));
        }

        let target = hovered.map(|k| items[k].node);
        if let Some(id) = target {
            if resp.double_clicked() {
                // Entrer dans le dossier de premier niveau qui contient la case.
                let mut cur = id;
                while self.tree.node(cur).parent != self.map_root && self.tree.node(cur).parent != NONE {
                    cur = self.tree.node(cur).parent;
                }
                let c = self.tree.node(cur);
                if c.is_dir() && self.tree.has_children(cur) {
                    acts.push(Act::ShowInMap(cur));
                }
            } else if resp.clicked() {
                acts.push(Act::Select(id));
            }
            if resp.secondary_clicked() {
                acts.push(Act::MenuTarget(id));
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("map_menu_target"), id));
            }
        }
        resp.context_menu(|ui| {
            let id: Option<NodeId> = ui.ctx().data(|d| d.get_temp(egui::Id::new("map_menu_target")));
            match id.filter(|&id| self.valid(id)) {
                Some(id) => item_menu(self, ui, p, id, &self.tree.path(id), acts),
                None => ui.close_menu(),
            }
        });
        if resp.context_menu_opened() {
            self.menu_open.set(true);
        }
        if let Some(id) = target {
            let n = self.tree.node(id);
            let total = self.tree.node(self.map_root).weight(on_disk).max(1);
            resp.on_hover_ui_at_pointer(|ui| {
                ui.set_max_width(420.0);
                w::label(ui, self.display_name(id), semibold(13.0), p.text);
                ui.add(Label::new(RichText::new(self.tree.path(id)).font(mono(11.5)).color(p.text3)).wrap());
                let mut line = format!("{} · {} du dossier affiché", fmt::size(n.weight(on_disk)), fmt::percent(n.weight(on_disk) as f64 / total as f64));
                if n.is_dir() {
                    line.push_str(&format!(" · {}", fmt::plural(n.files as u64, "fichier", "fichiers")));
                }
                w::label(ui, line, regular(12.0), p.text2);
            });
        }
        ui.painter().rect_stroke(rect.shrink(0.5), 8.0, stroke(1.0, p.border));
    }

    fn breadcrumb(&self, ui: &mut Ui, p: &'static Palette, acts: &mut Vec<Act>) {
        // Total à droite d'abord, le chemin prend la place qui reste.
        let n = self.tree.node(self.map_root);
        let info = format!("{} · {}", fmt::size(n.weight(self.s.on_disk)), fmt::plural(n.files as u64, "fichier", "fichiers"));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            w::label(ui, info, regular(12.0), p.text3);
            ui.add_space(12.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                self.crumbs(ui, p, acts);
            });
        });
    }

    fn crumbs(&self, ui: &mut Ui, p: &'static Palette, acts: &mut Vec<Act>) {
        ui.spacing_mut().item_spacing.x = 2.0;
        let parent = self.tree.node(self.map_root).parent;
        let r = Button::new(Kind::Secondary, "").icon(&icons::ARROW_UP).icon_size(14.0).size(28.0, 28.0).enabled(parent != NONE).show(ui, p);
        if r.on_hover_text("Dossier parent").clicked() && parent != NONE {
            acts.push(Act::ShowInMap(parent));
        }
        ui.add_space(8.0);
        let mut chain = Vec::new();
        let mut cur = self.map_root;
        while cur != NONE {
            chain.push(cur);
            cur = self.tree.node(cur).parent;
        }
        chain.reverse();
        // Chemin trop long : la racine, « … », puis les trois derniers dossiers.
        let skip = chain.len() > 5;
        let last = chain.len() - 1;
        for (k, &c) in chain.iter().enumerate() {
            if skip && k > 0 && k + 3 < chain.len() {
                if k == 1 {
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 26.0), Sense::hover());
                    icons::paint(ui, &icons::CHEVRON_RIGHT, Rect::from_center_size(r.center(), Vec2::splat(11.0)), p.text3);
                    w::label(ui, "…", regular(12.5), p.text3);
                }
                continue;
            }
            if k > 0 {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 26.0), Sense::hover());
                icons::paint(ui, &icons::CHEVRON_RIGHT, Rect::from_center_size(r.center(), Vec2::splat(11.0)), p.text3);
            }
            // La racine est nommée par son dernier dossier (« jeu-essai », « C: »).
            let name = if c == Tree::ROOT {
                let root = self.tree.root_path.trim_end_matches('\\');
                root.rsplit('\\').next().unwrap_or(root).to_string()
            } else {
                self.display_name(c)
            };
            if k == last {
                let r = ui.add(Label::new(RichText::new(name).font(semibold(13.0)).color(p.text)).truncate());
                if c == Tree::ROOT {
                    r.on_hover_text(self.tree.root_path.as_str());
                }
            } else {
                let r = Button::new(Kind::Link, &name).font(regular(12.5)).size(0.0, 26.0).show(ui, p);
                let r = if c == Tree::ROOT { r.on_hover_text(self.tree.root_path.as_str()) } else { r };
                if r.clicked() {
                    acts.push(Act::ShowInMap(c));
                }
            }
        }
    }

    // -- États vides ----------------------------------------------------------

    fn empty_state(&self, ui: &mut Ui, p: &'static Palette, body: Rect) {
        ui.allocate_new_ui(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Center)), |ui| {
            ui.add_space(((body.height() - 170.0) / 2.0).max(16.0));
            ui.spacing_mut().item_spacing.y = 10.0;
            let (r, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
            w::paint_empty_logo(ui.painter(), r, p);
            w::label(ui, "Aucune analyse pour l'instant", semibold(16.0), p.text);
            centered_text(ui, "Choisissez un lecteur ou un dossier à gauche, puis lancez l'analyse.", 360.0, p.text3);
            let parts_w = w::galley(ui, "F5", regular(12.0), p.text2).size().x
                + w::galley(ui, "Entrée", regular(12.0), p.text2).size().x
                + w::galley(ui, "Lancer", regular(12.0), p.text3).size().x
                + w::galley(ui, "dans le champ Dossier", regular(12.0), p.text3).size().x
                + 24.0
                + 8.0 * 3.0
                + 12.0;
            ui.allocate_ui_with_layout(vec2(parts_w, 22.0), Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                w::kbd(ui, p, "F5");
                w::label(ui, "Lancer", regular(12.0), p.text3);
                ui.add_space(4.0);
                w::kbd(ui, p, "Entrée");
                w::label(ui, "dans le champ Dossier", regular(12.0), p.text3);
            });
        });
    }

    /// Tableau vide alors qu'une analyse est affichée : filtre, pas de comparaison…
    fn no_match(&self, ui: &mut Ui, p: &'static Palette, acts: &mut Vec<Act>, body: Rect) {
        ui.allocate_new_ui(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Center)), |ui| {
            ui.add_space(((body.height() - 170.0) / 2.0).max(16.0));
            ui.spacing_mut().item_spacing.y = 10.0;
            let (r, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
            ui.painter().circle(r.center(), 23.5, p.surface2, stroke(1.0, p.border));
            let filter = self.filter.trim();
            let icon_r = Rect::from_center_size(r.center(), Vec2::splat(22.0));
            if !filter.is_empty() && self.filter_applies() {
                icons::paint(ui, &icons::SEARCH_X, icon_r, p.text3);
                w::label(ui, format!("Aucun résultat pour « {filter} »"), semibold(16.0), p.text);
                let text = match self.s.tab {
                    Tab::Tree if !self.s.show_files => "Le filtre cherche dans les noms de dossiers. Activez « Fichiers dans l'arborescence » pour chercher aussi les fichiers.",
                    Tab::Tree => "Le filtre cherche dans les noms de fichiers et de dossiers de toute l'analyse.",
                    _ => "Le filtre cherche dans toutes les colonnes du tableau.",
                };
                centered_text(ui, text, 400.0, p.text3);
                let b = w::galley(ui, "Effacer le filtre", regular(13.0), p.text).size().x + 24.0;
                ui.allocate_ui_with_layout(vec2(b, 32.0), Layout::left_to_right(Align::Center), |ui| {
                    if Button::new(Kind::Secondary, "Effacer le filtre").font(regular(13.0)).size(b, 32.0).show(ui, p).clicked() {
                        acts.push(Act::ClearFilter);
                    }
                });
                return;
            }
            let (icon, title, text): (&Icon, &str, &str) = match self.s.tab {
                Tab::Changes if !self.compared => (
                    &icons::ACTIVITY,
                    "Pas encore de comparaison",
                    "Relancez l'analyse du même lecteur ou dossier : les dossiers qui ont grossi ou diminué depuis la dernière fois apparaîtront ici.",
                ),
                Tab::Changes => (&icons::CHECK_CIRCLE, "Aucun changement", "Aucun dossier n'a changé de taille depuis l'analyse précédente."),
                _ if self.running() => (&icons::CLOCK, "Analyse en cours…", "Ce tableau se remplit au fil de l'analyse."),
                _ => (&icons::INFO, "Aucun fichier", "Le dossier analysé ne contient aucun fichier lisible."),
            };
            icons::paint(ui, icon, icon_r, p.text3);
            w::label(ui, title, semibold(16.0), p.text);
            centered_text(ui, text, 420.0, p.text3);
        });
    }

    // -- Panneau de détails ---------------------------------------------------

    fn details(&self, ui: &mut Ui, p: &'static Palette, id: NodeId, acts: &mut Vec<Act>) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 8.0, p.surface);
        let inner = rect.shrink(1.0);
        let n = self.tree.node(id);
        let on_disk = self.s.on_disk;
        let path = self.tree.path(id);
        let is_dir = n.is_dir();

        ui.allocate_new_ui(UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner.intersect(ui.clip_rect()));
            Frame::none().inner_margin(Margin { left: 15.0, right: 11.0, top: 11.0, bottom: 12.0 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.horizontal(|ui| {
                    let (fg, bg) = if n.has(flag::DENIED) {
                        (p.err, p.err_soft)
                    } else if n.has(flag::LINK) || n.has(flag::GROUP) {
                        (p.badge_text, p.badge)
                    } else {
                        (p.accent_text, p.accent_soft)
                    };
                    pill(ui, kind_label(n), fg, bg);
                    if self.deltas.get(&id).is_some_and(|d| d.new) {
                        pill(ui, "nouveau", p.accent_text, p.accent_soft);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let r = Button::new(Kind::Ghost, "").icon(&icons::X).size(28.0, 28.0).show(ui, p);
                        if r.on_hover_text("Fermer (Échap)").clicked() {
                            acts.push(Act::Deselect);
                        }
                    });
                });
                ui.add(Label::new(RichText::new(self.display_name(id)).font(semibold(16.0)).color(p.text)).wrap());
                if id != Tree::ROOT {
                    ui.add(Label::new(RichText::new(path.as_str()).font(mono(12.0)).color(p.text2)).wrap());
                }
            });
            let y = ui.cursor().top() - 0.5;
            w::hline(ui, inner, y, p.border);

            egui::ScrollArea::vertical().id_salt("details_scroll").auto_shrink(false).show(ui, |ui| {
                Frame::none().inner_margin(Margin { left: 15.0, right: 15.0, top: 8.0, bottom: 15.0 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let dup = n.has(flag::DUP);
                    field_row(ui, p, "Taille", &if dup { format!("{} (déjà comptée)", fmt::size(n.size)) } else { fmt::size(n.size) });
                    let mut alloc = fmt::size(n.alloc);
                    if n.size > 0 && n.alloc < n.size / 10 * 9 {
                        alloc.push_str(" · compressé ou creux");
                    }
                    field_row(ui, p, "Sur le disque", &alloc);
                    if id != Tree::ROOT {
                        field_row(ui, p, "Part du parent", &fmt::percent(self.share(id)));
                        let total = self.tree.node(Tree::ROOT).weight(on_disk);
                        let frac = if total == 0 { 0.0 } else { n.weight(on_disk) as f64 / total as f64 };
                        field_row(ui, p, "Part du total", &fmt::percent(frac));
                    }
                    if is_dir || n.has(flag::GROUP) {
                        field_row(ui, p, "Fichiers", &fmt::thousands(n.files as u64));
                    }
                    if is_dir {
                        field_row(ui, p, "Sous-dossiers", &fmt::thousands(n.dirs as u64));
                        if n.denied > 0 {
                            field_row(ui, p, "Illisibles", &fmt::thousands(n.denied as u64));
                        }
                    }
                    field_row(ui, p, if is_dir { "Dernière modif." } else { "Modifié le" }, &fmt::date_time(n.mtime));
                    if let Some(d) = self.deltas.get(&id) {
                        let text = if d.new { "nouveau depuis la dernière analyse".to_string() } else { format!("{} depuis la dernière analyse", fmt::delta(d.get(on_disk))) };
                        field_row(ui, p, "Évolution", &text);
                    }

                    let note = if dup {
                        Some("Lien physique : le même fichier a un autre nom ailleurs. Sa taille n'est comptée qu'une fois.")
                    } else if n.has(flag::LINK) && is_dir {
                        Some("Jonction ou lien symbolique : DiskScan ne le suit pas, pour ne pas compter deux fois la même chose.")
                    } else if n.has(flag::DENIED) {
                        Some("Accès refusé : le contenu de ce dossier n'est pas compté. Lancez DiskScan en administrateur pour le voir.")
                    } else if n.has(flag::GROUP) {
                        Some("Analyse rechargée : seuls les dossiers sont enregistrés. Relancez l'analyse pour le détail des fichiers.")
                    } else if n.has(flag::PENDING) && is_dir && !self.running() {
                        Some("Dossier non parcouru : l'analyse a été arrêtée avant de l'atteindre.")
                    } else {
                        None
                    };
                    if let Some(text) = note {
                        ui.add_space(10.0);
                        ui.add(Label::new(RichText::new(text).font(regular(12.0)).color(p.text3)).wrap());
                    }

                    // Plus gros éléments du dossier.
                    if is_dir && self.tree.has_children(id) {
                        let kids: Vec<NodeId> = self.tree.sorted_children(id, SortKey::Size, false, on_disk, true).into_iter().filter(|&c| self.tree.node(c).weight(on_disk) > 0).take(6).collect();
                        if !kids.is_empty() {
                            ui.add_space(14.0);
                            w::field_label(ui, p, "Plus gros éléments");
                            ui.add_space(6.0);
                            let total = n.weight(on_disk).max(1) as f32;
                            for c in kids {
                                let k = self.tree.node(c);
                                let width = ui.available_width();
                                let (r, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
                                let painter = ui.painter();
                                if resp.hovered() {
                                    painter.rect_filled(r, 5.0, p.hover);
                                }
                                let cy = r.top() + 13.0;
                                let icon = if k.has(flag::GROUP) {
                                    &icons::FILES
                                } else if k.is_dir() {
                                    &icons::FOLDER
                                } else {
                                    &icons::FILE
                                };
                                icons::paint(ui, icon, w::icon_rect(r.left() + 6.0, cy, 13.0), if k.is_dir() { p.accent_text } else { p.text3 });
                                let sg = w::galley(ui, fmt::size(k.weight(on_disk)), regular(12.0), p.text2);
                                let sw = sg.size().x;
                                w::text_at(painter, sg, r.right() - 6.0 - sw, cy);
                                let g = w::galley_trunc(ui, &self.display_name(c), regular(12.5), p.text, r.width() - 40.0 - sw);
                                w::text_at(painter, g, r.left() + 25.0, cy);
                                let bar = Rect::from_min_size(pos2(r.left() + 25.0, r.top() + 24.0), vec2(r.width() - 31.0, 3.0));
                                painter.rect_filled(bar, 1.5, p.track);
                                let frac = k.weight(on_disk) as f32 / total;
                                painter.rect_filled(Rect::from_min_size(bar.min, vec2((bar.width() * frac).max(2.0), 3.0)), 1.5, p.accent);
                                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                                if resp.clicked() {
                                    acts.push(if self.s.tab == Tab::Map { Act::Select(c) } else { Act::ShowInTree(c) });
                                }
                            }
                        }
                    }

                    ui.add_space(14.0);
                    w::field_label(ui, p, "Actions");
                    ui.add_space(8.0);
                    self.details_actions(ui, p, id, &path, acts);
                });
            });
        });
        ui.painter().rect_stroke(rect.shrink(0.5), 8.0, stroke(1.0, p.border));
    }

    /// Boutons d'action du panneau de détails (grille de 2 colonnes).
    fn details_actions(&self, ui: &mut Ui, p: &'static Palette, id: NodeId, path: &str, acts: &mut Vec<Act>) {
        let n = self.tree.node(id);
        let open_dir = id == Tree::ROOT || n.has(flag::GROUP);
        let dir = n.is_dir() && !n.has(flag::LINK);
        let mut items: Vec<(&Icon, &str, bool, Act, &str)> = vec![
            (&icons::EXTERNAL, "Explorateur", true, Act::Reveal(path.to_string(), open_dir), "Ouvrir dans l'Explorateur"),
            (&icons::INFO, "Propriétés", true, Act::Properties(path.to_string()), "Propriétés Windows"),
            (&icons::COPY, "Copier le chemin", true, Act::Copy(path.to_string(), format!("Chemin copié : {path}")), "Copier le chemin complet"),
        ];
        if dir {
            items.push((&icons::REFRESH, "Rescanner", !self.running(), Act::Rescan(id), "Analyser de nouveau ce dossier"));
            items.push((&icons::TREEMAP, "Voir la carte", true, Act::ShowInMap(id), "Carte proportionnelle de ce dossier"));
        }
        if id != Tree::ROOT && !n.has(flag::GROUP) {
            items.push((&icons::TRASH, "Corbeille…", self.can_trash(id), Act::AskTrash(id), "Envoyer à la corbeille (avec confirmation)"));
        }
        let cw = ((ui.available_width() - 6.0) / 2.0).floor();
        let mut it = items.into_iter().peekable();
        while it.peek().is_some() {
            let pair: Vec<_> = it.by_ref().take(2).collect();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (icon, label, enabled, act, tip) in pair {
                    let r = Button::new(Kind::Secondary, label).icon(icon).left().size(cw, 32.0).enabled(enabled).show(ui, p);
                    if r.on_hover_text(tip).clicked() {
                        acts.push(act);
                    }
                }
            });
            ui.add_space(6.0);
        }
    }

    // -- Confirmation d'envoi à la corbeille ----------------------------------

    pub fn confirm_dialog(&self, ctx: &egui::Context, p: &'static Palette, acts: &mut Vec<Act>) {
        let Some(id) = self.confirm_trash else { return };
        if !self.valid(id) || !self.can_trash(id) {
            acts.push(Act::CancelTrash);
            return;
        }
        let n = self.tree.node(id);
        let path = self.tree.path(id);
        let size = fmt::size(n.size.max(n.alloc));
        let text = if n.is_dir() {
            format!(
                "Ce dossier et tout son contenu ({}, {}, {}) seront déplacés dans la corbeille de Windows.",
                fmt::plural(n.files as u64, "fichier", "fichiers"),
                fmt::plural(n.dirs as u64, "sous-dossier", "sous-dossiers"),
                size
            )
        } else {
            format!("Ce fichier ({size}) sera déplacé dans la corbeille de Windows.")
        };
        let screen = ctx.screen_rect();
        // Fond assombri qui bloque le reste de la fenêtre.
        egui::Area::new(egui::Id::new("modal_bg")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(if p.dark { 150 } else { 90 }));
        });
        egui::Area::new(egui::Id::new("modal_trash")).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
            Frame::none()
                .fill(p.menu)
                .stroke(stroke(1.0, p.border_strong))
                .rounding(10.0)
                .shadow(ui.visuals().popup_shadow)
                .inner_margin(Margin::same(20.0))
                .show(ui, |ui| {
                    ui.set_width(440.0);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 18.0, p.err_soft);
                        icons::paint(ui, &icons::TRASH, Rect::from_center_size(r.center(), Vec2::splat(18.0)), p.err);
                        w::label(ui, "Envoyer à la corbeille ?", semibold(16.0), p.text);
                    });
                    Frame::none().fill(p.surface2).stroke(stroke(1.0, p.border)).rounding(6.0).inner_margin(Margin::symmetric(10.0, 8.0)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(Label::new(RichText::new(path.as_str()).font(mono(12.5)).color(p.text)).wrap());
                    });
                    ui.add(Label::new(RichText::new(text).font(regular(13.0)).color(p.text2)).wrap());
                    let mut more = "Vous pourrez le restaurer depuis la corbeille : rien n'est supprimé définitivement.".to_string();
                    if self.loaded {
                        more.push_str(" Les tailles indiquées datent de la dernière analyse.");
                    }
                    ui.add(Label::new(RichText::new(more).font(regular(12.0)).color(p.text3)).wrap());
                    ui.add_space(4.0);
                    ui.allocate_ui_with_layout(vec2(ui.available_width(), 34.0), Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if Button::new(Kind::Danger, "Envoyer à la corbeille").icon(&icons::TRASH).size(0.0, 34.0).font(semibold(13.0)).show(ui, p).clicked() {
                            acts.push(Act::ConfirmTrash);
                        }
                        if Button::new(Kind::Secondary, "Annuler").size(0.0, 34.0).font(regular(13.0)).show(ui, p).clicked() {
                            acts.push(Act::CancelTrash);
                        }
                    });
                });
        });
    }
}

// ---------------------------------------------------------------------------
// Fonctions d'interface
// ---------------------------------------------------------------------------

/// Menu contextuel d'un élément (clic droit), commun à toutes les vues.
fn item_menu(app: &DiskScanApp, ui: &mut Ui, p: &'static Palette, node: NodeId, path: &str, acts: &mut Vec<Act>) {
    ui.set_min_width(280.0);
    ui.spacing_mut().item_spacing = vec2(8.0, 0.0);

    fn item(ui: &mut Ui, p: &Palette, icon: Option<&Icon>, text: &str, right: Option<(&str, bool)>, act: Act, acts: &mut Vec<Act>) {
        if w::menu_item(ui, p, icon, text, right).clicked() {
            acts.push(act);
            ui.close_menu();
        }
    }

    let n = (node != NONE && app.valid(node)).then(|| app.tree.node(node));
    let is_drive = path.len() <= 3 && path.ends_with(":\\");
    let open_dir = is_drive || n.is_some_and(|n| n.has(flag::GROUP));
    w::menu_title(ui, p, path);
    item(ui, p, Some(&icons::EXTERNAL), "Ouvrir dans l'Explorateur", None, Act::Reveal(path.to_string(), open_dir), acts);
    item(ui, p, Some(&icons::COPY), "Copier le chemin", Some(("Ctrl+C", false)), Act::Copy(path.to_string(), format!("Chemin copié : {path}")), acts);
    if n.is_some() {
        item(ui, p, None, "Copier la ligne", Some(("Ctrl+Maj+C", false)), Act::Copy(app.node_fields(node).join("\t"), "Ligne copiée dans le presse-papiers.".into()), acts);
    }
    item(ui, p, Some(&icons::INFO), "Propriétés", None, Act::Properties(path.to_string()), acts);
    let Some(n) = n else { return };
    let dir = n.is_dir() && !n.has(flag::LINK);
    let in_tree = app.s.tab != Tab::Tree;
    let in_map = dir && (app.s.tab != Tab::Map || app.map_root != node);
    if in_tree || in_map || (dir && !app.running()) {
        w::menu_separator(ui, p);
    }
    if in_tree {
        item(ui, p, Some(&icons::LIST), "Afficher dans l'arborescence", None, Act::ShowInTree(node), acts);
    }
    if in_map {
        item(ui, p, Some(&icons::TREEMAP), "Afficher dans la carte", None, Act::ShowInMap(node), acts);
    }
    if dir && !app.running() {
        item(ui, p, Some(&icons::REFRESH), "Analyser de nouveau ce dossier", None, Act::Rescan(node), acts);
    }
    if app.can_trash(node) {
        w::menu_separator(ui, p);
        item(ui, p, Some(&icons::TRASH), "Envoyer à la corbeille…", Some(("Suppr", false)), Act::AskTrash(node), acts);
    }
}

fn paint_stat(ui: &Ui, p: &Palette, r: Rect, label: &str, value: &str, sub: &str) {
    let painter = ui.painter();
    painter.rect(r.shrink(0.5), 8.0, p.surface, stroke(1.0, p.border));
    let lg = w::galley(ui, label, regular(12.0), p.text3);
    painter.galley(pos2(r.left() + 14.0, r.top() + 11.0), lg, p.text3);
    let vg = w::galley(ui, value, semibold(22.0), p.text);
    let vh = vg.size().y;
    let vy = r.bottom() - 9.0 - vh;
    let x = r.left() + 14.0;
    let vw = vg.size().x;
    painter.galley(pos2(x, vy), vg, p.text);
    if !sub.is_empty() {
        let sg = w::galley_trunc(ui, sub, regular(12.0), p.text3, r.right() - 10.0 - (x + vw + 6.0));
        let sy = vy + vh * 0.78 - sg.size().y * 0.78;
        painter.galley(pos2(x + vw + 6.0, sy), sg, p.text3);
    }
}

fn field_row(ui: &mut Ui, p: &Palette, key: &str, value: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
    let cy = rect.center().y;
    let painter = ui.painter();
    painter.hline(rect.x_range(), rect.bottom() - 0.5, stroke(1.0, p.row_line));
    w::text_at(painter, w::galley(ui, key, regular(12.0), p.text3), rect.left(), cy);
    let vx = rect.left() + 108.0;
    let color = if value == "—" { p.text3 } else { p.text };
    w::text_at(painter, w::galley_trunc(ui, value, regular(12.5), color, rect.right() - vx), vx, cy);
}

/// Pastille « Dossier », « nouveau »…
fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    let g = w::galley(ui, text, semibold(12.0), fg);
    let (rect, _) = ui.allocate_exact_size(vec2(g.size().x + 18.0, 22.0), Sense::hover());
    ui.painter().rect_filled(rect, 11.0, bg);
    w::text_at(ui.painter(), g, rect.left() + 9.0, rect.center().y);
}

fn centered_text(ui: &mut Ui, text: &str, max_w: f32, color: Color32) {
    let width = max_w.min(ui.available_width());
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Center), |ui| {
        ui.set_max_width(width);
        ui.add(Label::new(RichText::new(text).font(regular(13.0)).color(color)).wrap());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colonnes() {
        let order = ColKind::DEFAULT_ORDER;
        let cols = layout_columns(0.0, 1000.0, &order, true);
        assert_eq!(cols.len(), 7);
        assert!(cols[0].w >= 240.0);
        // Sans comparaison, pas de colonne « Évolution ».
        assert!(!layout_columns(0.0, 1000.0, &order, false).iter().any(|c| c.kind == ColKind::Delta));
        // Étroit : les dossiers, puis la date disparaissent.
        let narrow = layout_columns(0.0, 560.0, &order, false);
        assert!(!narrow.iter().any(|c| c.kind == ColKind::Dirs));
        // Les colonnes se suivent sans se chevaucher.
        for pair in cols.windows(2) {
            assert!(pair[0].x + pair[0].w < pair[1].x);
        }
        let flat = flat_layout(0.0, 900.0, &FILES_COLS);
        assert_eq!(flat.len(), 4);
        assert!((flat[1].1 / flat[0].1 - 1.3).abs() < 1e-3);
    }
}
