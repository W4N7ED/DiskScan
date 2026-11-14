//! Fenêtre principale : état de l'application, analyse en cours, panneau de paramètres,
//! barre d'état, boutons de fenêtre. Les vues de la zone principale sont dans `views.rs`.

use crate::format as fmt;
use crate::history;
use crate::icons::{self, Icon};
use crate::scanner::{self, ScanEvent, ScanHandle};
use crate::theme::{self, mono, regular, semibold, stroke, Palette};
use crate::tree::{self, cmp_names, flag, Delta, NodeId, SortKey, Summary, Tree};
use crate::views::{ColKind, FlatRow};
use crate::widgets::{self as w, Button, Input, Kind};
use crate::win::{self, Drive};
use eframe::egui::{self, pos2, vec2, Align, Frame, Label, Layout, Margin, Rect, RichText, Sense, Ui, Vec2};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

const SHORTCUTS: &str = "F5 Lancer · Échap Arrêter / désélectionner · Clic droit : actions";
/// Nombre maximal de lignes affichées pour une recherche.
pub const MAX_HITS: usize = 2000;

// ---------------------------------------------------------------------------
// Réglages persistants
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
pub enum Tab {
    Tree,
    Map,
    Files,
    Exts,
    Ages,
    Changes,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub dark_mode: bool,
    pub target: String,
    /// Afficher la taille occupée sur le disque plutôt que la taille réelle.
    pub on_disk: bool,
    pub threads: usize,
    /// Montrer les fichiers dans l'arborescence (sinon : les dossiers seulement).
    pub show_files: bool,
    pub col_order: Vec<ColKind>,
    pub tab: Tab,
    pub win_size: Option<[f32; 2]>,
    pub win_pos: Option<[f32; 2]>,
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark_mode: false,
            target: String::new(),
            on_disk: false,
            threads: scanner::default_threads(),
            show_files: true,
            col_order: ColKind::DEFAULT_ORDER.to_vec(),
            tab: Tab::Tree,
            win_size: None,
            win_pos: None,
            maximized: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeSort {
    Name,
    Size,
    Files,
    Dirs,
    Modified,
    Delta,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MsgKind {
    Info,
    Spin,
    Ok,
    Warn,
    Err,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScanEnd {
    Done,
    Cancelled,
}

/// Actions demandées par l'interface pendant le dessin, appliquées en fin d'image.
pub enum Act {
    Copy(String, String),
    Select(NodeId),
    MenuTarget(NodeId),
    Deselect,
    Toggle(NodeId),
    Sort(TreeSort),
    SortFlat(usize),
    MoveCol(ColKind, ColKind),
    ResetCols,
    ClearFilter,
    SetTab(Tab),
    /// Ouvrir l'Explorateur sur ce chemin (vrai : c'est une racine, on l'ouvre au lieu de la sélectionner).
    Reveal(String, bool),
    Properties(String),
    Rescan(NodeId),
    AskTrash(NodeId),
    ConfirmTrash,
    CancelTrash,
    ShowInTree(NodeId),
    ShowInMap(NodeId),
}

/// Ligne affichée de l'arborescence.
#[derive(Clone, Copy)]
pub struct Row {
    pub id: NodeId,
    pub depth: u16,
}

// ---------------------------------------------------------------------------
// Application
// ---------------------------------------------------------------------------

pub struct DiskScanApp {
    pub s: Settings,
    pub drives: Vec<Drive>,
    /// Analyses enregistrées (une par lecteur ou dossier), de la plus récente à la plus ancienne.
    pub saved: Vec<history::SavedInfo>,
    drives_rx: Option<Receiver<Vec<Drive>>>,
    pub tree: Tree,
    /// Une analyse (en cours, terminée ou rechargée) est affichée.
    pub has_data: bool,
    /// L'analyse affichée vient du disque : dossiers seulement.
    pub loaded: bool,
    /// Numéros de dossiers du moteur → nœuds de l'arbre.
    map: Vec<NodeId>,
    pub scan: Option<ScanHandle>,
    /// L'analyse en cours ne porte que sur un dossier (clic droit → rescanner).
    pub rescan: Option<NodeId>,
    /// Analyse précédente (dossiers seulement), référence de la comparaison.
    prev: Option<Tree>,
    pub deltas: HashMap<NodeId, Delta>,
    /// L'analyse affichée a été comparée à une précédente (même sans aucun écart).
    pub compared: bool,
    pub summary: Summary,
    summary_dirty: bool,
    summary_at: Instant,
    /// Change à chaque modification des données : invalide les vues calculées.
    pub data_rev: u64,
    pub expanded: HashSet<NodeId>,
    pub rows: Vec<Row>,
    /// Nombre total de résultats de la recherche (les lignes sont plafonnées).
    pub matches: usize,
    pub rows_dirty: bool,
    rows_at: Instant,
    pub filter: String,
    pub sort: TreeSort,
    pub sort_asc: bool,
    /// Tri des tableaux simples, par onglet : (colonne, croissant).
    pub flat_sort: HashMap<Tab, (usize, bool)>,
    pub flat: Vec<FlatRow>,
    pub flat_key: Option<(Tab, u64, String, usize, bool, bool)>,
    pub selected: Option<NodeId>,
    pub scroll_to: Option<usize>,
    /// Dossier affiché par la carte proportionnelle.
    pub map_root: NodeId,
    pub map_cache: Option<((NodeId, u64, bool, [i32; 4]), Vec<crate::treemap::Item>)>,
    /// Élément dont l'envoi à la corbeille attend confirmation.
    pub confirm_trash: Option<NodeId>,
    frames: u32,
    stopping: bool,
    want_start: bool,
    toggle_theme: bool,
    pub drive_used: Option<u64>,
    started: Option<Instant>,
    pub elapsed: Duration,
    pub end: Option<ScanEnd>,
    /// Date de l'analyse affichée (secondes Unix).
    pub when: i64,
    message: Option<(MsgKind, String)>,
    summary_msg: Option<(MsgKind, String)>,
    target_cache: (String, Result<String, String>),
    /// Un menu contextuel était ouvert à l'image précédente (Échap le ferme sans désélectionner).
    pub menu_open: std::cell::Cell<bool>,
    menu_target: Option<NodeId>,
    /// Horodatage de la dernière entrée clavier traitée (egui peut rejouer une image en 2 passes).
    last_input_time: f64,
    #[cfg(debug_assertions)]
    script: Option<crate::demo::Script>,
}

fn now_unix() -> i64 {
    chrono::Local::now().timestamp()
}

/// Chemin absolu du dossier à analyser, ou la raison pour laquelle il ne convient pas.
pub fn normalize_target(input: &str) -> Result<String, String> {
    let t = input.trim().trim_matches('"').trim();
    if t.is_empty() {
        return Err("Indiquez un dossier ou choisissez un lecteur.".into());
    }
    let mut t = t.replace('/', "\\");
    if t.len() == 2 && t.ends_with(':') {
        t.push('\\');
    }
    let abs = std::path::absolute(&t).map_err(|_| "Chemin invalide.".to_string())?;
    let mut s = abs.to_string_lossy().to_string();
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    // Lettre de lecteur en majuscule : « c:\users » → « C:\users ».
    if s.as_bytes().get(1) == Some(&b':') {
        let first = s[..1].to_uppercase();
        s.replace_range(..1, &first);
    }
    match std::fs::metadata(&s) {
        Ok(m) if m.is_dir() => Ok(s),
        Ok(_) => Err("Ce chemin désigne un fichier, pas un dossier.".into()),
        Err(_) => Err("Dossier introuvable.".into()),
    }
}

impl DiskScanApp {
    pub fn new(cc: &eframe::CreationContext<'_>, cli_target: Option<String>, autostart: bool) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        theme::setup_fonts(&cc.egui_ctx);
        let stored: Option<Settings> = cc.storage.and_then(|st| eframe::get_value(st, eframe::APP_KEY));
        let first_run = stored.is_none();
        let mut s = stored.unwrap_or_default();
        if first_run {
            // Premier lancement : on suit le thème clair / sombre de Windows.
            s.dark_mode = win::system_prefers_dark().unwrap_or(false);
        }
        // Ordre des colonnes illisible ou incomplet (ancienne version) : celui du design.
        if s.col_order.len() != ColKind::DEFAULT_ORDER.len() || ColKind::DEFAULT_ORDER.iter().any(|k| !s.col_order.contains(k)) {
            s.col_order = ColKind::DEFAULT_ORDER.to_vec();
        }
        s.threads = s.threads.clamp(1, 32);
        restore_window(&cc.egui_ctx, &s);
        if let Some(t) = cli_target {
            s.target = t;
        }
        if s.target.trim().is_empty() {
            s.target = std::env::var("SystemDrive").map(|d| format!("{d}\\")).unwrap_or_else(|_| "C:\\".into());
        }
        theme::apply(&cc.egui_ctx, s.dark_mode);

        let mut app = Self {
            s,
            drives: Vec::new(),
            drives_rx: None,
            saved: Vec::new(),
            tree: Tree::new(""),
            has_data: false,
            loaded: false,
            map: Vec::new(),
            scan: None,
            rescan: None,
            prev: None,
            deltas: HashMap::new(),
            compared: false,
            summary: Summary::default(),
            summary_dirty: false,
            summary_at: Instant::now(),
            data_rev: 0,
            expanded: HashSet::new(),
            rows: Vec::new(),
            matches: 0,
            rows_dirty: false,
            rows_at: Instant::now(),
            filter: String::new(),
            sort: TreeSort::Size,
            sort_asc: false,
            flat_sort: HashMap::new(),
            flat: Vec::new(),
            flat_key: None,
            selected: None,
            scroll_to: None,
            map_root: Tree::ROOT,
            map_cache: None,
            confirm_trash: None,
            frames: 0,
            stopping: false,
            want_start: autostart,
            toggle_theme: false,
            drive_used: None,
            started: None,
            elapsed: Duration::ZERO,
            end: None,
            when: 0,
            message: None,
            summary_msg: None,
            target_cache: ("\0".into(), Err(String::new())),
            menu_open: std::cell::Cell::new(false),
            menu_target: None,
            last_input_time: -1.0,
            #[cfg(debug_assertions)]
            script: crate::demo::Script::from_env(),
        };
        app.refresh_drives(&cc.egui_ctx);
        app.saved = history::list();
        // Au lancement : l'analyse de la cible mémorisée, sinon la plus récente.
        let target = normalize_target(&app.s.target).ok();
        let root = target.filter(|t| app.saved_info(t).is_some()).or_else(|| app.saved.first().map(|i| i.root.clone()));
        if let Some(root) = root {
            app.show_saved(&root);
        }
        app
    }

    // -- Lecteurs -----------------------------------------------------------

    /// Relit la liste des lecteurs en arrière-plan (un lecteur réseau muet peut être lent).
    fn refresh_drives(&mut self, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            #[cfg(debug_assertions)]
            let list = crate::demo::fake_drives().unwrap_or_else(win::drives);
            #[cfg(not(debug_assertions))]
            let list = win::drives();
            let _ = tx.send(list);
            ctx.request_repaint();
        });
        self.drives_rx = Some(rx);
    }

    fn poll_drives(&mut self) {
        if let Some(rx) = &self.drives_rx {
            if let Ok(d) = rx.try_recv() {
                self.drives = d;
                self.drives_rx = None;
            }
        }
    }

    // -- Analyses enregistrées (disque) -------------------------------------

    /// Analyse enregistrée d'une racine, s'il y en a une.
    pub fn saved_info(&self, root: &str) -> Option<&history::SavedInfo> {
        let key = history::root_key(root);
        self.saved.iter().find(|i| history::root_key(&i.root) == key)
    }

    /// L'analyse affichée porte-t-elle sur cette racine ?
    pub fn showing(&self, root: &str) -> bool {
        self.has_data && history::root_key(&self.tree.root_path) == history::root_key(root)
    }

    /// Affiche l'analyse enregistrée d'une racine : consultable (dossiers seulement), elle sert
    /// aussi de référence pour repérer ce qui a grossi ou diminué à l'analyse suivante.
    fn show_saved(&mut self, root: &str) -> bool {
        let Some(saved) = history::load(root) else {
            // Fichier disparu ou illisible : la tuile ne doit plus l'annoncer.
            let key = history::root_key(root);
            self.saved.retain(|i| history::root_key(&i.root) != key);
            return false;
        };
        self.expanded.clear();
        self.selected = None;
        self.menu_target = None;
        self.confirm_trash = None;
        self.map_root = Tree::ROOT;
        self.map_cache = None;
        self.prev = None;
        self.drive_used = None;
        self.tree = saved.tree;
        self.summary = saved.summary;
        self.deltas = saved.deltas;
        self.compared = saved.compared;
        self.when = saved.when;
        self.elapsed = Duration::from_millis(saved.elapsed_ms);
        self.end = Some(if saved.cancelled { ScanEnd::Cancelled } else { ScanEnd::Done });
        self.has_data = true;
        self.loaded = true;
        self.expanded.insert(Tree::ROOT);
        self.rows_dirty = true;
        self.data_rev += 1;
        self.summary_msg = Some((
            MsgKind::Info,
            format!(
                "Analyse de {} du {} rechargée : dossiers seulement. Relancez l'analyse (F5) pour le détail des fichiers.",
                self.tree.root_path,
                fmt::date_time(self.when)
            ),
        ));
        true
    }

    fn save_last_scan(&mut self) {
        let root = self.tree.root_path.clone();
        let content = history::encode(&self.tree, &self.summary, &self.deltas, self.compared, self.when, self.elapsed.as_millis() as u64, false);
        let r = self.tree.node(Tree::ROOT);
        let info = history::SavedInfo { root: root.clone(), when: self.when, size: r.size, alloc: r.alloc };
        let key = history::root_key(&root);
        self.saved.retain(|i| history::root_key(&i.root) != key);
        self.saved.insert(0, info);
        std::thread::spawn(move || history::save(&root, content));
    }

    /// Mémorise taille, position et état « agrandi » pour le prochain lancement.
    fn track_window(&mut self, ctx: &egui::Context) {
        self.frames = self.frames.saturating_add(1);
        if self.frames < 10 {
            return; // la restauration de la fenêtre n'est pas encore appliquée
        }
        let vp = ctx.input(|i| i.viewport().clone());
        if vp.minimized == Some(true) {
            return;
        }
        let maximized = vp.maximized.unwrap_or(false);
        self.s.maximized = maximized;
        if !maximized && vp.fullscreen != Some(true) {
            if let Some(r) = vp.inner_rect.filter(|r| r.width() >= 200.0 && r.height() >= 200.0) {
                self.s.win_size = Some([r.width(), r.height()]);
            }
            if let Some(r) = vp.outer_rect {
                self.s.win_pos = Some([r.left(), r.top()]);
            }
        }
    }

    // -- État dérivé --------------------------------------------------------

    fn target_status(&mut self) -> Result<String, String> {
        if self.target_cache.0 != self.s.target {
            self.target_cache = (self.s.target.clone(), normalize_target(&self.s.target));
        }
        self.target_cache.1.clone()
    }

    pub fn running(&self) -> bool {
        self.scan.is_some()
    }

    /// Écart d'un dossier avec l'analyse précédente, dans l'unité affichée.
    pub fn delta_val(&self, id: NodeId) -> i64 {
        self.deltas.get(&id).map_or(0, |d| d.get(self.s.on_disk))
    }

    /// Part d'un nœud dans son parent (1 pour la racine).
    pub fn share(&self, id: NodeId) -> f64 {
        let n = self.tree.node(id);
        if n.parent == tree::NONE {
            return 1.0;
        }
        let total = self.tree.node(n.parent).weight(self.s.on_disk);
        if total == 0 {
            0.0
        } else {
            n.weight(self.s.on_disk) as f64 / total as f64
        }
    }

    fn status_line(&self) -> (MsgKind, String) {
        if let Some(m) = &self.message {
            return m.clone();
        }
        if self.running() {
            if self.stopping {
                return (MsgKind::Spin, "Arrêt en cours…".into());
            }
            let (what, id) = match self.rescan {
                Some(id) => ("Nouvelle analyse de", id),
                None => ("Analyse de", Tree::ROOT),
            };
            let r = self.tree.node(Tree::ROOT);
            return (
                MsgKind::Spin,
                format!(
                    "{what} {} — {} et {} parcourus, {}",
                    self.tree.path(id),
                    fmt::plural(r.files as u64, "fichier", "fichiers"),
                    fmt::plural(r.dirs as u64, "dossier", "dossiers"),
                    fmt::size(r.weight(self.s.on_disk))
                ),
            );
        }
        match &self.summary_msg {
            Some(m) => m.clone(),
            None => (MsgKind::Info, "Prêt. Choisissez un lecteur ou un dossier, puis lancez l'analyse.".into()),
        }
    }

    /// « 3 dossiers ont grossi, 1 a diminué, 2 nouveaux » (vide sans comparaison).
    fn changes_text(&self) -> String {
        let (mut up, mut down, mut new) = (0u64, 0u64, 0u64);
        for d in self.deltas.values() {
            if d.new {
                new += 1;
            } else if d.get(self.s.on_disk) > 0 {
                up += 1;
            } else if d.get(self.s.on_disk) < 0 {
                down += 1;
            }
        }
        let mut parts = Vec::new();
        if up > 0 {
            parts.push(fmt::plural(up, "dossier a grossi", "dossiers ont grossi"));
        }
        if down > 0 {
            parts.push(fmt::plural(down, "a diminué", "ont diminué"));
        }
        if new > 0 {
            parts.push(fmt::plural(new, "nouveau", "nouveaux"));
        }
        parts.join(", ")
    }

    // -- Analyse ------------------------------------------------------------

    fn start_scan(&mut self, ctx: &egui::Context) {
        if self.running() {
            return;
        }
        let root = match self.target_status() {
            Ok(r) => r,
            Err(e) => {
                self.message = Some((MsgKind::Err, e));
                return;
            }
        };
        // Référence de comparaison : l'analyse affichée si elle porte sur la même cible,
        // sinon l'analyse enregistrée de cette cible, sinon l'analyse affichée (si l'une
        // des deux racines contient l'autre, la comparaison porte sur la partie commune).
        let complete = self.has_data && self.end == Some(ScanEnd::Done);
        self.prev = if complete && self.showing(&root) {
            Some(self.tree.dirs_only())
        } else if let Some(saved) = history::load(&root) {
            Some(saved.tree)
        } else if complete {
            Some(self.tree.dirs_only())
        } else {
            None
        };
        self.tree = Tree::new(&root);
        self.map = vec![Tree::ROOT];
        self.has_data = true;
        self.loaded = false;
        self.expanded.clear();
        self.expanded.insert(Tree::ROOT);
        self.selected = None;
        self.menu_target = None;
        self.confirm_trash = None;
        self.deltas.clear();
        self.compared = false;
        self.summary = Summary::default();
        self.rows.clear();
        self.rows_dirty = true;
        self.map_root = Tree::ROOT;
        self.map_cache = None;
        self.data_rev += 1;
        self.drive_used = self.drives.iter().find(|d| d.root.eq_ignore_ascii_case(&root)).map(|d| d.used());
        let c = ctx.clone();
        self.scan = Some(scanner::start_scan(root, self.s.threads, move || c.request_repaint()));
        self.rescan = None;
        self.started = Some(Instant::now());
        self.elapsed = Duration::ZERO;
        self.end = None;
        self.when = now_unix();
        self.stopping = false;
        self.message = None;
        self.summary_msg = None;
    }

    fn stop_scan(&mut self) {
        if let Some(scan) = &self.scan {
            scan.cancel();
            self.stopping = true;
        }
    }

    /// Analyse de nouveau un seul dossier, sans toucher au reste de l'arbre.
    fn start_rescan(&mut self, ctx: &egui::Context, id: NodeId) {
        if self.running() || !self.has_data {
            return;
        }
        if id == Tree::ROOT {
            self.s.target = self.tree.root_path.clone();
            self.start_scan(ctx);
            return;
        }
        let n = self.tree.node(id);
        if !n.is_dir() || n.has(flag::LINK) {
            return;
        }
        let path = self.tree.path(id);
        if self.selected.is_some_and(|s| s != id && self.tree.is_under(s, id)) {
            self.selected = None;
        }
        if self.map_root != id && self.tree.is_under(self.map_root, id) {
            self.map_root = id;
        }
        self.tree.reset_dir(id);
        self.map = vec![id];
        self.expanded.insert(id);
        self.rows_dirty = true;
        self.data_rev += 1;
        let c = ctx.clone();
        self.scan = Some(scanner::start_scan(path, self.s.threads, move || c.request_repaint()));
        self.rescan = Some(id);
        self.started = Some(Instant::now());
        self.stopping = false;
        self.message = None;
    }

    /// Intègre les dossiers lus par le moteur. Renvoie vrai s'il en reste à traiter.
    fn poll_scan(&mut self) -> bool {
        let Some(scan) = &self.scan else { return false };
        let deadline = Instant::now() + Duration::from_millis(25);
        let mut finished = None;
        let mut got = 0u32;
        let mut more = false;
        while let Some(ev) = scan.try_recv() {
            match ev {
                ScanEvent::Dir(batch) => {
                    self.tree.apply_batch(&mut self.map, batch);
                    got += 1;
                }
                ScanEvent::Finished { cancelled } => {
                    finished = Some(cancelled);
                    break;
                }
            }
            if got % 64 == 0 && Instant::now() >= deadline {
                more = true;
                break;
            }
        }
        if self.rescan.is_none() {
            if let Some(start) = self.started {
                self.elapsed = start.elapsed();
            }
        }
        if got > 0 {
            self.rows_dirty = true;
            self.summary_dirty = true;
        }
        if let Some(cancelled) = finished {
            self.finish_scan(cancelled);
        }
        more
    }

    fn finish_scan(&mut self, cancelled: bool) {
        self.scan = None;
        self.stopping = false;
        let rescan = self.rescan.take();
        if rescan.is_none() {
            self.end = Some(if cancelled { ScanEnd::Cancelled } else { ScanEnd::Done });
        }
        self.refresh_summary();
        let complete = !cancelled && self.end == Some(ScanEnd::Done) && !self.loaded;
        if complete {
            let cmp = self.prev.as_ref().and_then(|p| tree::compare(p, &self.tree));
            self.compared = cmp.is_some();
            self.deltas = cmp.unwrap_or_default();
            self.save_last_scan();
        }
        self.rows_dirty = true;
        self.data_rev += 1;
        let r = self.tree.node(Tree::ROOT);
        let mut text = match (rescan, cancelled) {
            (Some(id), false) => {
                let n = self.tree.node(id);
                format!(
                    "Dossier analysé de nouveau : {} — {} dans {}.",
                    n.name,
                    fmt::size(n.weight(self.s.on_disk)),
                    fmt::plural(n.files as u64, "fichier", "fichiers")
                )
            }
            (Some(_), true) => "Nouvelle analyse du dossier interrompue : son contenu est incomplet.".to_string(),
            (None, false) => format!(
                "Analyse terminée en {} s : {} dans {} et {}.",
                fmt::secs(self.elapsed),
                fmt::size(r.weight(self.s.on_disk)),
                fmt::plural(r.files as u64, "fichier", "fichiers"),
                fmt::plural(r.dirs as u64, "dossier", "dossiers")
            ),
            (None, true) => format!("Analyse interrompue après {} s : résultats partiels, non enregistrés.", fmt::secs(self.elapsed)),
        };
        if rescan.is_some() {
            self.summary_msg = Some((if cancelled { MsgKind::Warn } else { MsgKind::Ok }, text));
            return;
        }
        if !cancelled && r.denied > 0 {
            text.push_str(&format!(" {}.", fmt::plural(r.denied as u64, "dossier illisible", "dossiers illisibles")));
        }
        let changes = self.changes_text();
        if complete && !changes.is_empty() {
            text.push_str(&format!(" Depuis la dernière analyse : {changes}."));
        } else if complete && self.compared {
            text.push_str(" Aucun dossier n'a changé depuis la dernière analyse.");
        }
        self.summary_msg = Some((if cancelled { MsgKind::Warn } else { MsgKind::Ok }, text));
    }

    /// Recalcule les vues complémentaires (plus gros fichiers, extensions, ancienneté).
    fn refresh_summary(&mut self) {
        self.summary_dirty = false;
        self.summary_at = Instant::now();
        // Une analyse rechargée n'a plus ses fichiers : ses vues viennent du disque.
        if !self.loaded && self.has_data {
            self.summary = self.tree.summarize(now_unix(), self.s.on_disk);
            self.data_rev += 1;
        }
    }

    // -- Lignes de l'arborescence -------------------------------------------

    fn cmp_nodes(&self, a: NodeId, b: NodeId) -> Ordering {
        let (x, y) = (self.tree.node(a), self.tree.node(b));
        let d = self.s.on_disk;
        let v = match self.sort {
            TreeSort::Name => Ordering::Equal,
            TreeSort::Size => x.weight(d).cmp(&y.weight(d)),
            TreeSort::Files => x.files.cmp(&y.files),
            TreeSort::Dirs => x.dirs.cmp(&y.dirs),
            TreeSort::Modified => x.mtime.cmp(&y.mtime),
            TreeSort::Delta => self.delta_val(a).cmp(&self.delta_val(b)),
        };
        let v = if self.sort_asc { v } else { v.reverse() };
        v.then_with(|| {
            let n = cmp_names(&x.name, &y.name);
            if self.sort == TreeSort::Name && !self.sort_asc {
                n.reverse()
            } else {
                n
            }
        })
    }

    /// Enfants d'un dossier dans l'ordre d'affichage.
    fn sorted(&self, id: NodeId) -> Vec<NodeId> {
        let key = match self.sort {
            TreeSort::Name => SortKey::Name,
            TreeSort::Size => SortKey::Size,
            TreeSort::Files => SortKey::Files,
            TreeSort::Dirs => SortKey::Dirs,
            TreeSort::Modified => SortKey::Modified,
            TreeSort::Delta => {
                let mut ids = self.tree.sorted_children(id, SortKey::Name, true, self.s.on_disk, self.s.show_files);
                ids.sort_by(|&a, &b| self.cmp_nodes(a, b));
                return ids;
            }
        };
        self.tree.sorted_children(id, key, self.sort_asc, self.s.on_disk, self.s.show_files)
    }

    fn rebuild_rows(&mut self) {
        self.rows_dirty = false;
        self.rows_at = Instant::now();
        self.matches = 0;
        let mut rows = Vec::new();
        if !self.has_data {
            self.rows = rows;
            return;
        }
        let f = self.filter.trim().to_lowercase();
        if f.is_empty() {
            let mut stack = vec![(Tree::ROOT, 0u16)];
            while let Some((id, depth)) = stack.pop() {
                rows.push(Row { id, depth });
                if self.expanded.contains(&id) && self.tree.node(id).is_dir() {
                    let kids = self.sorted(id);
                    stack.extend(kids.into_iter().rev().map(|c| (c, depth + 1)));
                }
            }
        } else {
            // Recherche : liste à plat de tout ce dont le nom contient le texte.
            let mut hits = Vec::new();
            let mut stack = vec![Tree::ROOT];
            while let Some(id) = stack.pop() {
                for c in self.tree.children(id) {
                    let n = self.tree.node(c);
                    if n.has(flag::GROUP) {
                        continue;
                    }
                    if (n.is_dir() || self.s.show_files) && n.name.to_lowercase().contains(&f) {
                        hits.push(c);
                    }
                    if n.is_dir() {
                        stack.push(c);
                    }
                }
            }
            self.matches = hits.len();
            hits.sort_by(|&a, &b| self.cmp_nodes(a, b));
            hits.truncate(MAX_HITS);
            rows = hits.into_iter().map(|id| Row { id, depth: 0 }).collect();
        }
        self.rows = rows;
    }

    fn toggle(&mut self, id: NodeId) {
        if !self.tree.node(id).is_dir() {
            return;
        }
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
        self.rows_dirty = true;
    }

    fn move_selection(&mut self, delta: i32) {
        if self.s.tab != Tab::Tree || self.rows.is_empty() {
            return;
        }
        let cur = self.selected.and_then(|s| self.rows.iter().position(|r| r.id == s));
        let next = match cur {
            None => 0,
            Some(c) => (c as i32 + delta).clamp(0, self.rows.len() as i32 - 1) as usize,
        };
        self.selected = Some(self.rows[next].id);
        self.scroll_to = Some(next);
    }

    /// Montre un nœud dans l'arborescence : onglet, dossiers parents dépliés, sélection.
    fn show_in_tree(&mut self, id: NodeId) {
        self.s.tab = Tab::Tree;
        self.filter.clear();
        let mut cur = self.tree.node(id).parent;
        while cur != tree::NONE {
            self.expanded.insert(cur);
            cur = self.tree.node(cur).parent;
        }
        // Un fichier n'apparaît que si les fichiers sont affichés.
        if !self.tree.node(id).is_dir() {
            self.s.show_files = true;
        }
        self.rebuild_rows();
        self.selected = Some(id);
        self.scroll_to = self.rows.iter().position(|r| r.id == id);
    }

    // -- Export -------------------------------------------------------------

    fn export_csv(&mut self) {
        let (headers, rows) = self.export_table();
        let name = format!("diskscan_{}.csv", chrono::Local::now().format("%Y-%m-%d_%H%M"));
        let Some(path) = rfd::FileDialog::new().set_title("Exporter le tableau").add_filter("CSV (séparateur ;)", &["csv"]).set_file_name(name).save_file() else {
            return;
        };
        self.message = Some(match std::fs::write(&path, fmt::csv(&headers, &rows)) {
            Ok(()) => (MsgKind::Ok, format!("Export CSV enregistré : {}", path.display())),
            Err(e) => (MsgKind::Err, format!("Export échoué : {e}")),
        });
    }

    fn clear_results(&mut self) {
        self.tree = Tree::new("");
        self.has_data = false;
        self.loaded = false;
        self.prev = None;
        self.deltas.clear();
        self.compared = false;
        self.summary = Summary::default();
        self.rows.clear();
        self.flat.clear();
        self.flat_key = None;
        self.expanded.clear();
        self.selected = None;
        self.map_cache = None;
        self.map_root = Tree::ROOT;
        self.elapsed = Duration::ZERO;
        self.started = None;
        self.end = None;
        self.summary_msg = None;
        self.data_rev += 1;
        history::delete(&self.tree.root_path);
        let key = history::root_key(&self.tree.root_path);
        self.saved.retain(|i| history::root_key(&i.root) != key);
        self.message = Some((MsgKind::Info, "Résultats effacés.".into()));
    }

    // -- Actions ------------------------------------------------------------

    fn apply_act(&mut self, ctx: &egui::Context, act: Act) {
        match act {
            Act::Copy(text, msg) => {
                ctx.copy_text(text);
                self.message = Some((MsgKind::Ok, msg));
            }
            Act::Select(id) => {
                self.selected = Some(id);
                self.menu_target = None;
            }
            Act::MenuTarget(id) => self.menu_target = Some(id),
            Act::Deselect => self.selected = None,
            Act::Toggle(id) => self.toggle(id),
            Act::Sort(k) => {
                if self.sort == k {
                    self.sort_asc = !self.sort_asc;
                } else {
                    self.sort = k;
                    // Les nombres commencent par le plus grand, les noms par A.
                    self.sort_asc = k == TreeSort::Name;
                }
                self.rows_dirty = true;
            }
            Act::SortFlat(col) => {
                let cur = self.flat_sort.get(&self.s.tab).copied();
                let next = match cur {
                    Some((c, asc)) if c == col => (col, !asc),
                    _ => (col, col == 0),
                };
                self.flat_sort.insert(self.s.tab, next);
            }
            Act::ClearFilter => {
                self.filter.clear();
                self.rows_dirty = true;
            }
            Act::MoveCol(src, dst) => {
                let order = &mut self.s.col_order;
                if let (Some(si), Some(di)) = (order.iter().position(|k| *k == src), order.iter().position(|k| *k == dst)) {
                    order.remove(si);
                    order.insert(di, src);
                }
            }
            Act::ResetCols => self.s.col_order = ColKind::DEFAULT_ORDER.to_vec(),
            Act::SetTab(t) => {
                self.s.tab = t;
                if matches!(t, Tab::Files | Tab::Exts | Tab::Ages | Tab::Map) && self.summary_dirty {
                    self.refresh_summary();
                }
            }
            Act::Reveal(path, is_root) => {
                if let Err(e) = win::reveal(&path, is_root) {
                    self.message = Some((MsgKind::Err, format!("Impossible d'ouvrir l'Explorateur : {e}")));
                }
            }
            Act::Properties(path) => {
                if let Err(e) = win::properties(&path) {
                    self.message = Some((MsgKind::Err, e));
                }
            }
            Act::Rescan(id) => self.start_rescan(ctx, id),
            Act::AskTrash(id) => {
                if self.can_trash(id) {
                    self.confirm_trash = Some(id);
                }
            }
            Act::CancelTrash => self.confirm_trash = None,
            Act::ConfirmTrash => {
                if let Some(id) = self.confirm_trash.take() {
                    self.do_trash(ctx, id);
                }
            }
            Act::ShowInTree(id) => self.show_in_tree(id),
            Act::ShowInMap(id) => {
                self.map_root = id;
                self.s.tab = Tab::Map;
                if self.summary_dirty {
                    self.refresh_summary();
                }
            }
        }
        ctx.request_repaint();
    }

    /// L'élément peut-il être envoyé à la corbeille ? (ni la racine, ni pendant une analyse)
    pub fn can_trash(&self, id: NodeId) -> bool {
        let n = self.tree.node(id);
        self.has_data && !self.running() && id != Tree::ROOT && !n.has(flag::GROUP) && n.parent != tree::NONE
    }

    fn do_trash(&mut self, ctx: &egui::Context, id: NodeId) {
        if !self.can_trash(id) {
            return;
        }
        let path = self.tree.path(id);
        let size = fmt::size(self.tree.node(id).weight(self.s.on_disk));
        match win::trash(&path) {
            Ok(()) => {
                if self.selected.is_some_and(|s| self.tree.is_under(s, id)) {
                    self.selected = None;
                }
                if self.tree.is_under(self.map_root, id) {
                    self.map_root = Tree::ROOT;
                }
                self.tree.remove(id);
                self.rows_dirty = true;
                self.data_rev += 1;
                self.refresh_summary();
                if !self.loaded && self.end == Some(ScanEnd::Done) {
                    self.save_last_scan();
                }
                self.refresh_drives(ctx);
                self.message = Some((MsgKind::Ok, format!("Envoyé à la corbeille : {path} ({size}).")));
            }
            Err(e) => self.message = Some((MsgKind::Err, e)),
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Une seule fois par image, même si egui exécute une seconde passe.
        let time = ctx.input(|i| i.time);
        if time == self.last_input_time {
            return;
        }
        self.last_input_time = time;
        let typing = ctx.wants_keyboard_input();
        let (f5, esc, copy, shift, up, down, left, right, enter, del) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::F5),
                i.key_pressed(egui::Key::Escape),
                i.events.iter().any(|e| matches!(e, egui::Event::Copy)) || (i.modifiers.command && i.key_pressed(egui::Key::C)),
                i.modifiers.shift,
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Delete),
            )
        });
        // Confirmation ouverte : seule Échap agit (Entrée ne valide jamais une mise à la corbeille).
        if self.confirm_trash.is_some() {
            if esc {
                self.confirm_trash = None;
            }
            return;
        }
        if f5 && !self.running() {
            self.want_start = true;
        }
        if esc {
            if self.running() {
                self.stop_scan();
            } else if !typing && !self.menu_open.get() {
                self.selected = None;
            }
        }
        if typing {
            return;
        }
        if copy {
            if let Some(id) = self.selected {
                let act = if shift {
                    Act::Copy(self.node_fields(id).join("\t"), "Ligne copiée dans le presse-papiers.".into())
                } else {
                    let path = self.tree.path(id);
                    Act::Copy(path.clone(), format!("Chemin copié : {path}"))
                };
                self.apply_act(ctx, act);
            }
        }
        if up || down {
            self.move_selection(if down { 1 } else { -1 });
        }
        if self.s.tab == Tab::Tree && self.filter.trim().is_empty() {
            if let Some(id) = self.selected {
                let open = self.expanded.contains(&id);
                let is_dir = self.tree.node(id).is_dir();
                if (enter && is_dir) || (right && is_dir && !open) || (left && open) {
                    self.toggle(id);
                } else if right && open {
                    self.rebuild_rows();
                    self.move_selection(1);
                } else if left {
                    let parent = self.tree.node(id).parent;
                    if parent != tree::NONE {
                        self.selected = Some(parent);
                        self.scroll_to = self.rows.iter().position(|r| r.id == parent);
                    }
                }
            }
        }
        if del {
            if let Some(id) = self.selected {
                self.apply_act(ctx, Act::AskTrash(id));
            }
        }
    }

    // =======================================================================
    // Barre d'état
    // =======================================================================

    fn status_bar(&self, ctx: &egui::Context, p: &'static Palette) {
        let (kind, text) = self.status_line();
        let (bg, fg, icon) = match kind {
            MsgKind::Info => (p.sidebar, p.accent_text, Some(&icons::INFO)),
            MsgKind::Spin => (p.sidebar, p.accent_text, None),
            MsgKind::Ok => (p.ok_soft, p.ok, Some(&icons::CHECK_CIRCLE)),
            MsgKind::Warn => (p.warn_soft, p.warn, Some(&icons::WARNING)),
            MsgKind::Err => (p.err_soft, p.err, Some(&icons::X_CIRCLE)),
        };
        egui::TopBottomPanel::bottom("status")
            .exact_height(28.0)
            .frame(Frame::none().fill(bg).inner_margin(Margin::symmetric(12.0, 0.0)))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let cy = rect.center().y;
                let ir = w::icon_rect(rect.left(), cy, 14.0);
                match icon {
                    Some(ic) => icons::paint(ui, ic, ir, fg),
                    None => egui::Spinner::new().size(14.0).color(fg).paint_at(ui, ir),
                }
                let sg = w::galley(ui, SHORTCUTS, regular(12.0), p.text3);
                let sw = sg.size().x;
                let full = w::galley(ui, text.as_str(), regular(12.0), p.text).size().x;
                // Les raccourcis s'effacent devant un message long.
                let show_shortcuts = rect.width() > sw + full + 60.0;
                if show_shortcuts {
                    w::text_at(ui.painter(), sg, rect.right() - sw, cy);
                }
                let max = rect.width() - 22.0 - if show_shortcuts { sw + 16.0 } else { 0.0 };
                let g = w::galley_trunc(ui, &text, regular(12.0), p.text, max);
                w::text_at(ui.painter(), g, rect.left() + 22.0, cy);
            });
    }

    // =======================================================================
    // Panneau de paramètres (gauche)
    // =======================================================================

    fn sidebar(&mut self, ctx: &egui::Context, p: &'static Palette) {
        egui::SidePanel::left("sidebar")
            .exact_width(300.0)
            .resizable(false)
            .frame(Frame::none().fill(p.sidebar))
            .show(ctx, |ui| {
                egui::TopBottomPanel::top("sidebar_head")
                    .frame(Frame::none().fill(p.sidebar).inner_margin(Margin { left: 16.0, right: 12.0, top: 12.0, bottom: 12.0 }))
                    .show_inside(ui, |ui| self.sidebar_head(ui, p));
                egui::TopBottomPanel::bottom("sidebar_foot")
                    .frame(Frame::none().fill(p.sidebar).inner_margin(Margin { left: 16.0, right: 16.0, top: 12.0, bottom: 16.0 }))
                    .show_inside(ui, |ui| self.sidebar_foot(ui, p));
                egui::CentralPanel::default().frame(Frame::none().fill(p.sidebar)).show_inside(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("sidebar_scroll").auto_shrink(false).show(ui, |ui| {
                        Frame::none().inner_margin(Margin { left: 16.0, right: 16.0, top: 14.0, bottom: 14.0 }).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            self.sidebar_body(ui, p);
                        });
                    });
                });
            });
    }

    fn sidebar_head(&mut self, ui: &mut Ui, p: &'static Palette) {
        // L'en-tête sert aussi de poignée pour déplacer la fenêtre.
        window_drag(ui, ui.max_rect().expand2(vec2(16.0, 12.0)), "drag_sidebar");
        ui.style_mut().interaction.selectable_labels = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            w::paint_logo(ui.painter(), r, p.accent);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                w::label(ui, "DiskScan", semibold(15.0), p.text);
                w::label(ui, "Analyseur d'espace disque", regular(11.0), p.text3);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (icon, tip) = if self.s.dark_mode { (&icons::SUN, "Passer au thème clair") } else { (&icons::MOON, "Passer au thème sombre") };
                let r = Button::new(Kind::Secondary, "").icon(icon).icon_size(16.0).size(32.0, 32.0).show(ui, p);
                if r.on_hover_text(tip).clicked() {
                    self.toggle_theme = true;
                }
            });
        });
    }

    fn sidebar_body(&mut self, ui: &mut Ui, p: &'static Palette) {
        let running = self.running();
        let target_st = self.target_status();
        let full_w = ui.available_width();
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.style_mut().interaction.selectable_labels = false;

        if running {
            lock_banner(ui, p);
            ui.add_space(10.0);
        }

        let mut refresh = false;
        let mut show: Option<String> = None;
        ui.scope(|ui| {
            // Les champs verrouillés s'estompent vers le fond du panneau.
            ui.visuals_mut().widgets.noninteractive.weak_bg_fill = p.sidebar;
            ui.add_enabled_ui(!running, |ui| {
                // --- Dossier à analyser ---
                w::field_label(ui, p, "Dossier à analyser");
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let out = w::input(
                        ui,
                        p,
                        Input {
                            text: &mut self.s.target,
                            id: "target",
                            hint: r"C:\Users, D:\",
                            width: full_w - 40.0,
                            height: 34.0,
                            mono: true,
                            error: target_st.is_err(),
                            search: false,
                            clearable: false,
                            highlight: false,
                        },
                    );
                    if out.response.changed() {
                        self.message = None;
                    }
                    if out.submitted {
                        self.want_start = true;
                    }
                    // Chemin plus long que le champ : on le montre en entier au survol.
                    if self.s.target.len() > 26 && !out.response.has_focus() {
                        out.response.on_hover_text(self.s.target.as_str());
                    }
                    let r = Button::new(Kind::Secondary, "").icon(&icons::FOLDER).icon_size(15.0).size(34.0, 34.0).show(ui, p);
                    if r.on_hover_text("Choisir un dossier…").clicked() {
                        let mut dialog = rfd::FileDialog::new().set_title("Dossier à analyser");
                        if let Ok(dir) = &target_st {
                            dialog = dialog.set_directory(dir);
                        }
                        if let Some(dir) = dialog.pick_folder() {
                            self.s.target = dir.to_string_lossy().to_string();
                            self.message = None;
                        }
                    }
                });
                match &target_st {
                    Ok(path) => {
                        let drive = self.drives.iter().any(|d| d.root.eq_ignore_ascii_case(path));
                        match self.saved_info(path).map(|i| i.when) {
                            Some(when) => {
                                let what = if drive { "Lecteur analysé" } else { "Dossier analysé" };
                                let text = format!("{what} le {}", fmt::date_time(when));
                                ui.horizontal(|ui| {
                                    hint_line(ui, Some((&icons::CHECK_CIRCLE, p.ok)), &text, p.ok);
                                    if !self.showing(path) && !running {
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            if Button::new(Kind::Link, "Afficher").font(regular(12.0)).size(0.0, 20.0).show(ui, p).clicked() {
                                                show = Some(path.clone());
                                            }
                                        });
                                    }
                                });
                            }
                            None => {
                                let what = if drive { "Lecteur entier, jamais analysé" } else { "Dossier jamais analysé" };
                                hint_line(ui, Some((&icons::CHECK, p.text3)), what, p.text3);
                            }
                        }
                    }
                    Err(e) => hint_line(ui, Some((&icons::ALERT_CIRCLE, p.err)), e, p.err),
                }
                ui.add_space(10.0);

                // --- Lecteurs ---
                ui.horizontal(|ui| {
                    w::field_label(ui, p, "Lecteurs");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let r = Button::new(Kind::Ghost, "").icon(&icons::REFRESH).icon_size(13.0).size(24.0, 22.0).show(ui, p);
                        if r.on_hover_text("Rafraîchir la liste des lecteurs").clicked() {
                            refresh = true;
                        }
                    });
                });
                if self.drives.is_empty() {
                    let text = if self.drives_rx.is_some() { "Lecture des lecteurs…" } else { "Aucun lecteur détecté." };
                    w::label(ui, text, regular(12.5), p.text3);
                }
                let current = target_st.as_deref().unwrap_or("").to_string();
                let mut picked = None;
                for d in &self.drives {
                    let state = DriveState { selected: d.root.eq_ignore_ascii_case(&current), shown: self.showing(&d.root), when: self.saved_info(&d.root).map(|i| i.when) };
                    if drive_card(ui, p, d, state).clicked() {
                        picked = Some(d.root.clone());
                    }
                }
                if let Some(root) = picked {
                    self.s.target = root.clone();
                    self.message = None;
                    // Lecteur déjà analysé : on affiche directement sa dernière analyse.
                    if !self.showing(&root) && self.saved_info(&root).is_some() {
                        show = Some(root);
                    }
                }
            });
        });
        if refresh {
            self.refresh_drives(ui.ctx());
        }
        if let Some(root) = show {
            if !self.running() && self.show_saved(&root) {
                self.s.tab = if self.s.tab == Tab::Changes && !self.compared { Tab::Tree } else { self.s.tab };
            }
        }
        ui.add_space(10.0);

        // --- Affichage (modifiable même pendant l'analyse) ---
        let y = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), y, stroke(1.0, p.border));
        ui.add_space(12.0);
        w::field_label(ui, p, "Affichage");
        ui.add_space(4.0);
        let before = (self.s.on_disk, self.s.show_files);
        option_row(ui, p, &mut self.s.on_disk, "Taille occupée sur le disque", Some("clusters ; fichiers compressés, creux ou disponibles en ligne seulement"));
        ui.add_space(4.0);
        option_row(ui, p, &mut self.s.show_files, "Fichiers dans l'arborescence", None);
        if before.0 != self.s.on_disk {
            self.rows_dirty = true;
            self.data_rev += 1;
            self.refresh_summary();
        }
        if before.1 != self.s.show_files {
            self.rows_dirty = true;
        }
        ui.add_space(10.0);

        // --- Réglage avancé ---
        let y = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), y, stroke(1.0, p.border));
        ui.add_space(12.0);
        ui.add_enabled_ui(!running, |ui| {
            let mut th = self.s.threads as f64;
            slider_block(ui, p, "Threads d'analyse", "", &mut th, 1.0..=32.0, 1.0);
            self.s.threads = th.round() as usize;
        });
    }

    fn sidebar_foot(&mut self, ui: &mut Ui, p: &'static Palette) {
        let running = self.running();
        let can = self.target_status().is_ok();
        let width = ui.available_width();
        let button = if running {
            Button::new(Kind::Danger, if self.stopping { "Arrêt en cours…" } else { "Arrêter l'analyse" }).icon(&icons::STOP).kbd("Échap").enabled(!self.stopping)
        } else {
            Button::new(Kind::Primary, "Lancer l'analyse").icon(&icons::PLAY).kbd("F5").enabled(can)
        };
        let resp = button.size(width, 40.0).font(semibold(14.0)).show(ui, p);
        let resp = if !running && !can { resp.on_hover_text("Indiquez un dossier existant pour lancer l'analyse.") } else { resp };
        if resp.clicked() {
            if running {
                self.stop_scan();
            } else {
                self.want_start = true;
            }
        }
    }

    // -- Barre d'outils : actions qui modifient l'état ------------------------

    pub fn toolbar_actions(&mut self, export: bool, pdf: bool, clear: bool) {
        if export {
            self.export_csv();
        }
        if pdf {
            self.export_pdf();
        }
        if clear {
            self.clear_results();
        }
    }

    // -- Rapport PDF ----------------------------------------------------------

    /// Écrit le rapport PDF de l'analyse affichée.
    fn write_report(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        if self.summary_dirty {
            self.refresh_summary();
        }
        let bytes = crate::report::render(&self.report_data(), now_unix());
        std::fs::write(path, bytes)
    }

    /// Demande où enregistrer le rapport, l'écrit puis l'ouvre dans le lecteur PDF.
    fn export_pdf(&mut self) {
        if !self.has_data || self.running() {
            return;
        }
        // « C » pour un lecteur, le nom du dossier sinon.
        let root = self.tree.root_path.trim_end_matches('\\');
        let base: String = root.rsplit('\\').next().unwrap_or(root).chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
        let name = format!("diskscan_rapport_{}_{}.pdf", base.trim_matches('_'), chrono::Local::now().format("%Y-%m-%d_%H%M"));
        let Some(path) = rfd::FileDialog::new().set_title("Enregistrer le rapport PDF").add_filter("PDF", &["pdf"]).set_file_name(name).save_file() else {
            return;
        };
        self.message = Some(match self.write_report(&path) {
            Ok(()) => match win::open(&path.to_string_lossy()) {
                Ok(()) => (MsgKind::Ok, format!("Rapport PDF enregistré et ouvert : {}", path.display())),
                Err(e) => (MsgKind::Warn, format!("Rapport PDF enregistré : {} ({e}).", path.display())),
            },
            Err(e) => (MsgKind::Err, format!("Rapport PDF non enregistré : {e}")),
        });
    }
}

