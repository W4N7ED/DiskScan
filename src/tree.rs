//! Arborescence en mémoire : nœuds, agrégation des tailles, tri, vues dérivées
//! (plus gros fichiers, extensions, ancienneté) et comparaison de deux analyses.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};

pub type NodeId = u32;
/// « Pas de nœud » (fin de liste, pas de parent, dossier non parcouru).
pub const NONE: NodeId = u32::MAX;

/// Indicateurs d'un nœud.
pub mod flag {
    /// Dossier (sinon : fichier).
    pub const DIR: u8 = 1;
    /// Lien symbolique ou jonction : listé, jamais suivi.
    pub const LINK: u8 = 2;
    /// Dossier illisible (accès refusé).
    pub const DENIED: u8 = 4;
    /// Lien physique vers un fichier déjà compté ailleurs : sa taille ne s'ajoute pas.
    pub const DUP: u8 = 8;
    /// Dossier pas encore parcouru (analyse en cours ou interrompue).
    pub const PENDING: u8 = 16;
    /// Fichiers d'un dossier regroupés en une ligne (analyse rechargée depuis le disque).
    pub const GROUP: u8 = 32;
}

/// Une entrée lue dans un dossier par le moteur d'analyse.
pub struct Entry {
    pub name: Box<str>,
    pub size: u64,
    pub alloc: u64,
    pub mtime: i64,
    pub flags: u8,
    /// Numéro attribué par le moteur aux sous-dossiers qu'il va parcourir (`NONE` sinon).
    pub id: u32,
}

/// Contenu d'un dossier, envoyé en un seul lot par le moteur.
pub struct DirBatch {
    pub id: u32,
    pub denied: bool,
    pub entries: Vec<Entry>,
}

pub struct Node {
    pub name: Box<str>,
    pub parent: NodeId,
    pub first_child: NodeId,
    pub next: NodeId,
    /// Taille réelle. Dossier : somme de tout ce qu'il contient.
    pub size: u64,
    /// Taille occupée sur le disque.
    pub alloc: u64,
    /// Dernière modification (secondes Unix). Dossier : la plus récente de son contenu.
    pub mtime: i64,
    /// Nombre de fichiers contenus (1 pour un fichier).
    pub files: u32,
    /// Nombre de sous-dossiers contenus, à tous les niveaux.
    pub dirs: u32,
    /// Nombre de dossiers illisibles (lui-même compris).
    pub denied: u32,
    pub flags: u8,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.flags & flag::DIR != 0
    }
    pub fn has(&self, f: u8) -> bool {
        self.flags & f != 0
    }
    /// Taille comptée dans les totaux : réelle ou sur le disque, nulle pour un lien physique déjà compté.
    pub fn weight(&self, on_disk: bool) -> u64 {
        if self.has(flag::DUP) {
            0
        } else if on_disk {
            self.alloc
        } else {
            self.size
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Size,
    Files,
    Dirs,
    Modified,
}

/// Écart d'un dossier par rapport à l'analyse précédente.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Delta {
    pub size: i64,
    pub alloc: i64,
    /// Dossier absent de l'analyse précédente.
    pub new: bool,
}

impl Delta {
    pub fn get(&self, on_disk: bool) -> i64 {
        if on_disk {
            self.alloc
        } else {
            self.size
        }
    }
}

