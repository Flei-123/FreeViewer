//! Ressourcen fuer die Windows-Datei: Symbol und Versionsangaben.
//!
//! Ohne diesen Schritt hat freeviewer.exe kein eigenes Symbol - Explorer,
//! Startmenue und die Windows-Suche zeigen dann das graue Standardbild, egal
//! wie huebsch das Fenster selbst aussieht. Das Fenstersymbol (`app_icon()`)
//! kommt aus derselben Datei, damit beides gleich aussieht.
//!
//! Bewusst OHNE zusaetzliche Kiste (winres/embed-resource): der Windows-SDK
//! bringt `rc.exe` mit, und dessen Ausgabe (.res) nimmt der MSVC-Linker
//! direkt entgegen. Wird `rc.exe` nicht gefunden, laeuft der Build normal
//! weiter - nur eben ohne Symbol.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/freeviewer.ico");
    println!("cargo:rerun-if-changed=assets/icon.png");
    println!("cargo:rerun-if-changed=build.rs");

    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target != "windows" {
        return;
    }
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ico = root.join("assets").join("freeviewer.ico");
    if !ico.exists() {
        println!("cargo:warning=assets/freeviewer.ico fehlt - Datei bekommt kein Symbol");
        return;
    }
    // Zwei Wege zum selben Ziel:
    //  * MSVC (auf Windows gebaut): rc.exe aus dem Windows-SDK -> .res
    //  * mingw (auf Linux quergebaut): windres -> .o
    // Der zweite Weg ist dazugekommen, weil der Server sonst zwar eine
    // lauffaehige .exe baut, aber ohne Symbol - und eine Auslieferung ohne
    // Programmsymbol sieht aus wie ein Bastelstueck.
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let werkzeug = if env == "gnu" {
        find_windres().map(|p| (p, true))
    } else {
        find_rc().map(|p| (p, false))
    };
    let Some((rc, ist_windres)) = werkzeug else {
        println!(
            "cargo:warning={} nicht gefunden - Datei bekommt kein Symbol",
            if env == "gnu" { "windres" } else { "rc.exe (Windows SDK?)" }
        );
        return;
    };

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let rc_file = out.join("freeviewer.rc");
    let res_file = out.join("freeviewer.res");

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut parts: Vec<u16> = version
        .split(['.', '-'])
        .filter_map(|p| p.parse::<u16>().ok())
        .collect();
    while parts.len() < 4 {
        parts.push(0);
    }

    // rc.exe versteht Schraegstriche in Pfaden, das spart das Verdoppeln der
    // Rueckwaertsschraegstriche.
    let ico_path = ico.to_string_lossy().replace('\\', "/");
    let script = format!(
        r#"#define VOS_NT_WINDOWS32 0x00040004L
#define VFT_APP 0x00000001L

1 ICON "{ico}"

1 VERSIONINFO
FILEVERSION {a},{b},{c},{d}
PRODUCTVERSION {a},{b},{c},{d}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040704b0"
        BEGIN
            VALUE "CompanyName", "FleiTec"
            VALUE "FileDescription", "FreeViewer - Fernwartung und Meetings"
            VALUE "FileVersion", "{ver}"
            VALUE "InternalName", "freeviewer"
            VALUE "LegalCopyright", "GPL-3.0"
            VALUE "OriginalFilename", "freeviewer.exe"
            VALUE "ProductName", "FreeViewer"
            VALUE "ProductVersion", "{ver}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x407, 1200
    END
END
"#,
        ico = ico_path,
        a = parts[0],
        b = parts[1],
        c = parts[2],
        d = parts[3],
        ver = version,
    );
    if std::fs::write(&rc_file, script).is_err() {
        println!("cargo:warning=konnte {} nicht schreiben", rc_file.display());
        return;
    }

    // windres schreibt eine COFF-Objektdatei, rc.exe eine .res - beide
    // nimmt der jeweilige Linker wie eine Objektdatei entgegen.
    let ziel = if ist_windres {
        out.join("freeviewer_res.o")
    } else {
        res_file.clone()
    };
    let mut befehl = std::process::Command::new(&rc);
    if ist_windres {
        befehl
            .arg("-I")
            .arg(root.join("assets"))
            .arg("-O")
            .arg("coff")
            .arg("-i")
            .arg(&rc_file)
            .arg("-o")
            .arg(&ziel);
    } else {
        befehl.arg("/nologo").arg("/fo").arg(&ziel).arg(&rc_file);
    }
    match befehl.status() {
        Ok(s) if s.success() && ziel.exists() => {
            println!("cargo:rustc-link-arg-bins={}", ziel.display());
        }
        Ok(s) => println!("cargo:warning=Ressourcenwerkzeug endete mit {} - kein Symbol", s),
        Err(e) => println!("cargo:warning=Ressourcenwerkzeug nicht startbar: {} - kein Symbol", e),
    }
}

/// `windres` fuer den mingw-Querbau suchen: erst FV_WINDRES, dann die
/// ueblichen Namen im PATH.
fn find_windres() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("FV_WINDRES") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let namen = [
        "x86_64-w64-mingw32-windres",
        "windres",
        "llvm-windres",
    ];
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            for n in namen {
                let cand = dir.join(n);
                if cand.exists() {
                    return Some(cand);
                }
            }
        }
    }
    None
}

/// Sucht `rc.exe`: erst die Umgebungsvariable FV_RC, dann PATH, dann die
/// neueste x64-Fassung im Windows Kit.
fn find_rc() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("FV_RC") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let cand = dir.join("rc.exe");
            if cand.exists() {
                return Some(cand);
            }
        }
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    for key in ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"] {
        if let Ok(v) = std::env::var(key) {
            roots.push(Path::new(&v).join("Windows Kits").join("10").join("bin"));
        }
    }
    let mut best: Option<(String, PathBuf)> = None;
    for root in roots {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("10.") {
                continue;
            }
            let cand = e.path().join("x64").join("rc.exe");
            if cand.exists() && best.as_ref().map(|(v, _)| ver_key(&name) > ver_key(v)).unwrap_or(true)
            {
                best = Some((name, cand));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// "10.0.22621.0" -> vergleichbare Zahlenliste.
fn ver_key(s: &str) -> Vec<u32> {
    s.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect()
}