impl eframe::App for DiskScanApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_drives();
        let more = self.poll_scan();
        self.handle_keys(ctx);
        // Rapports demandés par le script de test (débogage uniquement).
        #[cfg(debug_assertions)]
        {
            let requests = self.script.as_mut().map(|s| std::mem::take(&mut s.pdf)).unwrap_or_default();
            for path in requests {
                if let Err(e) = self.write_report(std::path::Path::new(&path)) {
                    self.message = Some((MsgKind::Err, format!("Rapport non écrit : {e}")));
                }
            }
        }
        if std::mem::take(&mut self.want_start) {
            self.start_scan(ctx);
        }
        if more {
            ctx.request_repaint();
        } else if self.running() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.track_window(ctx);
        // Pendant l'analyse, les vues ne sont recalculées que quelques fois par seconde.
        if self.rows_dirty && (!self.running() || self.rows_at.elapsed() > Duration::from_millis(250)) {
            self.rebuild_rows();
            self.data_rev += 1;
        }
        if self.summary_dirty && self.s.tab != Tab::Tree && self.s.tab != Tab::Changes && self.summary_at.elapsed() > Duration::from_millis(1000) {
            self.refresh_summary();
        }

        let p = Palette::get(self.s.dark_mode);
        let mut acts = Vec::new();
        self.menu_open.set(false);
        self.status_bar(ctx, p);
        self.sidebar(ctx, p);
        title_bar(ctx, p);
        egui::CentralPanel::default()
            .frame(Frame::none().fill(p.bg).inner_margin(Margin { left: 16.0, right: 16.0, top: 4.0, bottom: 16.0 }))
            .show(ctx, |ui| self.main_area(ui, p, &mut acts));
        self.confirm_dialog(ctx, p, &mut acts);
        resize_handles(ctx);
        #[cfg(debug_assertions)]
        if self.script.as_ref().is_some_and(|s| s.pending()) {
            crate::demo::keep_alive(ctx);
        }

        self.scroll_to = None;
        for a in acts {
            self.apply_act(ctx, a);
        }
        // Menu contextuel refermé : la ligne visée devient la sélection.
        if self.menu_target.is_some() && !self.menu_open.get() {
            self.selected = self.menu_target.take();
            ctx.request_repaint();
        }
        if std::mem::take(&mut self.toggle_theme) {
            self.s.dark_mode = !self.s.dark_mode;
            theme::apply(ctx, self.s.dark_mode);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.s);
    }

    #[cfg(debug_assertions)]
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(script) = &mut self.script {
            script.inject(ctx, raw_input);
            if script.pending() {
                crate::demo::keep_alive(ctx);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fenêtre sans barre de titre : déplacement, boutons, redimensionnement
// ---------------------------------------------------------------------------

/// Remet la fenêtre comme à la dernière fermeture : taille, position (si l'écran est
/// toujours branché), puis agrandissement.
fn restore_window(ctx: &egui::Context, s: &Settings) {
    use egui::ViewportCommand as Cmd;
    if let Some([w, h]) = s.win_size {
        ctx.send_viewport_cmd(Cmd::InnerSize(vec2(w.max(960.0), h.max(560.0))));
    }
    if let Some([x, y]) = s.win_pos {
        let ppp = ctx.native_pixels_per_point().unwrap_or(1.0);
        if win::point_on_screen(((x + 120.0) * ppp) as i32, ((y + 16.0) * ppp) as i32) {
            ctx.send_viewport_cmd(Cmd::OuterPosition(pos2(x, y)));
        }
    }
    if s.maximized {
        ctx.send_viewport_cmd(Cmd::Maximized(true));
    }
}

const TITLE_H: f32 = 32.0;
const CAPTION_W: f32 = 46.0;

fn is_maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// Zone qui déplace la fenêtre (glisser) ou l'agrandit (double-clic).
/// À appeler avant les widgets qu'elle contient, pour qu'ils restent cliquables.
fn window_drag(ui: &Ui, rect: Rect, id: &str) {
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    if resp.double_clicked() {
        let maximized = is_maximized(ui.ctx());
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if resp.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Caption {
    Minimize,
    Maximize,
    Close,
}

/// Bandeau du haut, de la couleur du fond : uniquement réduire / agrandir / fermer.
fn title_bar(ctx: &egui::Context, p: &'static Palette) {
    egui::TopBottomPanel::top("titlebar")
        .exact_height(TITLE_H)
        .show_separator_line(false)
        .frame(Frame::none().fill(p.bg))
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            window_drag(ui, rect, "drag_title");
            let maximized = is_maximized(ctx);
            let mut x = rect.right();
            for kind in [Caption::Close, Caption::Maximize, Caption::Minimize] {
                x -= CAPTION_W;
                let r = Rect::from_min_size(pos2(x, rect.top()), vec2(CAPTION_W, TITLE_H));
                if caption_button(ui, p, r, kind, maximized).clicked() {
                    ctx.send_viewport_cmd(match kind {
                        Caption::Minimize => egui::ViewportCommand::Minimized(true),
                        Caption::Maximize => egui::ViewportCommand::Maximized(!maximized),
                        Caption::Close => egui::ViewportCommand::Close,
                    });
                }
            }
        });
}

fn caption_button(ui: &Ui, p: &Palette, rect: Rect, kind: Caption, maximized: bool) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with(("caption", kind as u8)), Sense::click());
    let down = resp.is_pointer_button_down_on();
    let hot = resp.hovered() || down;
    let (bg, fg) = match (kind, hot) {
        (_, false) => (p.bg, p.text2),
        (Caption::Close, true) => {
            let red = egui::Color32::from_rgb(0xC4, 0x2B, 0x1C);
            (if down { w::lerp_color(red, p.bg, 0.25) } else { red }, egui::Color32::WHITE)
        }
        (_, true) => (w::lerp_color(p.bg, p.text, if down { 0.14 } else { 0.08 }), p.text),
    };
    let painter = ui.painter();
    if hot {
        painter.rect_filled(rect, 0.0, bg);
    }
    let c = pos2(rect.center().x.round(), rect.center().y.round());
    let line = stroke(1.0, fg);
    match kind {
        Caption::Minimize => {
            painter.hline(c.x - 5.0..=c.x + 5.0, c.y + 0.5, line);
        }
        Caption::Maximize if maximized => {
            let back = Rect::from_min_size(pos2(c.x - 2.5, c.y - 5.5), Vec2::splat(8.0));
            let front = Rect::from_min_size(pos2(c.x - 5.5, c.y - 2.5), Vec2::splat(8.0));
            painter.rect_stroke(back, 1.5, line);
            painter.rect(front, 1.5, bg, line);
        }
        Caption::Maximize => {
            painter.rect_stroke(Rect::from_center_size(pos2(c.x + 0.5, c.y + 0.5), Vec2::splat(10.0)), 1.5, line);
        }
        Caption::Close => {
            let line = stroke(1.1, fg);
            painter.line_segment([pos2(c.x - 4.5, c.y - 4.5), pos2(c.x + 5.5, c.y + 5.5)], line);
            painter.line_segment([pos2(c.x + 5.5, c.y - 4.5), pos2(c.x - 4.5, c.y + 5.5)], line);
        }
    }
    resp.on_hover_text(match kind {
        Caption::Minimize => "Réduire",
        Caption::Maximize if maximized => "Restaurer",
        Caption::Maximize => "Agrandir",
        Caption::Close => "Fermer",
    })
}