pub struct TopFile {
    pub path: String,
    pub size: u64,
    pub alloc: u64,
    pub mtime: i64,
    /// Nœud correspondant (`NONE` pour une analyse rechargée, qui ne garde pas les fichiers).
    pub node: NodeId,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExtStat {
    pub ext: String,
    pub files: u64,
    pub size: u64,
    pub alloc: u64,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct AgeStat {
    pub files: u64,
    pub size: u64,
    pub alloc: u64,
}

pub const AGE_LABELS: [&str; 6] = ["Moins d'un mois", "1 à 6 mois", "6 à 12 mois", "1 à 2 ans", "2 à 5 ans", "Plus de 5 ans"];
/// Limite haute de chaque tranche d'ancienneté, en jours.
const AGE_DAYS: [i64; 5] = [30, 182, 365, 730, 1825];
pub const TOP_FILES: usize = 100;
pub const NO_EXT: &str = "(sans extension)";

/// Vues complémentaires calculées en un seul parcours.
#[derive(Default)]
pub struct Summary {
    pub top: Vec<TopFile>,
    pub exts: Vec<ExtStat>,
    pub ages: [AgeStat; 6],
}

pub struct Tree {
    pub nodes: Vec<Node>,
    /// Chemin complet de la racine (« C:\ », « C:\Users »).
    pub root_path: String,
}

/// Extension en minuscules, point compris (« .txt »), ou `NO_EXT`.
pub fn extension(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i > 1 && name.len() - i <= 16 && !name[i..].contains(' ') => name[i..].to_lowercase(),
        _ => NO_EXT.to_string(),
    }
}

/// Comparaison sans tenir compte de la casse, sans allocation.
pub fn cmp_names(a: &str, b: &str) -> Ordering {
    let mut x = a.chars().flat_map(char::to_lowercase);
    let mut y = b.chars().flat_map(char::to_lowercase);
    loop {
        match (x.next(), y.next()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c != d => return c.cmp(&d),
            _ => {}
        }
    }
}

/// Chemin sans barre oblique finale (« C:\ » → « C: »), pour comparer des racines.
fn trim_root(path: &str) -> &str {
    path.trim_end_matches('\\')
}

impl Tree {
    pub fn new(root_path: &str) -> Self {
        let root = Node {
            name: root_path.into(),
            parent: NONE,
            first_child: NONE,
            next: NONE,
            size: 0,
            alloc: 0,
            mtime: 0,
            files: 0,
            dirs: 0,
            denied: 0,
            flags: flag::DIR | flag::PENDING,
        };
        Self { nodes: vec![root], root_path: root_path.to_string() }
    }

    pub const ROOT: NodeId = 0;

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let mut cur = self.nodes[id as usize].first_child;
        std::iter::from_fn(move || {
            if cur == NONE {
                return None;
            }
            let id = cur;
            cur = self.nodes[id as usize].next;
            Some(id)
        })
    }

    pub fn has_children(&self, id: NodeId) -> bool {
        self.nodes[id as usize].first_child != NONE
    }

    /// Chemin complet d'un nœud.
    pub fn path(&self, id: NodeId) -> String {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != NONE && cur != Self::ROOT {
            let n = &self.nodes[cur as usize];
            if !n.has(flag::GROUP) {
                parts.push(&*n.name);
            }
            cur = n.parent;
        }
        let mut out = self.root_path.clone();
        for p in parts.iter().rev() {
            if !out.ends_with('\\') {
                out.push('\\');
            }
            out.push_str(p);
        }
        out
    }

    /// `id` est-il `ancestor` ou l'un de ses descendants ?
    pub fn is_under(&self, mut id: NodeId, ancestor: NodeId) -> bool {
        while id != NONE {
            if id == ancestor {
                return true;
            }
            id = self.nodes[id as usize].parent;
        }
        false
    }

    /// Ajoute un nœud en tête de la liste des enfants de `parent`, sans toucher aux totaux.
    fn push(&mut self, parent: NodeId, mut node: Node) -> NodeId {
        let id = self.nodes.len() as NodeId;
        node.parent = parent;
        node.next = self.nodes[parent as usize].first_child;
        self.nodes[parent as usize].first_child = id;
        self.nodes.push(node);
        id
    }

    /// Reporte un écart sur `from` et tous ses ancêtres.
    fn bubble(&mut self, from: NodeId, size: i64, alloc: i64, files: i64, dirs: i64, denied: i64, mtime: i64) {
        let mut cur = from;
        while cur != NONE {
            let n = &mut self.nodes[cur as usize];
            n.size = n.size.saturating_add_signed(size);
            n.alloc = n.alloc.saturating_add_signed(alloc);
            n.files = n.files.saturating_add_signed(files as i32);
            n.dirs = n.dirs.saturating_add_signed(dirs as i32);
            n.denied = n.denied.saturating_add_signed(denied as i32);
            n.mtime = n.mtime.max(mtime);
            cur = n.parent;
        }
    }

