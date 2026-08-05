//! Richtige Installation - so, wie Windows ein Programm erwartet.
//!
//! FreeViewer ist eine einzige Datei und laeuft ueberall, auch vom USB-Stick.
//! Wer ihn dauerhaft auf seinem Rechner haben will, klickt in den
//! Einstellungen auf "Installieren" (AnyDesk macht es genauso). Dann passiert
//! das, was ein Setup normalerweise tut:
//!
//! * Datei nach `C:\Program Files\FreeViewer\freeviewer.exe`
//! * Eintrag im Startmenue - dadurch findet die Windows-Suche "FreeViewer"
//!   als Programm und nicht mehr als herumliegende .exe
//! * Verknuepfung auf dem oeffentlichen Desktop
//! * Eintrag in "Apps & Features" mit Deinstallieren-Knopf
//! * auf Wunsch der Dienst, damit der Rechner auch vor der Anmeldung
//!   erreichbar ist
//!
//! Deinstallieren raeumt alles davon wieder weg. Die Konfiguration
//! (ID, Adressbuch, Passwoerter) bleibt absichtlich liegen - wer neu
//! installiert, will dieselbe ID behalten.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

pub use crate::brand::NAME as APP;
pub use crate::brand::PUBLISHER;
#[cfg(windows)]
fn uninstall_key() -> String {
    format!(
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{}",
        crate::brand::DIR
    )
}

/// `C:\Program Files\FreeViewer`
pub fn install_dir() -> PathBuf {
    let pf = std::env::var("ProgramW6432")
        .or_else(|_| std::env::var("ProgramFiles"))
        .unwrap_or_else(|_| r"C:\Program Files".to_string());
    PathBuf::from(pf).join(crate::brand::DIR)
}

pub fn installed_exe() -> PathBuf {
    install_dir().join(crate::brand::EXE)
}

/// Liegt dort schon eine Installation?
pub fn is_installed() -> bool {
    installed_exe().exists()
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase().replace('/', "\\")
        == b.to_string_lossy().to_lowercase().replace('/', "\\")
}

/// Laeuft genau diese Exe aus dem Installationsordner?
pub fn running_installed() -> bool {
    match std::env::current_exe() {
        Ok(cur) => same_path(&cur, &installed_exe()),
        Err(_) => false,
    }
}

fn start_menu_link() -> PathBuf {
    let pd = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    PathBuf::from(pd)
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join(format!("{}.lnk", APP))
}

fn desktop_link() -> PathBuf {
    let pub_dir = std::env::var("PUBLIC").unwrap_or_else(|_| r"C:\Users\Public".to_string());
    PathBuf::from(pub_dir)
        .join("Desktop")
        .join(format!("{}.lnk", APP))
}

