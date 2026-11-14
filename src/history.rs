//! Dernière analyse : enregistrée sur disque pour être rechargée au lancement
//! et comparée à la suivante (dossiers qui ont grossi ou diminué).
//!
//! Format texte, une ligne par élément, champs séparés par des tabulations :
//! - en-tête : `diskscan-scan-1`, racine, date, durée (ms), interrompue (0/1), comparée (0/1)
//! - `D` : un dossier, dans l'ordre de l'arbre (profondeur, nom, taille, taille sur le disque,
//!   fichiers, dossiers, illisibles, date, indicateurs, écart, écart sur le disque, nouveau)
//! - `F` : un des plus gros fichiers · `E` : une extension · `A` : une tranche d'ancienneté
//!
//! Seuls les dossiers sont gardés : les fichiers d'un dossier se déduisent de ses totaux.

use crate::tree::{cmp_names, flag, AgeStat, Delta, ExtStat, NodeId, Summary, TopFile, Tree, NONE};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

pub struct Saved {
    pub tree: Tree,
    pub summary: Summary,
    pub deltas: HashMap<NodeId, Delta>,
    /// Les écarts viennent d'une comparaison (vide : aucun changement).
    pub compared: bool,
    /// Date de l'analyse (secondes Unix).
    pub when: i64,
    pub elapsed_ms: u64,
    pub cancelled: bool,
}

const MAGIC: &str = "diskscan-scan-1";
/// Indicateurs conservés d'une analyse à l'autre.
const KEPT_FLAGS: u8 = flag::LINK | flag::DENIED | flag::PENDING;

fn path() -> Option<PathBuf> {
    Some(eframe::storage_dir("DiskScan")?.join("last_scan.tsv"))
}

/// Un nom de fichier Windows ne contient ni tabulation ni saut de ligne ; par prudence on les neutralise.
fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

pub fn encode(tree: &Tree, summary: &Summary, deltas: &HashMap<NodeId, Delta>, compared: bool, when: i64, elapsed_ms: u64, cancelled: bool) -> String {
    let mut out = format!("{MAGIC}\t{}\t{when}\t{elapsed_ms}\t{}\t{}\n", clean(&tree.root_path), u8::from(cancelled), u8::from(compared));
    let mut stack = vec![(Tree::ROOT, 0u32)];
    while let Some((id, depth)) = stack.pop() {
        let n = tree.node(id);
        let d = deltas.get(&id).copied().unwrap_or_default();
        let _ = writeln!(
            out,
            "D\t{depth}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            clean(&n.name),
            n.size,
            n.alloc,
            n.files,
            n.dirs,
            n.denied,
            n.mtime,
            n.flags & KEPT_FLAGS,
            d.size,
            d.alloc,
            u8::from(d.new)
        );
        // Sous-dossiers par ordre alphabétique (le fichier ne dépend pas de l'ordre de parcours),
        // empilés à l'envers pour ressortir dans cet ordre.
        let mut dirs: Vec<NodeId> = tree.children(id).filter(|&c| tree.node(c).is_dir()).collect();
        dirs.sort_by(|&a, &b| cmp_names(&tree.node(a).name, &tree.node(b).name));
        stack.extend(dirs.into_iter().rev().map(|c| (c, depth + 1)));
    }
    for f in &summary.top {
        let _ = writeln!(out, "F\t{}\t{}\t{}\t{}", clean(&f.path), f.size, f.alloc, f.mtime);
    }
    for e in &summary.exts {
        let _ = writeln!(out, "E\t{}\t{}\t{}\t{}", clean(&e.ext), e.files, e.size, e.alloc);
    }
    for (i, a) in summary.ages.iter().enumerate() {
        let _ = writeln!(out, "A\t{i}\t{}\t{}\t{}", a.files, a.size, a.alloc);
    }
    out
}

pub fn decode(text: &str) -> Option<Saved> {
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next()?.split('\t').collect();
    if head.len() != 6 || head[0] != MAGIC {
        return None;
    }
    let mut tree = Tree::new(head[1]);
    let mut summary = Summary::default();
    let mut deltas = HashMap::new();
    // Dernier dossier vu à chaque profondeur : le parent du suivant.
    let mut parents: Vec<NodeId> = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split('\t').collect();
        match f[0] {
            "D" if f.len() == 13 => {
                let depth: usize = f[1].parse().ok()?;
                let (size, alloc) = (f[3].parse().ok()?, f[4].parse().ok()?);
                let (files, dirs, denied) = (f[5].parse().ok()?, f[6].parse().ok()?, f[7].parse().ok()?);
                let mtime = f[8].parse().ok()?;
                let flags = f[9].parse::<u8>().ok()? & KEPT_FLAGS;
                let id = if depth == 0 {
                    if !parents.is_empty() {
                        return None;
                    }
                    let r = &mut tree.nodes[0];
                    (r.size, r.alloc, r.files, r.dirs, r.denied, r.mtime, r.flags) = (size, alloc, files, dirs, denied, mtime, flags | flag::DIR);
                    Tree::ROOT
                } else {
                    let parent = *parents.get(depth - 1)?;
                    tree.add_dir_raw(parent, f[2], size, alloc, files, dirs, denied, mtime, flags)
                };
                parents.truncate(depth);
                parents.push(id);
                let d = Delta { size: f[10].parse().ok()?, alloc: f[11].parse().ok()?, new: f[12] == "1" };
                if d != Delta::default() {
                    deltas.insert(id, d);
                }
            }
            "F" if f.len() == 5 => summary.top.push(TopFile {
                path: f[1].to_string(),
                size: f[2].parse().ok()?,
                alloc: f[3].parse().ok()?,
                mtime: f[4].parse().ok()?,
                node: NONE,
            }),
            "E" if f.len() == 5 => summary.exts.push(ExtStat { ext: f[1].to_string(), files: f[2].parse().ok()?, size: f[3].parse().ok()?, alloc: f[4].parse().ok()? }),
            "A" if f.len() == 5 => {
                let i: usize = f[1].parse().ok()?;
                *summary.ages.get_mut(i)? = AgeStat { files: f[2].parse().ok()?, size: f[3].parse().ok()?, alloc: f[4].parse().ok()? };
            }
            _ => return None,
        }
    }
    if parents.is_empty() {
        return None;
    }
    tree.add_groups();
    Some(Saved { tree, summary, deltas, compared: head[5] == "1", when: head[2].parse().ok()?, elapsed_ms: head[3].parse().ok()?, cancelled: head[4] == "1" })
}