    /// Intègre le contenu d'un dossier et met à jour les totaux jusqu'à la racine.
    /// `map` associe les numéros de dossiers du moteur aux nœuds de l'arbre.
    pub fn apply_batch(&mut self, map: &mut Vec<NodeId>, batch: DirBatch) {
        let Some(&dir) = map.get(batch.id as usize) else { return };
        if dir == NONE {
            return;
        }
        self.nodes[dir as usize].flags &= !flag::PENDING;
        if batch.denied {
            self.nodes[dir as usize].flags |= flag::DENIED;
            self.bubble(dir, 0, 0, 0, 0, 1, 0);
        }
        let (mut size, mut alloc, mut files, mut dirs, mut mtime) = (0i64, 0i64, 0i64, 0i64, 0i64);
        for e in batch.entries {
            let is_dir = e.flags & flag::DIR != 0;
            let counted = e.flags & flag::DUP == 0;
            let mut flags = e.flags;
            if is_dir {
                dirs += 1;
                if e.id != NONE {
                    flags |= flag::PENDING;
                }
            } else {
                files += 1;
                if counted {
                    size += e.size as i64;
                    alloc += e.alloc as i64;
                }
                mtime = mtime.max(e.mtime);
            }
            let node = Node {
                name: e.name,
                parent: dir,
                first_child: NONE,
                next: NONE,
                size: if is_dir { 0 } else { e.size },
                alloc: if is_dir { 0 } else { e.alloc },
                mtime: e.mtime,
                files: u32::from(!is_dir),
                dirs: 0,
                denied: 0,
                flags,
            };
            let id = self.push(dir, node);
            if e.id != NONE {
                let k = e.id as usize;
                if map.len() <= k {
                    map.resize(k + 1, NONE);
                }
                map[k] = id;
            }
        }
        self.bubble(dir, size, alloc, files, dirs, 0, mtime);
    }

    /// Retire un fichier ou un dossier (envoyé à la corbeille) et corrige les totaux.
    pub fn remove(&mut self, id: NodeId) {
        if id == Self::ROOT {
            return;
        }
        let (parent, next) = (self.nodes[id as usize].parent, self.nodes[id as usize].next);
        // Décroche le nœud de la liste de son parent.
        if self.nodes[parent as usize].first_child == id {
            self.nodes[parent as usize].first_child = next;
        } else {
            let mut cur = self.nodes[parent as usize].first_child;
            while cur != NONE {
                if self.nodes[cur as usize].next == id {
                    self.nodes[cur as usize].next = next;
                    break;
                }
                cur = self.nodes[cur as usize].next;
            }
        }
        let n = &self.nodes[id as usize];
        let (size, alloc) = if n.is_dir() { (n.size, n.alloc) } else { (n.weight(false), n.weight(true)) };
        let (files, dirs, denied) = (n.files as i64, n.dirs as i64 + i64::from(n.is_dir()), n.denied as i64);
        self.nodes[id as usize].parent = NONE;
        self.bubble(parent, -(size as i64), -(alloc as i64), -files, -dirs, -denied, 0);
    }

    /// Vide un dossier avant de le rescanner : son contenu est retiré des totaux.
    pub fn reset_dir(&mut self, id: NodeId) {
        let n = &mut self.nodes[id as usize];
        let (size, alloc, files, dirs, denied) = (n.size as i64, n.alloc as i64, n.files as i64, n.dirs as i64, n.denied as i64);
        n.first_child = NONE;
        n.flags = flag::DIR | flag::PENDING;
        n.mtime = 0;
        self.bubble(id, -size, -alloc, -files, -dirs, -denied, 0);
    }

    /// Ordre d'affichage de deux nœuds selon la colonne triée (les égalités se départagent par le nom).
    pub fn compare(&self, a: NodeId, b: NodeId, key: SortKey, on_disk: bool) -> Ordering {
        let (x, y) = (self.node(a), self.node(b));
        let ord = match key {
            SortKey::Name => Ordering::Equal,
            SortKey::Size => x.weight(on_disk).cmp(&y.weight(on_disk)),
            SortKey::Files => x.files.cmp(&y.files),
            SortKey::Dirs => x.dirs.cmp(&y.dirs),
            SortKey::Modified => x.mtime.cmp(&y.mtime),
        };
        ord.then_with(|| cmp_names(&x.name, &y.name))
    }