/// Bords et coins de la fenêtre : redimensionnement (la bordure Windows a disparu avec la barre).
fn resize_handles(ctx: &egui::Context) {
    use egui::{CursorIcon as C, ResizeDirection as D};
    if is_maximized(ctx) {
        return;
    }
    const E: f32 = 4.0;
    const K: f32 = 8.0;
    let s = ctx.screen_rect();
    let (l, r, t, b) = (s.left(), s.right(), s.top(), s.bottom());
    let zones = [
        (Rect::from_min_max(pos2(l + K, t), pos2(r - K, t + E)), D::North, C::ResizeNorth),
        (Rect::from_min_max(pos2(l + K, b - E), pos2(r - K, b)), D::South, C::ResizeSouth),
        (Rect::from_min_max(pos2(l, t + K), pos2(l + E, b - K)), D::West, C::ResizeWest),
        (Rect::from_min_max(pos2(r - E, t + K), pos2(r, b - K)), D::East, C::ResizeEast),
        (Rect::from_min_max(pos2(l, t), pos2(l + K, t + K)), D::NorthWest, C::ResizeNorthWest),
        (Rect::from_min_max(pos2(r - K, t), pos2(r, t + K)), D::NorthEast, C::ResizeNorthEast),
        (Rect::from_min_max(pos2(l, b - K), pos2(l + K, b)), D::SouthWest, C::ResizeSouthWest),
        (Rect::from_min_max(pos2(r - K, b - K), pos2(r, b)), D::SouthEast, C::ResizeSouthEast),
    ];
    // Appelé en fin d'image : le curseur de redimensionnement l'emporte sur celui des widgets.
    let (pos, pressed, held) = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.primary_pressed(), i.pointer.primary_down()));
    let Some(pos) = pos else { return };
    if let Some((_, dir, cursor)) = zones.into_iter().find(|z| z.0.contains(pos)) {
        if pressed {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
        if pressed || !held {
            ctx.set_cursor_icon(cursor);
        }
    }
}