// --------------------------------------------------------------- Windows

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS};
    use winreg::RegKey;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Legt eine .lnk an (das, was ein Setup normalerweise macht).
    pub fn shortcut(link: &Path, target: &Path, desc: &str) -> Result<()> {
        use windows::core::{Interface, PCWSTR};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

        if let Some(dir) = link.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let t = wide(&target.to_string_lossy());
            sl.SetPath(PCWSTR(t.as_ptr()))?;
            if let Some(dir) = target.parent() {
                let d = wide(&dir.to_string_lossy());
                sl.SetWorkingDirectory(PCWSTR(d.as_ptr()))?;
            }
            let de = wide(desc);
            sl.SetDescription(PCWSTR(de.as_ptr()))?;
            let ic = wide(&target.to_string_lossy());
            sl.SetIconLocation(PCWSTR(ic.as_ptr()), 0)?;
            let pf: IPersistFile = sl.cast()?;
            let l = wide(&link.to_string_lossy());
            pf.Save(PCWSTR(l.as_ptr()), true)?;
        }
        Ok(())
    }

    /// Eintrag in "Apps & Features".
    fn write_uninstall_entry(exe: &Path, size_kb: u32) -> Result<()> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let (key, _) = hklm.create_subkey(uninstall_key())?;
        key.set_value("DisplayName", &format!("{} Fernwartung", APP))?;
        key.set_value("DisplayVersion", &crate::update::VERSION.to_string())?;
        key.set_value("Publisher", &PUBLISHER.to_string())?;
        key.set_value("DisplayIcon", &exe.to_string_lossy().to_string())?;
        key.set_value(
            "InstallLocation",
            &install_dir().to_string_lossy().to_string(),
        )?;
        key.set_value(
            "UninstallString",
            &format!("\"{}\" --uninstall", exe.display()),
        )?;
        key.set_value(
            "QuietUninstallString",
            &format!("\"{}\" --uninstall --quiet", exe.display()),
        )?;
        key.set_value("NoModify", &1u32)?;
        key.set_value("NoRepair", &1u32)?;
        key.set_value("EstimatedSize", &size_kb)?;
        key.set_value("URLInfoAbout", &"https://freeviewer.fleitec.com".to_string())?;
        Ok(())
    }

    fn remove_uninstall_entry() {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        if let Ok(key) = hklm.open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
            KEY_ALL_ACCESS,
        ) {
            let _ = key.delete_subkey_all(APP);
        }
    }

    /// Kopiert die laufende Datei an ihren Platz und traegt alles ein.
    /// Braucht Administrator-Rechte (der Aufrufer holt sie per UAC).
    pub fn install(with_service: bool) -> Result<()> {
        let src = std::env::current_exe()?;
        let dst = installed_exe();
        std::fs::create_dir_all(install_dir())?;
        if !same_path(&src, &dst) {
            // Der Dienst haelt die alte Datei fest - erst anhalten, dann tauschen.
            let dienst_lief = std::process::Command::new("sc")
                .args(["query", crate::service::SERVICE_NAME])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains("RUNNING"))
                .unwrap_or(false);
            if dienst_lief {
                let _ = std::process::Command::new("sc")
                    .args(["stop", crate::service::SERVICE_NAME])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(1200));
            }
            // Eine laufende Exe kann man nicht ueberschreiben, aber umbenennen.
            // Klemmt auch das (weil noch ein alter Stand aus der .old-Datei
            // laeuft), nehmen wir den naechsten freien Namen.
            if dst.exists() {
                let mut weg = false;
                for i in 0..12 {
                    let old = if i == 0 {
                        dst.with_extension("old")
                    } else {
                        dst.with_extension(format!("old{}", i))
                    };
                    let _ = std::fs::remove_file(&old);
                    if std::fs::rename(&dst, &old).is_ok() {
                        weg = true;
                        break;
                    }
                }
                if !weg {
                    return Err(anyhow!(
                        "{} liess sich nicht beiseite legen - laeuft dort noch etwas?",
                        dst.display()
                    ));
                }
            }
            std::fs::copy(&src, &dst)
                .map_err(|e| anyhow!("Kopieren nach {} fehlgeschlagen: {}", dst.display(), e))?;
            if dienst_lief {
                let _ = std::process::Command::new("sc")
                    .args(["start", crate::service::SERVICE_NAME])
                    .output();
            }
        }
        // Reste frueherer Installationen wegraeumen, soweit sie freigegeben sind
        if let Ok(list) = std::fs::read_dir(install_dir()) {
            for e in list.flatten() {
                let name = e.file_name().to_string_lossy().to_lowercase();
                let old_prefix = crate::brand::EXE.replace(".exe", ".old").to_lowercase();
                if name.starts_with(&old_prefix) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let size_kb = std::fs::metadata(&dst).map(|m| (m.len() / 1024) as u32).unwrap_or(0);

        shortcut(
            &start_menu_link(),
            &dst,
            &format!("{} - Fernwartung ohne Konto", crate::brand::NAME),
        )?;
        // Der Desktop-Eintrag ist Kuer, kein Grund zum Abbrechen.
        let _ = shortcut(&desktop_link(), &dst, crate::brand::NAME);
        write_uninstall_entry(&dst, size_kb)?;
        // freeviewer://-Adressen sollen fuer alle Nutzer dieses Rechners gehen
        let _ = crate::link::register_for(&dst, true);

        if with_service {
            let _ = std::process::Command::new(&dst)
                .arg("--install-service")
                .status();
        }
        Ok(())
    }

    /// Raeumt alles wieder weg. Die Konfiguration bleibt liegen.
    pub fn uninstall(alles: bool) -> Result<()> {
        let exe = installed_exe();
        // Dienst zuerst - sonst haelt er die Datei fest.
        let _ = std::process::Command::new("sc")
            .args(["stop", crate::service::SERVICE_NAME])
            .output();
        let _ = std::process::Command::new("sc")
            .args(["delete", crate::service::SERVICE_NAME])
            .output();
        let _ = crate::autostart::set(false);
        let _ = std::fs::remove_file(start_menu_link());
        let _ = std::fs::remove_file(desktop_link());
        remove_uninstall_entry();
        // freeviewer://-Registrierung wegraeumen (HKLM von der Installation,
        // HKCU vom portablen Lauf) - sonst starten Links ins Leere.
        for root in [
            RegKey::predef(HKEY_LOCAL_MACHINE),
            RegKey::predef(HKEY_CURRENT_USER),
        ] {
            if let Ok(k) = root.open_subkey_with_flags(r"Software\Classes", KEY_ALL_ACCESS) {
                // Beide Schemata: das eigene der Marke und - falls dieser
                // Build es uebernommen hatte - das alte gemeinsame.
                for schema in crate::link::schemes() {
                    let _ = k.delete_subkey_all(schema);
                }
            }
        }

        if alles {
            // wirklich alles: Konfiguration (beide Orte) und die
            // Identitaets-Sicherung in der Registry. Geraete mit abgeleiteter
            // Identitaet bekommen ihre ID bei der Neuinstallation trotzdem
            // wieder - sie kommt aus der Maschinen-Kennung.
            let _ = std::fs::remove_dir_all(crate::ident::real_config_dir());
            if let Some(m) = crate::ident::machine_config_dir() {
                let _ = std::fs::remove_dir_all(m);
            }
            crate::ident::winid::drop_backup();
        }

        // Laufende Prozesse beenden, dann Ordner loeschen. Eine Exe kann sich
        // nicht selbst loeschen, darum macht das ein kurzer cmd-Aufruf,
        // nachdem wir weg sind.
        let dir = install_dir();
        let script = format!(
            "ping -n 3 127.0.0.1 >nul & del /f /q \"{}\" >nul 2>&1 & del /f /q \"{}\" >nul 2>&1 & rmdir \"{}\" >nul 2>&1",
            exe.display(),
            exe.with_extension("old").display(),
            dir.display()
        );
        let _ = std::process::Command::new("cmd")
            .args(["/c", &script])
            .spawn();
        Ok(())
    }
}