    fn value_eq(&self, a: NodeId, b: NodeId, key: SortKey, on_disk: bool) -> bool {
        let (x, y) = (self.node(a), self.node(b));
        match key {
            SortKey::Name => true,
            SortKey::Size => x.weight(on_disk) == y.weight(on_disk),
            SortKey::Files => x.files == y.files,
            SortKey::Dirs => x.dirs == y.dirs,
            SortKey::Modified => x.mtime == y.mtime,
        }
    }

    /// Enfants d'un dossier, triés. `asc == false` : du plus grand au plus petit.
    pub fn sorted_children(&self, id: NodeId, key: SortKey, asc: bool, on_disk: bool, with_files: bool) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self.children(id).filter(|&c| with_files || self.node(c).is_dir() || self.node(c).has(flag::GROUP)).collect();
        ids.sort_unstable_by(|&a, &b| {
            if asc {
                self.compare(a, b, key, on_disk)
            } else if key == SortKey::Name {
                self.compare(b, a, key, on_disk)
            } else {
                // Valeur décroissante, mais nom croissant entre ex æquo.
                if self.value_eq(a, b, key, on_disk) {
                    cmp_names(&self.node(a).name, &self.node(b).name)
                } else {
                    self.compare(b, a, key, on_disk)
                }
            }
        });
        ids
    }

    /// Plus gros fichiers, répartition par extension et par ancienneté, en un parcours.
    pub fn summarize(&self, now: i64, on_disk: bool) -> Summary {
        let mut heap: BinaryHeap<Reverse<(u64, NodeId)>> = BinaryHeap::with_capacity(TOP_FILES + 1);
        let mut exts: HashMap<String, ExtStat> = HashMap::new();
        let mut ages = [AgeStat::default(); 6];
        let mut stack = vec![Self::ROOT];
        while let Some(id) = stack.pop() {
            for c in self.children(id) {
                let n = self.node(c);
                if n.is_dir() {
                    stack.push(c);
                    continue;
                }
                if n.has(flag::GROUP) {
                    continue;
                }
                let (size, alloc) = (n.weight(false), n.weight(true));
                let w = if on_disk { alloc } else { size };
                if heap.len() < TOP_FILES {
                    heap.push(Reverse((w, c)));
                } else if heap.peek().is_some_and(|m| m.0 .0 < w) {
                    heap.pop();
                    heap.push(Reverse((w, c)));
                }
                let ext = extension(&n.name);
                let e = exts.entry(ext).or_insert_with_key(|k| ExtStat { ext: k.clone(), files: 0, size: 0, alloc: 0 });
                e.files += 1;
                e.size += size;
                e.alloc += alloc;
                let days = (now - n.mtime).max(0) / 86_400;
                let a = &mut ages[AGE_DAYS.iter().position(|&d| days < d).unwrap_or(5)];
                a.files += 1;
                a.size += size;
                a.alloc += alloc;
            }
        }
        let mut top: Vec<(u64, NodeId)> = heap.into_iter().map(|r| r.0).collect();
        top.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let top = top
            .into_iter()
            .map(|(_, id)| {
                let n = self.node(id);
                TopFile { path: self.path(id), size: n.size, alloc: n.alloc, mtime: n.mtime, node: id }
            })
            .collect();
        let mut exts: Vec<ExtStat> = exts.into_values().collect();
        exts.sort_unstable_by(|a, b| {
            let (x, y) = if on_disk { (a.alloc, b.alloc) } else { (a.size, b.size) };
            y.cmp(&x).then_with(|| a.ext.cmp(&b.ext))
        });
        Summary { top, exts, ages }
    }

    /// Ajoute à chaque dossier une ligne « fichiers » regroupant ce que ses sous-dossiers n'expliquent pas.
    /// Sert aux arbres qui ne gardent que les dossiers (analyse rechargée, référence de comparaison).
    pub fn add_groups(&mut self) {
        for id in 0..self.nodes.len() as NodeId {
            let n = self.node(id);
            if !n.is_dir() {
                continue;
            }
            let (mut size, mut alloc, mut files) = (n.size, n.alloc, n.files);
            let mtime = n.mtime;
            for c in self.children(id) {
                let k = self.node(c);
                size = size.saturating_sub(k.size);
                alloc = alloc.saturating_sub(k.alloc);
                files = files.saturating_sub(k.files);
            }
            if files > 0 {
                let node = Node {
                    name: "".into(),
                    parent: id,
                    first_child: NONE,
                    next: NONE,
                    size,
                    alloc,
                    mtime,
                    files,
                    dirs: 0,
                    denied: 0,
                    flags: flag::GROUP,
                };
                self.push(id, node);
            }
        }
    }

    /// Ajoute un dossier déjà totalisé (rechargement depuis le disque). Ne touche pas aux ancêtres.
    #[allow(clippy::too_many_arguments)]
    pub fn add_dir_raw(&mut self, parent: NodeId, name: &str, size: u64, alloc: u64, files: u32, dirs: u32, denied: u32, mtime: i64, flags: u8) -> NodeId {
        let node = Node { name: name.into(), parent, first_child: NONE, next: NONE, size, alloc, mtime, files, dirs, denied, flags: flags | flag::DIR };
        self.push(parent, node)
    }

    /// Copie ne gardant que les dossiers et leurs totaux (les fichiers deviennent des lignes groupées).
    pub fn dirs_only(&self) -> Tree {
        let mut out = Tree::new(&self.root_path);
        let r = self.node(Self::ROOT);
        let o = &mut out.nodes[0];
        (o.size, o.alloc, o.files, o.dirs, o.denied, o.mtime, o.flags) = (r.size, r.alloc, r.files, r.dirs, r.denied, r.mtime, r.flags);
        let mut stack = vec![(Self::ROOT, Tree::ROOT)];
        while let Some((src, dst)) = stack.pop() {
            for c in self.children(src) {
                let n = self.node(c);
                if n.is_dir() {
                    let id = out.add_dir_raw(dst, &n.name, n.size, n.alloc, n.files, n.dirs, n.denied, n.mtime, n.flags);
                    stack.push((c, id));
                }
            }
        }
        out.add_groups();
        out
    }

    /// Nœud d'un chemin situé sous la racine (sans tenir compte de la casse).
    pub fn locate(&self, path: &str) -> Option<NodeId> {
        let root = trim_root(&self.root_path).to_lowercase();
        let path = trim_root(path).to_lowercase();
        if path == root {
            return Some(Self::ROOT);
        }
        let rest = path.strip_prefix(&root)?.strip_prefix('\\')?;
        let mut cur = Self::ROOT;
        for part in rest.split('\\') {
            cur = self.children(cur).find(|&c| {
                let n = self.node(c);
                n.is_dir() && n.name.to_lowercase() == part
            })?;
        }
        Some(cur)
    }
}

