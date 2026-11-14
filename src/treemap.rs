//! Carte proportionnelle (treemap) : découpage « squarified » d'un rectangle en cases
//! dont la surface est proportionnelle à la taille des fichiers et dossiers.

use crate::tree::{NodeId, Tree};
use eframe::egui::{pos2, vec2, Rect};

pub struct Item {
    pub rect: Rect,
    pub node: NodeId,
    /// 0 pour les enfants directs du dossier affiché.
    pub depth: u8,
    /// Case finale (fichier, ou dossier trop petit pour être détaillé) ; sinon cadre d'un dossier.
    pub leaf: bool,
    /// Cadre de dossier avec un bandeau de titre réservé en haut.
    pub header: bool,
    /// Teinte d'un fichier selon son extension (0 à 7), 255 pour le reste.
    pub hue: u8,
}

/// Hauteur du bandeau de titre d'un grand dossier.
pub const HEADER_H: f32 = 16.0;

/// Teinte stable d'une extension (FNV-1a).
fn ext_hue(name: &str) -> u8 {
    let mut h: u32 = 2_166_136_261;
    for b in crate::tree::extension(name).bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    (h % 8) as u8
}

/// Côté minimal d'un dossier pour que son contenu soit détaillé.
const MIN_SIDE: f32 = 8.0;
/// Garde-fou : nombre maximal de cases dessinées.
const MAX_ITEMS: usize = 40_000;

/// Découpe `rect` en cases de surfaces proportionnelles à `weights` (triés du plus grand
/// au plus petit, tous positifs), en gardant les cases aussi carrées que possible.
pub fn squarify(weights: &[f64], rect: Rect) -> Vec<Rect> {
    let total: f64 = weights.iter().sum();
    let (mut x, mut y, mut w, mut h) = (rect.left() as f64, rect.top() as f64, rect.width().max(0.0) as f64, rect.height().max(0.0) as f64);
    if total <= 0.0 || w <= 0.0 || h <= 0.0 {
        return vec![Rect::from_min_size(rect.min, vec2(0.0, 0.0)); weights.len()];
    }
    let scale = w * h / total;
    let area = |i: usize| weights[i] * scale;
    let mut out = Vec::with_capacity(weights.len());
    let mut i = 0;
    while i < weights.len() {
        let short = w.min(h).max(1e-9);
        // On allonge la rangée tant que sa case la plus étirée s'améliore.
        let (mut j, mut sum, mut best) = (i, 0.0, f64::INFINITY);
        while j < weights.len() {
            let s = sum + area(j);
            let worst = (short * short * area(i) / (s * s)).max(s * s / (short * short * area(j).max(1e-12)));
            if j > i && worst > best {
                break;
            }
            best = worst;
            sum = s;
            j += 1;
        }
        let thick = sum / short;
        if w >= h {
            // Bande verticale à gauche, cases empilées.
            let mut yy = y;
            for k in i..j {
                let hh = area(k) / thick;
                out.push(Rect::from_min_size(pos2(x as f32, yy as f32), vec2(thick as f32, hh as f32)));
                yy += hh;
            }
            x += thick;
            w -= thick;
        } else {
            // Bande horizontale en haut, cases côte à côte.
            let mut xx = x;
            for k in i..j {
                let ww = area(k) / thick;
                out.push(Rect::from_min_size(pos2(xx as f32, y as f32), vec2(ww as f32, thick as f32)));
                xx += ww;
            }
            y += thick;
            h -= thick;
        }
        i = j;
    }
    out
}

