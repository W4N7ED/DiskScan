//! Moteur d'analyse : parcours du disque par un pool de threads, annulable.
//!
//! Chaque dossier est lu en un seul appel système par bloc d'entrées
//! (`GetFileInformationByHandleEx` + `FileIdBothDirectoryInfo`) : nom, taille réelle,
//! taille allouée, date, attributs et identifiant de fichier arrivent ensemble,
//! sans ouvrir les fichiers un par un. Les liens physiques sont repérés par l'arbre
//! grâce à l'identifiant de fichier (voir `Tree::apply_batch`).

use crate::tree::{flag, DirBatch, Entry, Tree, NONE};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

pub enum ScanEvent {
    Dir(DirBatch),
    Finished { cancelled: bool },
}

pub struct ScanHandle {
    rx: Receiver<ScanEvent>,
    cancel: Arc<AtomicBool>,
}

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn try_recv(&self) -> Option<ScanEvent> {
        self.rx.try_recv().ok()
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct Job {
    path: String,
    id: u32,
}

struct Queue {
    stack: Vec<Job>,
    /// Threads en train de lire un dossier (ils peuvent encore ajouter du travail).
    active: usize,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    next_id: AtomicU32,
    cancel: Arc<AtomicBool>,
}

/// Nombre de threads conseillé : un par cœur, 16 au plus.
pub fn default_threads() -> usize {
    thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16)
}

/// Lance l'analyse de `root` (chemin absolu). Le dossier racine porte le numéro 0.
/// `notify` est appelé quand l'analyse se termine, pour réveiller l'interface.
pub fn start_scan<F>(root: String, threads: usize, notify: F) -> ScanHandle
where
    F: Fn() + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let shared = Shared {
        queue: Mutex::new(Queue { stack: vec![Job { path: root, id: 0 }], active: 0 }),
        wake: Condvar::new(),
        next_id: AtomicU32::new(1),
        cancel: cancel.clone(),
    };
    let threads = threads.clamp(1, 64);
    thread::spawn(move || {
        thread::scope(|s| {
            for _ in 0..threads {
                let tx = tx.clone();
                let shared = &shared;
                let _ = thread::Builder::new().name("diskscan".into()).spawn_scoped(s, move || worker(shared, tx));
            }
        });
        let _ = tx.send(ScanEvent::Finished { cancelled: shared.cancel.load(Ordering::Relaxed) });
        notify();
    });
    ScanHandle { rx, cancel }
}

fn worker(sh: &Shared, tx: Sender<ScanEvent>) {
    // Tampon de lecture aligné sur 8 octets (les entrées contiennent des entiers 64 bits).
    let mut buf = vec![0u64; 8192];
    loop {
        let job = {
            let mut q = sh.queue.lock().unwrap();
            loop {
                if sh.cancel.load(Ordering::Relaxed) {
                    sh.wake.notify_all();
                    return;
                }
                if let Some(j) = q.stack.pop() {
                    q.active += 1;
                    break j;
                }
                if q.active == 0 {
                    // Plus rien à lire et plus personne pour en ajouter : terminé.
                    sh.wake.notify_all();
                    return;
                }
                q = sh.wake.wait_timeout(q, Duration::from_millis(50)).unwrap().0;
            }
        };
        let (batch, subdirs) = read_dir(&job, sh, &mut buf);
        // Le lot part avant que ses sous-dossiers soient distribués : l'interface reçoit
        // toujours un dossier avant son contenu.
        if tx.send(ScanEvent::Dir(batch)).is_err() {
            sh.cancel.store(true, Ordering::Relaxed);
        }
        let mut q = sh.queue.lock().unwrap();
        q.stack.extend(subdirs);
        q.active -= 1;
        drop(q);
        sh.wake.notify_all();
    }
}

fn join(parent: &str, name: &str) -> String {
    let mut p = String::with_capacity(parent.len() + name.len() + 1);
    p.push_str(parent);
    if !p.ends_with('\\') {
        p.push('\\');
    }
    p.push_str(name);
    p
}