// ---------------------------------------------------------------------------
// Éléments du panneau de paramètres
// ---------------------------------------------------------------------------

fn hint_line(ui: &mut Ui, icon: Option<(&Icon, egui::Color32)>, text: &str, color: egui::Color32) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some((ic, c)) = icon {
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 16.0), Sense::hover());
            icons::paint(ui, ic, w::icon_rect(r.left(), r.center().y, 14.0), c);
        }
        ui.add(Label::new(RichText::new(text).font(regular(12.0)).color(color)).wrap());
    });
}

fn lock_banner(ui: &mut Ui, p: &Palette) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
    let painter = ui.painter();
    painter.rect(rect.shrink(0.5), 6.0, p.surface2, stroke(1.0, p.border));
    icons::paint(ui, &icons::LOCK, w::icon_rect(rect.left() + 10.0, rect.center().y, 14.0), p.text2);
    let g = w::galley(ui, "Cible verrouillée pendant l'analyse", regular(12.0), p.text2);
    w::text_at(painter, g, rect.left() + 32.0, rect.center().y);
}

/// État d'un lecteur dans la liste.
struct DriveState {
    /// Cible actuelle.
    selected: bool,
    /// Son analyse est affichée.
    shown: bool,
    /// Date de sa dernière analyse enregistrée.
    when: Option<i64>,
}

