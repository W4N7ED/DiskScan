//! Appels à Windows hors analyse : thème, écrans, lecteurs, Explorateur, propriétés, corbeille.

use std::ffi::c_void;

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(key: isize, sub_key: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, size: *mut u32) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn MonitorFromPoint(pt: Point, flags: u32) -> isize;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetVolumeInformationW(root: *const u16, name: *mut u16, name_len: u32, serial: *mut u32, max_component: *mut u32, flags: *mut u32, fs: *mut u16, fs_len: u32) -> i32;
    fn GetDiskFreeSpaceExW(dir: *const u16, free_for_caller: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    fn SetErrorMode(mode: u32) -> u32;
}

#[repr(C)]
struct ShellExecuteInfoW {
    size: u32,
    mask: u32,
    hwnd: isize,
    verb: *const u16,
    file: *const u16,
    parameters: *const u16,
    directory: *const u16,
    show: i32,
    inst_app: isize,
    id_list: *mut c_void,
    class: *const u16,
    key_class: isize,
    hot_key: u32,
    icon: isize,
    process: isize,
}

#[repr(C)]
struct ShFileOpStructW {
    hwnd: isize,
    func: u32,
    from: *const u16,
    to: *const u16,
    flags: u16,
    any_aborted: i32,
    name_mappings: *mut c_void,
    progress_title: *const u16,
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
    fn SHFileOperationW(op: *mut ShFileOpStructW) -> i32;
    fn SHQueryRecycleBinW(root: *const u16, info: *mut ShQueryRbInfo) -> i32;
}

#[repr(C)]
struct ShQueryRbInfo {
    size: u32,
    bytes: i64,
    items: i64,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Windows est-il réglé sur le thème sombre pour les applications ?
pub fn system_prefers_dark() -> Option<bool> {
    const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
    const RRF_RT_REG_DWORD: u32 = 0x10;
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut light: u32 = 1;
    let mut size: u32 = 4;
    let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut light as *mut u32 as *mut c_void, &mut size) };
    (rc == 0).then_some(light == 0)
}

/// Le point (pixels physiques du bureau) est-il sur un écran branché ?
pub fn point_on_screen(x: i32, y: i32) -> bool {
    // MONITOR_DEFAULTTONULL = 0 : renvoie 0 si aucun écran ne contient le point.
    unsafe { MonitorFromPoint(Point { x, y }, 0) != 0 }
}

// ---------------------------------------------------------------------------
// Lecteurs
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Drive {
    /// Racine : « C:\ ».
    pub root: String,
    pub label: String,
    pub total: u64,
    pub free: u64,
}

impl Drive {
    /// « Windows (C:) ».
    pub fn title(&self) -> String {
        format!("{} ({})", self.label, self.root.trim_end_matches('\\'))
    }
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }
}

/// Lecteurs prêts (un disque amovible vide ou un lecteur réseau injoignable est ignoré).
/// Peut être lent si un lecteur réseau ne répond pas : à appeler hors du thread d'affichage.
pub fn drives() -> Vec<Drive> {
    const SEM_FAILCRITICALERRORS: u32 = 1;
    let mut out = Vec::new();
    // Pas de boîte « Insérez un disque » pour les lecteurs vides.
    unsafe { SetErrorMode(SEM_FAILCRITICALERRORS) };
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u8 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i) as char);
        let w = wide(&root);
        let kind = unsafe { GetDriveTypeW(w.as_ptr()) };
        if kind < 2 {
            continue;
        }
        let (mut total, mut free, mut free_caller) = (0u64, 0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free_caller, &mut total, &mut free) } == 0 || total == 0 {
            continue;
        }
        let mut name = [0u16; 262];
        let mut fs = [0u16; 262];
        unsafe { GetVolumeInformationW(w.as_ptr(), name.as_mut_ptr(), 261, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), fs.as_mut_ptr(), 261) };
        let mut label = from_wide(&name);
        if label.trim().is_empty() {
            label = match kind {
                2 => "Disque amovible",
                4 => "Lecteur réseau",
                5 => "Disque optique",
                _ => "Disque local",
            }
            .to_string();
        }
        out.push(Drive { root, label, total, free });
    }
    out
}

// ---------------------------------------------------------------------------
// Explorateur, propriétés, corbeille
// ---------------------------------------------------------------------------

/// Ouvre l'Explorateur sur l'élément : sélectionné dans son dossier, ou ouvert si c'est une racine.
pub fn reveal(path: &str, is_root: bool) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new("explorer.exe");
    if is_root {
        cmd.arg(path);
    } else {
        // L'Explorateur attend la virgule collée et le chemin entre guillemets.
        cmd.raw_arg(format!("/select,\"{path}\""));
    }
    cmd.spawn().map(|_| ())
}