/// Lit un dossier : le lot pour l'interface et les sous-dossiers à parcourir.
fn read_dir(job: &Job, sh: &Shared, buf: &mut [u64]) -> (DirBatch, Vec<Job>) {
    let mut entries = Vec::new();
    let mut subdirs = Vec::new();
    let ok = sys::list(&job.path, buf, &sh.cancel, |raw| {
        let mut e = Entry { name: raw.name.into(), size: 0, alloc: 0, mtime: raw.mtime, flags: 0, file_id: 0, id: NONE };
        if raw.dir {
            e.flags |= flag::DIR;
            if raw.link {
                // Jonction ou lien symbolique : on ne le suit pas (boucles, double compte).
                e.flags |= flag::LINK;
            } else {
                e.id = sh.next_id.fetch_add(1, Ordering::Relaxed);
                subdirs.push(Job { path: join(&job.path, &e.name), id: e.id });
            }
        } else {
            e.size = raw.size;
            e.alloc = raw.alloc;
            if raw.link {
                e.flags |= flag::LINK;
            }
            // Les liens physiques (même identifiant sous plusieurs noms) sont résolus par l'arbre.
            e.file_id = raw.file_id;
        }
        entries.push(e);
    });
    if !ok {
        // Illisible (accès refusé, dossier disparu…) : signalé, l'analyse continue.
        // Ce qui a pu être lu avant l'erreur est gardé.
        return (DirBatch { id: job.id, denied: true, entries }, subdirs);
    }
    (DirBatch { id: job.id, denied: false, entries }, subdirs)
}

/// Analyse complète et bloquante (tests, petits dossiers).
#[allow(dead_code)]
pub fn scan_blocking(root: &str, threads: usize) -> Tree {
    let handle = start_scan(root.to_string(), threads, || {});
    let mut tree = Tree::new(root);
    let mut map = vec![Tree::ROOT];
    loop {
        match handle.rx.recv() {
            Ok(ScanEvent::Dir(b)) => tree.apply_batch(&mut map, b),
            Ok(ScanEvent::Finished { .. }) | Err(_) => return tree,
        }
    }
}

// ---------------------------------------------------------------------------
// API Windows
// ---------------------------------------------------------------------------

mod sys {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(name: *const u16, access: u32, share: u32, security: *mut c_void, disposition: u32, flags: u32, template: isize) -> isize;
        fn GetFileInformationByHandleEx(handle: isize, class: u32, buffer: *mut c_void, size: u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
        fn GetLastError() -> u32;
    }

    const INVALID_HANDLE: isize = -1;
    const FILE_LIST_DIRECTORY: u32 = 0x0001;
    const SHARE_ALL: u32 = 0x7;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const ERROR_NO_MORE_FILES: u32 = 18;

    const ATTR_DIRECTORY: u32 = 0x10;
    const ATTR_REPARSE_POINT: u32 = 0x400;
    const TAG_MOUNT_POINT: u32 = 0xA000_0003;
    const TAG_SYMLINK: u32 = 0xA000_000C;

    /// Classes d'information : avec identifiant de fichier (NTFS, ReFS) ou sans (FAT, exFAT, certains partages).
    const FILE_ID_BOTH_DIR_INFO: u32 = 10;
    const FILE_FULL_DIR_INFO: u32 = 14;
    const FILE_FULL_DIR_RESTART_INFO: u32 = 15;