/// Compare deux analyses : écart de taille de chaque dossier de `new` présent dans `old`,
/// et dossiers nouveaux. Les deux racines peuvent différer si l'une contient l'autre.
pub fn compare(old: &Tree, new: &Tree) -> HashMap<NodeId, Delta> {
    let mut out = HashMap::new();
    let start = if let Some(o) = old.locate(&new.root_path) {
        (o, Tree::ROOT)
    } else if let Some(n) = new.locate(&old.root_path) {
        (Tree::ROOT, n)
    } else {
        return out;
    };
    let mut stack = vec![start];
    while let Some((o, n)) = stack.pop() {
        let (a, b) = (old.node(o), new.node(n));
        // Un dossier resté illisible ou non parcouru d'un côté ne se compare pas.
        if a.has(flag::PENDING) || b.has(flag::PENDING) {
            continue;
        }
        let d = Delta { size: b.size as i64 - a.size as i64, alloc: b.alloc as i64 - a.alloc as i64, new: false };
        if d.size != 0 || d.alloc != 0 {
            out.insert(n, d);
        }
        let olds: HashMap<String, NodeId> = old.children(o).filter(|&c| old.node(c).is_dir()).map(|c| (old.node(c).name.to_lowercase(), c)).collect();
        for c in new.children(n) {
            let k = new.node(c);
            if !k.is_dir() || k.has(flag::LINK) {
                continue;
            }
            match olds.get(&k.name.to_lowercase()) {
                Some(&oc) => stack.push((oc, c)),
                None => {
                    out.insert(c, Delta { size: k.size as i64, alloc: k.alloc as i64, new: true });
                }
            }
        }
    }
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn file(name: &str, size: u64, alloc: u64, mtime: i64) -> Entry {
        Entry { name: name.into(), size, alloc, mtime, flags: 0, id: NONE }
    }
    pub fn dir(name: &str, id: u32) -> Entry {
        Entry { name: name.into(), size: 0, alloc: 0, mtime: 0, flags: flag::DIR, id }
    }

    /// C:\data ─ a.txt (100) · b.bin (5000) · sub ─ c.txt (30) · lien (dup 5000) · deep ─ d.log (7)
    ///                                        └ interdit (illisible) · jonction (lien, non suivie)
    pub fn sample() -> (Tree, Vec<NodeId>) {
        let mut t = Tree::new(r"C:\data");
        let mut map = vec![Tree::ROOT];
        let mut jonction = dir("jonction", NONE);
        jonction.flags |= flag::LINK;
        t.apply_batch(&mut map, DirBatch { id: 0, denied: false, entries: vec![file("a.txt", 100, 4096, 1000), file("b.bin", 5000, 8192, 3000), dir("sub", 1), dir("interdit", 2), jonction] });
        let mut dup = file("lien", 5000, 8192, 2000);
        dup.flags |= flag::DUP;
        t.apply_batch(&mut map, DirBatch { id: 1, denied: false, entries: vec![file("c.txt", 30, 4096, 9000), dup, dir("deep", 3)] });
        t.apply_batch(&mut map, DirBatch { id: 2, denied: true, entries: vec![] });
        t.apply_batch(&mut map, DirBatch { id: 3, denied: false, entries: vec![file("d.log", 7, 0, 500)] });
        (t, map)
    }

    #[test]
    fn agregation() {
        let (t, map) = sample();
        let r = t.node(Tree::ROOT);
        assert_eq!((r.size, r.alloc), (100 + 5000 + 30 + 7, 4096 + 8192 + 4096));
        // Le lien physique compte comme fichier, pas comme taille.
        assert_eq!((r.files, r.dirs, r.denied), (5, 4, 1));
        assert_eq!(r.mtime, 9000);
        assert!(!r.has(flag::PENDING));
        let sub = t.node(map[1]);
        assert_eq!((sub.size, sub.alloc, sub.files, sub.dirs), (37, 4096, 3, 1));
        let deep = t.node(map[3]);
        assert_eq!((deep.size, deep.files, deep.dirs), (7, 1, 0));
        assert!(t.node(map[2]).has(flag::DENIED));
        assert_eq!(t.path(map[3]), r"C:\data\sub\deep");
        assert_eq!(t.path(Tree::ROOT), r"C:\data");
    }

    #[test]
    fn chemins_a_la_racine_d_un_lecteur() {
        let mut t = Tree::new(r"C:\");
        let mut map = vec![Tree::ROOT];
        t.apply_batch(&mut map, DirBatch { id: 0, denied: false, entries: vec![dir("Users", 1)] });
        assert_eq!(t.path(map[1]), r"C:\Users");
        assert_eq!(t.locate(r"c:\users"), Some(map[1]));
        assert_eq!(t.locate(r"C:"), Some(Tree::ROOT));
        assert_eq!(t.locate(r"D:\Users"), None);
        assert_eq!(t.locate(r"C:\Usersx"), None);
    }

    #[test]
    fn dossier_en_attente() {
        let mut t = Tree::new(r"C:\x");
        let mut map = vec![Tree::ROOT];
        t.apply_batch(&mut map, DirBatch { id: 0, denied: false, entries: vec![dir("a", 1)] });
        assert!(t.node(map[1]).has(flag::PENDING));
        // Un lot pour un numéro inconnu est ignoré.
        t.apply_batch(&mut map, DirBatch { id: 42, denied: false, entries: vec![file("x", 1, 1, 1)] });
        assert_eq!(t.node(Tree::ROOT).files, 0);
    }

    #[test]
    fn tri() {
        let (t, _) = sample();
        let names = |ids: Vec<NodeId>| ids.into_iter().map(|i| t.node(i).name.to_string()).collect::<Vec<_>>();
        assert_eq!(names(t.sorted_children(0, SortKey::Size, false, false, true)), ["b.bin", "a.txt", "sub", "interdit", "jonction"]);
        assert_eq!(names(t.sorted_children(0, SortKey::Size, false, true, true)), ["b.bin", "a.txt", "sub", "interdit", "jonction"]);
        assert_eq!(names(t.sorted_children(0, SortKey::Size, true, false, true)), ["interdit", "jonction", "sub", "a.txt", "b.bin"]);
        assert_eq!(names(t.sorted_children(0, SortKey::Name, true, false, true)), ["a.txt", "b.bin", "interdit", "jonction", "sub"]);
        assert_eq!(names(t.sorted_children(0, SortKey::Name, false, false, true)), ["sub", "jonction", "interdit", "b.bin", "a.txt"]);
        assert_eq!(names(t.sorted_children(0, SortKey::Files, false, false, true)), ["sub", "a.txt", "b.bin", "interdit", "jonction"]);
        assert_eq!(names(t.sorted_children(0, SortKey::Modified, false, false, true)), ["sub", "b.bin", "a.txt", "interdit", "jonction"]);
        // Sans les fichiers.
        assert_eq!(names(t.sorted_children(0, SortKey::Size, false, false, false)), ["sub", "interdit", "jonction"]);
        assert_eq!(cmp_names("Abc", "abd"), Ordering::Less);
        assert_eq!(cmp_names("b", "A"), Ordering::Greater);
    }

    #[test]
    fn retrait_et_rescan() {
        // Retirer « sub » enlève son contenu et lui-même des totaux.
        let (mut t, map) = sample();
        t.remove(map[1]);
        let r = t.node(Tree::ROOT);
        assert_eq!((r.size, r.alloc, r.files, r.dirs), (5100, 12288, 2, 2));
        assert_eq!(t.children(0).count(), 4);

        let (mut t, map) = sample();
        t.reset_dir(map[1]);
        let r = t.node(Tree::ROOT);
        assert_eq!((r.size, r.files, r.dirs), (5100, 2, 3));
        assert!(t.node(map[1]).has(flag::PENDING));
        assert!(!t.has_children(map[1]));
        // Nouveau parcours du dossier : les totaux reviennent.
        let mut map2 = vec![map[1]];
        t.apply_batch(&mut map2, DirBatch { id: 0, denied: false, entries: vec![file("c.txt", 30, 4096, 9000)] });
        assert_eq!((t.node(0).size, t.node(0).files), (5130, 3));

        // Retirer un fichier, puis un lien physique déjà compté (taille nulle).
        let (mut t, map) = sample();
        let a = t.children(0).find(|&c| &*t.node(c).name == "a.txt").unwrap();
        t.remove(a);
        assert_eq!((t.node(0).size, t.node(0).files), (5037, 4));
        let dup = t.children(map[1]).find(|&c| &*t.node(c).name == "lien").unwrap();
        t.remove(dup);
        assert_eq!((t.node(0).size, t.node(0).files), (5037, 3));
    }

    #[test]
    fn extensions() {
        assert_eq!(extension("Photo.JPG"), ".jpg");
        assert_eq!(extension("archive.tar.gz"), ".gz");
        assert_eq!(extension("Makefile"), NO_EXT);
        assert_eq!(extension(".gitignore"), NO_EXT);
        assert_eq!(extension("fin."), NO_EXT);
        assert_eq!(extension("v1. final"), NO_EXT);
    }

    #[test]
    fn vues() {
        let (t, _) = sample();
        let now = 9000 + 40 * 86_400;
        let s = t.summarize(now, false);
        let top: Vec<(&str, u64)> = s.top.iter().map(|f| (f.path.as_str(), f.size)).collect();
        assert_eq!(top[0], (r"C:\data\b.bin", 5000));
        assert_eq!(top[1], (r"C:\data\a.txt", 100));
        assert_eq!(top.len(), 5);
        assert_eq!(s.exts[0], ExtStat { ext: ".bin".into(), files: 1, size: 5000, alloc: 8192 });
        assert_eq!(s.exts[1], ExtStat { ext: ".txt".into(), files: 2, size: 130, alloc: 8192 });
        // Le lien physique déjà compté apparaît avec une taille nulle.
        assert!(s.exts.iter().any(|e| e.ext == NO_EXT && e.files == 1 && e.size == 0));
        // Tous les fichiers ont entre 40 jours et 6 mois.
        assert_eq!(s.ages[1], AgeStat { files: 5, size: 5137, alloc: 16384 });
        assert_eq!(s.ages[0], AgeStat::default());
        // Classement par taille sur le disque : a.txt (4096) passe devant d.log (0), ex æquo départagés.
        let s = t.summarize(now, true);
        assert_eq!(s.top[0].path, r"C:\data\b.bin");
        assert_eq!(s.exts[0].alloc, 8192);
    }

    #[test]
    fn dossiers_seuls() {
        let (t, _) = sample();
        let d = t.dirs_only();
        let r = d.node(Tree::ROOT);
        assert_eq!((r.size, r.files, r.dirs, r.denied), (5137, 5, 4, 1));
        // Racine : 3 dossiers + la ligne des 2 fichiers (a.txt, b.bin).
        let group = d.children(0).find(|&c| d.node(c).has(flag::GROUP)).unwrap();
        assert_eq!((d.node(group).files, d.node(group).size), (2, 5100));
        assert_eq!(d.children(0).count(), 4);
        let sub = d.locate(r"C:\data\sub").unwrap();
        let g = d.children(sub).find(|&c| d.node(c).has(flag::GROUP)).unwrap();
        assert_eq!((d.node(g).files, d.node(g).size), (2, 30));
        assert_eq!(d.path(g), r"C:\data\sub");
    }

    #[test]
    fn comparaison() {
        let (old, _) = sample();
        let old = old.dirs_only();
        // Nouvelle analyse : « sub » a grossi de 1000, « neuf » est apparu.
        let mut t = Tree::new(r"C:\data");
        let mut map = vec![Tree::ROOT];
        t.apply_batch(&mut map, DirBatch { id: 0, denied: false, entries: vec![file("a.txt", 100, 4096, 1000), file("b.bin", 5000, 8192, 3000), dir("SUB", 1), dir("neuf", 2)] });
        t.apply_batch(&mut map, DirBatch { id: 1, denied: false, entries: vec![file("c.txt", 1030, 4096, 9000), dir("deep", 3)] });
        t.apply_batch(&mut map, DirBatch { id: 2, denied: false, entries: vec![file("n", 50, 4096, 1)] });
        t.apply_batch(&mut map, DirBatch { id: 3, denied: false, entries: vec![file("d.log", 7, 0, 500)] });
        let d = compare(&old, &t);
        assert_eq!(d[&Tree::ROOT], Delta { size: 1050, alloc: 4096, new: false });
        assert_eq!(d[&map[1]], Delta { size: 1000, alloc: 0, new: false });
        assert_eq!(d[&map[2]], Delta { size: 50, alloc: 4096, new: true });
        // « deep » n'a pas changé : pas d'écart.
        assert!(!d.contains_key(&map[3]));

        // Racines différentes : l'ancienne analyse couvrait C:\data, la nouvelle C:\data\sub.
        let mut s = Tree::new(r"C:\data\sub");
        let mut m = vec![Tree::ROOT];
        s.apply_batch(&mut m, DirBatch { id: 0, denied: false, entries: vec![file("c.txt", 10, 0, 1), dir("deep", 1)] });
        s.apply_batch(&mut m, DirBatch { id: 1, denied: false, entries: vec![file("d.log", 7, 0, 500)] });
        let d = compare(&old, &s);
        assert_eq!(d[&Tree::ROOT].size, 17 - 37);
        // Et l'inverse : l'ancienne couvrait le sous-dossier.
        let d = compare(&s.dirs_only(), &t);
        assert_eq!(d[&map[1]].size, 1037 - 17);
        assert!(!d.contains_key(&Tree::ROOT));
        // Sans rapport : rien.
        assert!(compare(&Tree::new(r"D:\autre"), &t).is_empty());
    }
}