/// Cases de la carte du dossier `root` dans `rect`. Les cadres de dossiers précèdent leur contenu.
pub fn layout(tree: &Tree, root: NodeId, rect: Rect, on_disk: bool) -> Vec<Item> {
    let mut out = Vec::new();
    let mut stack = vec![(root, rect, 0u8)];
    while let Some((id, r, depth)) = stack.pop() {
        let mut kids: Vec<(NodeId, u64)> = tree.children(id).map(|c| (c, tree.node(c).weight(on_disk))).filter(|k| k.1 > 0).collect();
        if kids.is_empty() {
            continue;
        }
        kids.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        // Les éléments de moins d'un pixel carré ne sont pas dessinés.
        let total: f64 = kids.iter().map(|k| k.1 as f64).sum();
        let px = (r.width() * r.height()) as f64 / total;
        let keep = kids.partition_point(|k| k.1 as f64 * px >= 1.0).max(1);
        kids.truncate(keep);
        let weights: Vec<f64> = kids.iter().map(|k| k.1 as f64).collect();
        for (&(c, _), cr) in kids.iter().zip(squarify(&weights, r)) {
            let n = tree.node(c);
            let roomy = cr.width() >= MIN_SIDE && cr.height() >= MIN_SIDE;
            if n.is_dir() && tree.has_children(c) && roomy && out.len() < MAX_ITEMS {
                let header = cr.width() >= 60.0 && cr.height() >= 48.0;
                let inner = if header { Rect::from_min_max(pos2(cr.left() + 2.0, cr.top() + HEADER_H + 1.0), pos2(cr.right() - 2.0, cr.bottom() - 2.0)) } else { cr.shrink(1.0) };
                out.push(Item { rect: cr, node: c, depth, leaf: false, header, hue: 255 });
                stack.push((c, inner, depth.saturating_add(1)));
            } else if cr.width() >= 0.5 && cr.height() >= 0.5 {
                let hue = if n.is_dir() || n.has(crate::tree::flag::GROUP) { 255 } else { ext_hue(&n.name) };
                out.push(Item { rect: cr, node: c, depth, leaf: true, header: false, hue });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::tests::sample;

    #[test]
    fn decoupage() {
        // Exemple classique de l'article « Squarified Treemaps » : 6×4, surfaces 6 6 4 3 2 2 1.
        let w = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(6.0, 4.0));
        let r = squarify(&w, rect);
        assert_eq!(r.len(), 7);
        for (k, c) in r.iter().enumerate() {
            assert!((c.width() * c.height() - w[k] as f32).abs() < 1e-3, "surface de la case {k}");
            assert!(rect.expand(1e-3).contains_rect(*c), "case {k} hors du rectangle");
        }
        // Les deux premières cases : 3 × 2, empilées à gauche.
        assert!((r[0].width() - 3.0).abs() < 1e-4 && (r[0].height() - 2.0).abs() < 1e-4);
        assert!((r[1].top() - 22.0).abs() < 1e-4 && (r[1].left() - 10.0).abs() < 1e-4);
        // Aucune case ne chevauche une autre.
        for a in 0..7 {
            for b in a + 1..7 {
                assert!(!r[a].shrink(1e-3).intersects(r[b].shrink(1e-3)), "cases {a} et {b}");
            }
        }
        // Cas limites.
        assert!(squarify(&[], rect).is_empty());
        assert_eq!(squarify(&[5.0], rect)[0], rect);
        assert_eq!(squarify(&[1.0, 1.0], Rect::from_min_size(pos2(0.0, 0.0), vec2(0.0, 5.0))).len(), 2);
    }

    #[test]
    fn carte_d_un_arbre() {
        let (t, map) = sample();
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let items = layout(&t, Tree::ROOT, rect, false);
        // b.bin (5000), a.txt (100), sub (37) et son contenu ; le lien physique et les dossiers vides n'ont pas de case.
        let name = |i: &Item| t.node(i.node).name.to_string();
        assert_eq!(name(&items[0]), "b.bin");
        assert!(items[0].leaf && items[0].rect.width() * items[0].rect.height() > 0.95 * 400.0 * 300.0 * 5000.0 / 5137.0);
        assert!(items.iter().any(|i| i.node == map[1] && !i.leaf));
        assert!(items.iter().any(|i| name(i) == "c.txt" && i.depth == 1));
        assert!(!items.iter().any(|i| name(i) == "lien" || name(i) == "interdit"));
        // Sur le disque : d.log (0 octet alloué) disparaît.
        assert!(!layout(&t, Tree::ROOT, rect, true).iter().any(|i| name(i) == "d.log"));
        // Un dossier vide ne donne rien.
        assert!(layout(&t, map[2], rect, false).is_empty());
    }
}