/// Ouvre la fenêtre « Propriétés » de Windows pour l'élément.
pub fn properties(path: &str) -> Result<(), String> {
    const SEE_MASK_INVOKEIDLIST: u32 = 0xC;
    const SW_SHOW: i32 = 5;
    let verb = wide("properties");
    let file = wide(path);
    let mut info = ShellExecuteInfoW {
        size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
        mask: SEE_MASK_INVOKEIDLIST,
        hwnd: 0,
        verb: verb.as_ptr(),
        file: file.as_ptr(),
        parameters: std::ptr::null(),
        directory: std::ptr::null(),
        show: SW_SHOW,
        inst_app: 0,
        id_list: std::ptr::null_mut(),
        class: std::ptr::null(),
        key_class: 0,
        hot_key: 0,
        icon: 0,
        process: 0,
    };
    if unsafe { ShellExecuteExW(&mut info) } != 0 {
        Ok(())
    } else {
        Err("Windows n'a pas pu afficher les propriétés.".into())
    }
}

/// Envoie un fichier ou un dossier à la corbeille. Jamais de suppression définitive :
/// si le lecteur n'a pas de corbeille, rien n'est fait.
pub fn trash(path: &str) -> Result<(), String> {
    const FO_DELETE: u32 = 3;
    const FOF_SILENT: u16 = 0x0004;
    const FOF_NOCONFIRMATION: u16 = 0x0010;
    const FOF_ALLOWUNDO: u16 = 0x0040;
    const FOF_NOERRORUI: u16 = 0x0400;
    // Si Windows ne peut que supprimer définitivement (élément trop gros pour la corbeille),
    // il le demande lui-même au lieu de le faire en silence.
    const FOF_WANTNUKEWARNING: u16 = 0x4000;

    if !std::path::Path::new(path).exists() {
        return Err("L'élément n'existe plus sur le disque.".into());
    }
    let root: String = path.chars().take(3).collect();
    if !root.ends_with(":\\") {
        return Err("Pas de corbeille sur un emplacement réseau : rien n'a été supprimé.".into());
    }
    let mut rb = ShQueryRbInfo { size: std::mem::size_of::<ShQueryRbInfo>() as u32, bytes: 0, items: 0 };
    if unsafe { SHQueryRecycleBinW(wide(&root).as_ptr(), &mut rb) } != 0 {
        return Err(format!("Le lecteur {} n'a pas de corbeille : rien n'a été supprimé.", root.trim_end_matches('\\')));
    }
    // Liste de chemins terminée par deux zéros.
    let mut from = wide(path);
    from.push(0);
    let mut op = ShFileOpStructW {
        hwnd: 0,
        func: FO_DELETE,
        from: from.as_ptr(),
        to: std::ptr::null(),
        flags: FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING,
        any_aborted: 0,
        name_mappings: std::ptr::null_mut(),
        progress_title: std::ptr::null(),
    };
    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc != 0 {
        return Err(format!("Windows a refusé l'envoi à la corbeille (code {rc:#x})."));
    }
    if op.any_aborted != 0 || std::path::Path::new(path).exists() {
        return Err("Envoi à la corbeille annulé : l'élément est toujours en place.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structures_windows() {
        // Tailles attendues par l'API en 64 bits : une erreur ici ferait échouer les appels.
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(std::mem::size_of::<ShellExecuteInfoW>(), 112);
            assert_eq!(std::mem::size_of::<ShFileOpStructW>(), 56);
            assert_eq!(std::mem::size_of::<ShQueryRbInfo>(), 24);
        }
    }

    #[test]
    fn lecteurs() {
        let d = drives();
        assert!(!d.is_empty(), "au moins le lecteur système");
        for x in &d {
            assert!(x.root.len() == 3 && x.root.ends_with(":\\"));
            assert!(x.free <= x.total && !x.label.is_empty());
        }
        let c = Drive { root: "C:\\".into(), label: "Windows".into(), total: 100, free: 30 };
        assert_eq!((c.title().as_str(), c.used()), ("Windows (C:)", 70));
    }

    #[test]
    fn corbeille_refuse_ce_qui_n_existe_pas() {
        assert!(trash(r"C:\ce-fichier-n-existe-pas-diskscan.tmp").is_err());
        assert!(trash(r"\\serveur\partage\fichier").is_err());
    }
}