    pub struct Raw<'a> {
        pub name: &'a str,
        pub size: u64,
        pub alloc: u64,
        pub mtime: i64,
        pub file_id: u64,
        pub dir: bool,
        /// Lien symbolique ou jonction (point de montage compris).
        pub link: bool,
    }

    /// Chemin étendu (`\\?\`), terminé par zéro : pas de limite à 260 caractères.
    pub fn wide_path(path: &str) -> Vec<u16> {
        let long = if path.starts_with(r"\\?\") {
            path.to_string()
        } else if let Some(unc) = path.strip_prefix(r"\\") {
            format!(r"\\?\UNC\{unc}")
        } else {
            format!(r"\\?\{path}")
        };
        long.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Dates Windows (pas de 100 ns depuis 1601) → secondes Unix.
    pub fn filetime_to_unix(ft: i64) -> i64 {
        if ft <= 0 {
            0
        } else {
            ft / 10_000_000 - 11_644_473_600
        }
    }

    struct Handle(isize);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    /// Appelle `each` pour chaque entrée du dossier (sauf « . » et « .. »).
    /// Renvoie faux si le dossier n'a pas pu être lu en entier.
    pub fn list(path: &str, buf: &mut [u64], cancel: &AtomicBool, mut each: impl FnMut(Raw)) -> bool {
        let wide = wide_path(path);
        let h = unsafe { CreateFileW(wide.as_ptr(), FILE_LIST_DIRECTORY, SHARE_ALL, std::ptr::null_mut(), OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, 0) };
        if h == INVALID_HANDLE {
            return false;
        }
        let handle = Handle(h);
        let bytes = (buf.len() * 8) as u32;
        let base = buf.as_mut_ptr() as *mut u8;
        let mut class = FILE_ID_BOTH_DIR_INFO;
        let mut first = true;
        let mut name16: Vec<u16> = Vec::with_capacity(260);
        let mut name8 = String::with_capacity(260);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return true;
            }
            let ok = unsafe { GetFileInformationByHandleEx(handle.0, class, base as *mut c_void, bytes) };
            if ok == 0 {
                let err = unsafe { GetLastError() };
                if err == ERROR_NO_MORE_FILES {
                    return true;
                }
                if first && class == FILE_ID_BOTH_DIR_INFO {
                    // Système de fichiers sans identifiants : on recommence avec la classe de base.
                    class = FILE_FULL_DIR_RESTART_INFO;
                    continue;
                }
                return false;
            }
            first = false;
            let with_id = class == FILE_ID_BOTH_DIR_INFO;
            if class == FILE_FULL_DIR_RESTART_INFO {
                class = FILE_FULL_DIR_INFO;
            }
            // Décalages des champs de FILE_ID_BOTH_DIR_INFO / FILE_FULL_DIR_INFO (identiques jusqu'à EaSize).
            let name_at = if with_id { 104 } else { 68 };
            let mut off = 0usize;
            loop {
                let rd32 = |o: usize| unsafe { (base.add(off + o) as *const u32).read_unaligned() };
                let rd64 = |o: usize| unsafe { (base.add(off + o) as *const i64).read_unaligned() };
                let next = rd32(0) as usize;
                let attrs = rd32(56);
                let name_len = rd32(60) as usize / 2;
                if off + name_at + name_len * 2 > bytes as usize {
                    return false;
                }
                name16.clear();
                for i in 0..name_len {
                    name16.push(unsafe { (base.add(off + name_at + i * 2) as *const u16).read_unaligned() });
                }
                let dots = name16.len() <= 2 && name16.iter().all(|&c| c == b'.' as u16);
                if !dots {
                    name8.clear();
                    name8.extend(char::decode_utf16(name16.iter().copied()).map(|r| r.unwrap_or('\u{fffd}')));
                    let dir = attrs & ATTR_DIRECTORY != 0;
                    let reparse = attrs & ATTR_REPARSE_POINT != 0;
                    // Pour un point d'analyse, EaSize contient son étiquette. Seuls les liens et
                    // jonctions sont écartés : les dossiers OneDrive, par exemple, restent parcourus.
                    let tag = rd32(64);
                    let link = reparse && (tag == TAG_SYMLINK || tag == TAG_MOUNT_POINT || (dir && tag == 0));
                    each(Raw {
                        name: &name8,
                        size: rd64(40).max(0) as u64,
                        alloc: rd64(48).max(0) as u64,
                        mtime: filetime_to_unix(rd64(24)),
                        file_id: if with_id { rd64(96) as u64 } else { 0 },
                        dir,
                        link,
                    });
                }
                if next == 0 {
                    break;
                }
                off += next;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::SortKey;

    #[test]
    fn chemins_etendus() {
        let s = |p: &str| String::from_utf16(&sys::wide_path(p)).unwrap();
        assert_eq!(s(r"C:\Users"), "\\\\?\\C:\\Users\0");
        assert_eq!(s(r"\\serveur\partage\x"), "\\\\?\\UNC\\serveur\\partage\\x\0");
        assert_eq!(s(r"\\?\C:\x"), "\\\\?\\C:\\x\0");
        assert_eq!(join(r"C:\", "a"), r"C:\a");
        assert_eq!(join(r"C:\a", "b"), r"C:\a\b");
    }

    #[test]
    fn dates_windows() {
        assert_eq!(sys::filetime_to_unix(0), 0);
        // 1er janvier 1970 et 2001-09-09 01:46:40 UTC.
        assert_eq!(sys::filetime_to_unix(116_444_736_000_000_000), 0);
        assert_eq!(sys::filetime_to_unix(126_444_736_000_000_000), 1_000_000_000);
    }

    /// Analyse d'un vrai petit dossier, créé dans le dossier temporaire (jamais supprimé :
    /// les fichiers sont simplement réécrits à chaque exécution).
    #[test]
    fn analyse_d_un_dossier_reel() {
        let root = std::env::temp_dir().join("diskscan_test_moteur");
        let sub = root.join("sous-dossier").join("profond");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(root.join("vide")).unwrap();
        std::fs::write(root.join("un.txt"), vec![b'a'; 1000]).unwrap();
        std::fs::write(root.join("gros.bin"), vec![0u8; 300_000]).unwrap();
        std::fs::write(sub.join("deux.log"), vec![b'b'; 50_000]).unwrap();
        // Lien physique vers gros.bin : même fichier, ne doit compter qu'une fois.
        let link = root.join("sous-dossier").join("lien-physique.bin");
        if !link.exists() {
            std::fs::hard_link(root.join("gros.bin"), &link).unwrap();
        }
        // Jonction vers la racine : ne doit pas être suivie (sinon boucle infinie).
        let junction = root.join("jonction");
        if !junction.exists() {
            let st = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&junction).arg(&root).output().unwrap();
            assert!(st.status.success(), "mklink /J a échoué");
        }

        let root_s = root.to_string_lossy().to_string();
        for threads in [1, 8] {
            let t = scan_blocking(&root_s, threads);
            let r = t.node(Tree::ROOT);
            assert_eq!(r.size, 351_000, "taille réelle ({threads} threads)");
            assert_eq!(r.files, 4, "fichiers");
            assert_eq!(r.dirs, 4, "dossiers");
            assert_eq!(r.denied, 0);
            assert!(r.alloc >= 350_000 && r.alloc < 400_000, "taille sur le disque : {}", r.alloc);
            assert!(r.mtime > 1_600_000_000);
            let j = t.children(0).find(|&c| &*t.node(c).name == "jonction").unwrap();
            assert!(t.node(j).has(flag::LINK) && !t.has_children(j) && !t.node(j).has(flag::PENDING));
            // Un seul des deux noms de gros.bin porte la taille.
            let dups = t.nodes.iter().filter(|n| n.has(flag::DUP)).count();
            assert_eq!(dups, 1);
            let first = t.sorted_children(0, SortKey::Size, false, false, true)[0];
            assert!(t.node(first).size >= 300_000);
            assert!(t.nodes.iter().all(|n| !n.has(flag::PENDING)));
        }
    }

    /// Vrai parcours d'un dossier au choix, avec durée et détail des deux premiers niveaux :
    /// `set DISKSCAN_TEST_DIR=C:\Users` puis `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn analyse_au_choix() {
        let Ok(root) = std::env::var("DISKSCAN_TEST_DIR") else { return };
        let start = std::time::Instant::now();
        let t = scan_blocking(&root, default_threads());
        let r = t.node(Tree::ROOT);
        println!("{root} : {} o réels, {} o sur le disque, {} fichiers, {} dossiers, {} illisibles, {:.2} s", r.size, r.alloc, r.files, r.dirs, r.denied, start.elapsed().as_secs_f32());
        let mut stack = vec![(Tree::ROOT, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            for c in t.sorted_children(id, SortKey::Size, true, true, true) {
                let n = t.node(c);
                if depth < 2 {
                    println!("{}{:<40} {:>12} {:>12} fl={:02x}", "  ".repeat(depth + 1), n.name, n.size, n.alloc, n.flags);
                    stack.push((c, depth + 1));
                }
            }
        }
    }

    #[test]
    fn dossier_inexistant() {
        let t = scan_blocking(r"C:\ce-dossier-n-existe-pas-diskscan", 2);
        let r = t.node(Tree::ROOT);
        assert!(r.has(flag::DENIED));
        assert_eq!((r.denied, r.files), (1, 0));
    }

    #[test]
    fn annulation() {
        let h = start_scan(std::env::temp_dir().to_string_lossy().to_string(), 4, || {});
        h.cancel();
        loop {
            match h.rx.recv() {
                Ok(ScanEvent::Finished { cancelled }) => {
                    assert!(cancelled);
                    break;
                }
                Ok(_) => {}
                Err(_) => panic!("canal fermé sans fin d'analyse"),
            }
        }
    }
}