/// Carte d'un lecteur : nom, espace libre, barre d'occupation, taille, état de l'analyse.
/// Un lecteur déjà analysé porte une bande verte à gauche et la date de son analyse.
fn drive_card(ui: &mut Ui, p: &Palette, d: &Drive, st: DriveState) -> egui::Response {
    let selected = st.selected;
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 56.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let enabled = ui.is_enabled();
    let hot = enabled && (resp.hovered() || resp.is_pointer_button_down_on());
    let (fill, line) = match (selected, hot) {
        (true, _) => (p.accent_soft, p.selected_border),
        (false, true) => (p.hover, p.border_strong),
        (false, false) => (p.surface, p.border),
    };
    let painter = ui.painter();
    painter.rect(rect.shrink(0.5), 6.0, fill, stroke(1.0, line));
    if st.when.is_some() {
        let band = Rect::from_min_max(pos2(rect.left() + 1.0, rect.top() + 1.0), pos2(rect.left() + 4.0, rect.bottom() - 1.0));
        painter.rect_filled(band, egui::Rounding { nw: 5.0, sw: 5.0, ne: 0.0, se: 0.0 }, p.ok);
    }
    let (left, right) = (rect.left() + 12.0, rect.right() - 12.0);
    let frac = if d.total == 0 { 0.0 } else { (d.used() as f64 / d.total as f64) as f32 };
    let cy = rect.top() + 16.0;
    icons::paint(ui, &icons::HARD_DRIVE, w::icon_rect(left, cy, 15.0), if selected { p.accent_text } else { p.text2 });
    let fg = w::galley(ui, format!("{} libres", fmt::size(d.free)), regular(11.5), p.text2);
    let fw = fg.size().x;
    w::text_at(painter, fg, right - fw, cy);
    let tg = w::galley_trunc(ui, &d.title(), semibold(13.0), p.text, right - fw - 8.0 - (left + 23.0));
    w::text_at(painter, tg, left + 23.0, cy);
    // Barre d'occupation : bleue, orange au-delà de 85 %, rouge au-delà de 95 %.
    let track = Rect::from_min_max(pos2(left, rect.top() + 28.0), pos2(right, rect.top() + 33.0));
    let sub = format!("{} · {}", fmt::size(d.total), fmt::percent(frac as f64));
    w::text_at(painter, w::galley(ui, sub, regular(11.0), p.text3), left, rect.top() + 44.0);
    // État de l'analyse, à droite de la dernière ligne.
    let (state, color) = match st.when {
        Some(when) if st.shown => (format!("affiché · {}", short_date(when)), p.ok),
        Some(when) => (format!("analysé le {}", short_date(when)), p.ok),
        None => ("jamais analysé".to_string(), p.text3),
    };
    let sg = w::galley(ui, state, if st.when.is_some() { semibold(11.0) } else { regular(11.0) }, color);
    let sw = sg.size().x;
    w::text_at(painter, sg, right - sw, rect.top() + 44.0);
    if st.when.is_some() {
        icons::paint(ui, &icons::CHECK_CIRCLE, w::icon_rect(right - sw - 15.0, rect.top() + 44.0, 11.0), p.ok);
    }
    painter.rect_filled(track, 2.5, p.track);
    let color = if frac >= 0.95 {
        p.err_fill
    } else if frac >= 0.85 {
        p.warn
    } else {
        p.accent
    };
    if frac > 0.0 {
        painter.rect_filled(Rect::from_min_size(track.min, vec2((track.width() * frac).max(5.0), 5.0)), 2.5, color);
    }
    if enabled {
        let mut tip = format!("{} — {} occupés ({}), {} libres sur {}", d.root, fmt::size(d.used()), fmt::percent(frac as f64), fmt::size(d.free), fmt::size(d.total));
        match st.when {
            Some(when) if st.shown => tip.push_str(&format!("\nAnalyse du {} affichée. F5 pour l'analyser de nouveau.", fmt::date_time(when))),
            Some(when) => tip.push_str(&format!("\nDernière analyse : {}. Cliquer pour l'afficher.", fmt::date_time(when))),
            None => tip.push_str("\nJamais analysé. Cliquer pour le choisir, puis F5."),
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
    } else {
        resp
    }
}

/// Libellé + valeur modifiable (clic pour saisir, glisser pour ajuster) + curseur.
fn slider_block(ui: &mut Ui, p: &Palette, title: &str, suffix: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>, step: f64) {
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.horizontal(|ui| {
        w::field_label(ui, p, title);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.style_mut().drag_value_text_style = egui::TextStyle::Monospace;
            ui.style_mut().text_styles.insert(egui::TextStyle::Monospace, mono(12.0));
            ui.spacing_mut().button_padding = vec2(5.0, 1.0);
            ui.spacing_mut().interact_size = vec2(34.0, 20.0);
            ui.visuals_mut().override_text_color = Some(p.text);
            let field = egui::DragValue::new(value).range(range.clone()).speed(step).max_decimals(0).suffix(suffix).update_while_editing(false);
            ui.add(field).on_hover_text("Cliquer pour saisir une valeur");
        });
    });
    w::slider(ui, p, value, range, step);
}

