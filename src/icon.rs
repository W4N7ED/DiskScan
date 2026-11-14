// Logo DiskScan (carré bleu, disque dont une part est occupée), partagé entre
// l'application (icône de fenêtre) et build.rs (icône de l'exe).
// Ce fichier ne doit dépendre de rien : il est inclus tel quel par build.rs.

/// Pixels RGBA (non prémultipliés) du logo, anticrénelé par suréchantillonnage 4×4.
pub fn logo_rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    const ACCENT: [f32; 3] = [30.0, 99.0, 214.0];
    // Part occupée : de midi, dans le sens horaire, sur 220°.
    const USED_DEG: f32 = 220.0;

    // Coordonnées du dessin : carré de 32 × 32.
    let scale = size as f32 / 32.0;
    let inside = |u: f32, v: f32| {
        let cx = u.clamp(8.0, 24.0);
        let cy = v.clamp(8.0, 24.0);
        (u - cx).powi(2) + (v - cy).powi(2) <= 64.0
    };

    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0f32; 3];
            let mut covered = 0u32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (x as f32 + (sx as f32 + 0.5) / SS as f32) / scale;
                    let v = (y as f32 + (sy as f32 + 0.5) / SS as f32) / scale;
                    if !inside(u, v) {
                        continue;
                    }
                    let mut c = ACCENT;
                    let (dx, dy) = (u - 16.0, v - 16.0);
                    let d = (dx * dx + dy * dy).sqrt();
                    if d <= 9.5 && d > 3.2 {
                        let mut ang = dx.atan2(-dy).to_degrees();
                        if ang < 0.0 {
                            ang += 360.0;
                        }
                        let op = if ang <= USED_DEG { 1.0 } else { 0.4 };
                        for ch in c.iter_mut() {
                            *ch += (255.0 - *ch) * op;
                        }
                    }
                    for i in 0..3 {
                        acc[i] += c[i];
                    }
                    covered += 1;
                }
            }
            if covered > 0 {
                let i = ((y * size + x) * 4) as usize;
                let k = covered as f32;
                out[i] = (acc[0] / k) as u8;
                out[i + 1] = (acc[1] / k) as u8;
                out[i + 2] = (acc[2] / k) as u8;
                out[i + 3] = (255.0 * k / (SS * SS) as f32) as u8;
            }
        }
    }
    out
}