pub fn save(content: String) {
    if let Some(p) = path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, content);
    }
}

pub fn load() -> Option<Saved> {
    decode(&std::fs::read_to_string(path()?).ok()?)
}

/// Oublie la dernière analyse (bouton « Effacer les résultats »).
pub fn delete() {
    if let Some(p) = path() {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::tests::sample;

    #[test]
    fn aller_retour() {
        let (t, map) = sample();
        let summary = t.summarize(10_000, false);
        let mut deltas = HashMap::new();
        deltas.insert(map[1], Delta { size: -500, alloc: 4096, new: false });
        deltas.insert(map[3], Delta { size: 7, alloc: 0, new: true });
        let text = encode(&t, &summary, &deltas, true, 1_700_000_000, 1234, true);
        let s = decode(&text).unwrap();
        assert_eq!((s.when, s.elapsed_ms, s.cancelled, s.compared), (1_700_000_000, 1234, true, true));
        assert_eq!(s.tree.root_path, r"C:\data");

        // Mêmes totaux, mêmes dossiers, fichiers regroupés.
        let r = s.tree.node(Tree::ROOT);
        assert_eq!((r.size, r.alloc, r.files, r.dirs, r.denied, r.mtime), (5137, 16384, 5, 4, 1, 9000));
        let sub = s.tree.locate(r"C:\data\sub").unwrap();
        let deep = s.tree.locate(r"C:\data\sub\deep").unwrap();
        assert_eq!((s.tree.node(sub).size, s.tree.node(sub).files), (37, 3));
        assert!(s.tree.node(s.tree.locate(r"C:\data\interdit").unwrap()).has(flag::DENIED));
        assert!(s.tree.node(s.tree.locate(r"C:\data\jonction").unwrap()).has(flag::LINK));
        let group = s.tree.children(0).find(|&c| s.tree.node(c).has(flag::GROUP)).unwrap();
        assert_eq!((s.tree.node(group).files, s.tree.node(group).size), (2, 5100));

        // Écarts rattachés aux bons dossiers.
        assert_eq!(s.deltas.len(), 2);
        assert_eq!(s.deltas[&sub], Delta { size: -500, alloc: 4096, new: false });
        assert_eq!(s.deltas[&deep], Delta { size: 7, alloc: 0, new: true });

        // Vues complémentaires.
        assert_eq!(s.summary.top.len(), 5);
        assert_eq!((s.summary.top[0].path.as_str(), s.summary.top[0].size), (r"C:\data\b.bin", 5000));
        assert_eq!(s.summary.exts, summary.exts);
        assert_eq!(s.summary.ages, summary.ages);

        // Réenregistrer l'arbre rechargé redonne le même texte (aux fichiers près, déjà absents).
        assert_eq!(encode(&s.tree, &s.summary, &s.deltas, s.compared, s.when, s.elapsed_ms, s.cancelled), text);
        // Et l'arbre réduit aux dossiers s'enregistre comme l'arbre complet.
        assert_eq!(encode(&t.dirs_only(), &summary, &HashMap::new(), false, 1, 2, false), encode(&t, &summary, &HashMap::new(), false, 1, 2, false));
    }

    #[test]
    fn fichiers_invalides() {
        assert!(decode("").is_none());
        assert!(decode("autre chose").is_none());
        assert!(decode("netscan-scan-1\tx\t1\t2\t0\t0\n").is_none());
        // Ancien en-tête (sans le drapeau « comparée ») : refusé plutôt que mal lu.
        assert!(decode("diskscan-scan-1\tC:\\x\t1\t2\t0\nD\t0\tC:\\x\t10\t10\t1\t0\t0\t5\t0\t0\t0\t0\n").is_none());
        // En-tête seul, sans racine : refusé.
        assert!(decode("diskscan-scan-1\tC:\\x\t1\t2\t0\t0\n").is_none());
        // Dossier sans parent, ligne tronquée, nombre illisible.
        let head = "diskscan-scan-1\tC:\\x\t1\t2\t0\t0\nD\t0\tC:\\x\t10\t10\t1\t0\t0\t5\t0\t0\t0\t0\n";
        assert!(decode(head).is_some());
        assert!(decode(&format!("{head}D\t3\ta\t1\t1\t1\t0\t0\t5\t0\t0\t0\t0\n")).is_none());
        assert!(decode(&format!("{head}D\t1\ta\t1\n")).is_none());
        assert!(decode(&format!("{head}D\t1\ta\tx\t1\t1\t0\t0\t5\t0\t0\t0\t0\n")).is_none());
        assert!(decode(&format!("{head}Z\t1\n")).is_none());
    }
}