#[cfg(windows)]
pub use imp::{install, shortcut};

/// Deinstallieren: Programm, Verknuepfungen, Dienst und Protokoll weg -
/// die Konfiguration bleibt (die ID soll eine Neuinstallation ueberleben).
#[cfg(windows)]
pub fn uninstall() -> Result<()> {
    imp::uninstall(false)
}

/// Vollstaendig entfernen - inklusive Konfiguration und Identitaets-Sicherung.
#[cfg(windows)]
pub fn uninstall_all() -> Result<()> {
    imp::uninstall(true)
}

#[cfg(not(windows))]
pub fn install(_with_service: bool) -> Result<()> {
    Err(anyhow!("nur unter Windows"))
}
#[cfg(not(windows))]
pub fn uninstall() -> Result<()> {
    Err(anyhow!("nur unter Windows"))
}

/// Vollstaendig entfernen - inklusive Konfiguration und Identitaets-Sicherung.
#[cfg(not(windows))]
pub fn uninstall_all() -> Result<()> {
    uninstall()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_dir_ends_with_the_app_name() {
        let d = install_dir();
        assert!(d.ends_with(crate::brand::DIR), "{}", d.display());
        assert!(installed_exe().ends_with(crate::brand::EXE));
    }

    #[test]
    fn paths_compare_without_case_and_slashes() {
        assert!(same_path(
            Path::new(r"C:\Program Files\FreeViewer\freeviewer.exe"),
            Path::new("c:/program files/freeviewer/FreeViewer.EXE")
        ));
        assert!(!same_path(
            Path::new(r"C:\a\freeviewer.exe"),
            Path::new(r"C:\b\freeviewer.exe")
        ));
    }

    #[test]
    fn shortcut_targets_live_next_to_the_program() {
        // Startmenue-Eintrag ist maschinenweit, nicht im Profil eines Nutzers
        let l = start_menu_link().to_string_lossy().to_lowercase();
        assert!(l.contains("start menu"), "{}", l);
        let lnk = crate::brand::EXE.replace(".exe", ".lnk").to_lowercase();
        assert!(l.ends_with(&lnk), "{} endet nicht auf {}", l, lnk);
    }
}
