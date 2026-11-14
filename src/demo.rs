//! Outil de test (builds de débogage uniquement) : rejoue des clics et touches
//! décrits dans la variable d'environnement `DISKSCAN_SCRIPT`, pour vérifier
//! l'interface sans toucher à la souris.
//!
//! Format : `secondes:action:arguments` séparés par `;`
//! ex. `9:click:450,247;10:rclick:450,279;11:move:520,433;12:key:Escape`
//! Glisser-déposer : `1:down:700,215;1.5:move:500,215;2:up:450,215`
//! Autres actions : `dclick:x,y` (double-clic), `text:abc` (saisie),
//! `shot:chemin.bmp` (capture de la fenêtre), `pdf:chemin.pdf` (rapport sans boîte de dialogue),
//! `quit` (ferme l'app).
//! Les chemins ne doivent pas contenir de « ; » ; le « : » du lecteur est accepté.

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, RawInput};
use std::time::Instant;

enum Step {
    Move(Pos2),
    Button(PointerButton, Pos2, bool),
    Key(Key),
    Text(String),
    Shot(String),
    Pdf(String),
    Quit,
}

pub struct Script {
    start: Instant,
    steps: Vec<(f64, Step)>,
    next: usize,
    /// Captures demandées, en attente de l'image renvoyée par eframe.
    shots: Vec<String>,
    /// Rapports PDF à écrire (sans boîte de dialogue), pris en charge par l'application.
    pub pdf: Vec<String>,
}

impl Script {
    pub fn from_env() -> Option<Self> {
        let src = std::env::var("DISKSCAN_SCRIPT").ok()?;
        let mut steps = Vec::new();
        for part in src.split(';').filter(|s| !s.trim().is_empty()) {
            let mut it = part.trim().splitn(3, ':');
            let t: f64 = it.next()?.parse().ok()?;
            let action = it.next()?;
            let arg = it.next().unwrap_or("");
            let pos = || {
                let (x, y) = arg.split_once(',')?;
                Some(Pos2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
            };
            match action {
                "move" => steps.push((t, Step::Move(pos()?))),
                "click" | "rclick" => {
                    let b = if action == "click" { PointerButton::Primary } else { PointerButton::Secondary };
                    let p = pos()?;
                    steps.push((t, Step::Move(p)));
                    steps.push((t + 0.05, Step::Button(b, p, true)));
                    steps.push((t + 0.15, Step::Button(b, p, false)));
                }
                "dclick" => {
                    let p = pos()?;
                    steps.push((t, Step::Move(p)));
                    for k in 0..2 {
                        let dt = k as f64 * 0.12;
                        steps.push((t + 0.05 + dt, Step::Button(PointerButton::Primary, p, true)));
                        steps.push((t + 0.10 + dt, Step::Button(PointerButton::Primary, p, false)));
                    }
                }
                // Glisser-déposer : `down`, un ou plusieurs `move`, puis `up`.
                "down" | "up" => {
                    let p = pos()?;
                    steps.push((t, Step::Move(p)));
                    steps.push((t + 0.05, Step::Button(PointerButton::Primary, p, action == "down")));
                }
                "key" => {
                    let k = Key::from_name(arg)?;
                    steps.push((t, Step::Key(k)));
                }
                "text" => steps.push((t, Step::Text(arg.to_string()))),
                "shot" => steps.push((t, Step::Shot(arg.to_string()))),
                "pdf" => steps.push((t, Step::Pdf(arg.to_string()))),
                "quit" => steps.push((t, Step::Quit)),
                _ => {}
            }
        }
        steps.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(Self { start: Instant::now(), steps, next: 0, shots: Vec::new(), pdf: Vec::new() })
    }

    pub fn pending(&self) -> bool {
        self.next < self.steps.len() || !self.shots.is_empty()
    }

    pub fn inject(&mut self, ctx: &egui::Context, raw: &mut RawInput) {
        // Captures arrivées depuis l'image précédente.
        for ev in &raw.events {
            if let Event::Screenshot { image, .. } = ev {
                if !self.shots.is_empty() {
                    let path = self.shots.remove(0);
                    let _ = std::fs::write(path, bmp(image));
                }
            }
        }
        let now = self.start.elapsed().as_secs_f64();
        while self.next < self.steps.len() && self.steps[self.next].0 <= now {
            match &self.steps[self.next].1 {
                Step::Move(p) => raw.events.push(Event::PointerMoved(*p)),
                Step::Button(button, pos, pressed) => raw.events.push(Event::PointerButton {
                    pos: *pos,
                    button: *button,
                    pressed: *pressed,
                    modifiers: Modifiers::NONE,
                }),
                Step::Key(key) => {
                    for pressed in [true, false] {
                        raw.events.push(Event::Key {
                            key: *key,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: Modifiers::NONE,
                        });
                    }
                }
                Step::Text(t) => raw.events.push(Event::Text(t.clone())),
                Step::Shot(path) => {
                    self.shots.push(path.clone());
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                }
                Step::Pdf(path) => self.pdf.push(path.clone()),
                Step::Quit => {
                    // On laisse les captures en attente s'écrire avant de fermer.
                    if !self.shots.is_empty() {
                        return;
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            self.next += 1;
        }
    }
}

/// Image BMP 24 bits (lignes de bas en haut, alignées sur 4 octets).
fn bmp(img: &egui::ColorImage) -> Vec<u8> {
    let [w, h] = img.size;
    let row = (w * 3 + 3) / 4 * 4;
    let mut out = Vec::with_capacity(54 + row * h);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + row * h) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        for x in 0..w {
            let c = img.pixels[y * w + x];
            out.extend_from_slice(&[c.b(), c.g(), c.r()]);
        }
        out.extend(std::iter::repeat(0u8).take(row - w * 3));
    }
    out
}

/// `DISKSCAN_DATA_DIR` : réglages et analyses enregistrées dans ce dossier plutôt que dans
/// `%APPDATA%\DiskScan\data`, pour tester sans toucher aux vraies données.
pub fn data_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("DISKSCAN_DATA_DIR").map(std::path::PathBuf::from)
}

/// `DISKSCAN_FAKE_DRIVES` : lecteurs fictifs à la place des vrais (captures d'écran),
/// ex. `C:\|Windows|476|182;S:\|Données|931|512` (racine, nom, capacité et espace libre en Go).
pub fn fake_drives() -> Option<Vec<crate::win::Drive>> {
    let src = std::env::var("DISKSCAN_FAKE_DRIVES").ok()?;
    let gib = |s: &str| s.trim().parse::<f64>().ok().map(|v| (v * 1024.0 * 1024.0 * 1024.0) as u64);
    Some(
        src.split(';')
            .filter_map(|d| {
                let f: Vec<&str> = d.split('|').collect();
                (f.len() == 4).then_some(())?;
                Some(crate::win::Drive { root: f[0].trim().to_string(), label: f[1].trim().to_string(), total: gib(f[2])?, free: gib(f[3])? })
            })
            .collect(),
    )
}

pub fn keep_alive(ctx: &egui::Context) {
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
}