fn option_row(ui: &mut Ui, p: &Palette, on: &mut bool, title: &str, hint: Option<&str>) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.vertical(|ui| {
            ui.add_space(1.0);
            w::switch(ui, p, on);
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            let r = ui.add(Label::new(RichText::new(title).font(regular(13.0)).color(p.text)).sense(Sense::click()));
            if r.clicked() {
                *on = !*on;
            }
            if let Some(hint) = hint {
                ui.add(Label::new(RichText::new(hint).font(regular(11.5)).color(p.text3)).wrap());
            }
        });
    });
}

/// « 14/11 15:39 » : date courte d'une analyse (avec l'année si ce n'est pas l'année en cours).
fn short_date(unix: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    match Local.timestamp_opt(unix, 0).single() {
        Some(t) if t.year() == Local::now().year() => t.format("%d/%m %H:%M").to_string(),
        Some(t) => t.format("%d/%m/%Y").to_string(),
        None => "—".into(),
    }
}

/// Icône de la fenêtre.
pub fn app_icon() -> egui::IconData {
    egui::IconData {
        rgba: crate::icon::logo_rgba(64),
        width: 64,
        height: 64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cible() {
        assert!(normalize_target("").is_err());
        assert!(normalize_target("   ").is_err());
        assert!(normalize_target(r"C:\ce-dossier-n-existe-pas-diskscan").is_err());
        assert_eq!(normalize_target("c:").unwrap(), r"C:\");
        assert_eq!(normalize_target(r"c:\").unwrap(), r"C:\");
        assert_eq!(normalize_target("\"C:\\Windows\\\"").unwrap(), r"C:\Windows");
        assert_eq!(normalize_target("C:/Windows/").unwrap(), r"C:\Windows");
        // Un fichier n'est pas une cible.
        assert!(normalize_target(r"C:\Windows\explorer.exe").is_err());
    }
}
