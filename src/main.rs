#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! FreeViewer - open source remote desktop.
//!
//! One binary does both jobs, like TeamViewer: it registers this machine at the
//! relay (so others can connect to your ID) and it can connect to another ID.
//!
//! Relay can be overridden with the FV_RELAY environment variable,
//! the session password with FV_PASSWORD.

mod audio;
mod brand;
mod autostart;
mod capture;
mod chrome;
mod clip;
mod crypto;
mod encoder;
mod feedback;
mod h264;
mod hostside;
mod i18n;
mod account;
mod icons;
mod ident;
#[cfg(feature = "license")]
mod license;
mod link;
mod meet;
mod meetsig;
mod meetrtc;
mod meetaudio;
mod meetui;
mod meetfenster;
mod meetvideo;
mod meetcam;
mod meetschirm;
mod input;
mod net;
mod p2p;
mod res;
mod partners;
mod presence;
mod pwlist;
mod proto;
mod selftest;
mod service;
mod setup;
mod shared;
mod theme;
mod tray;
mod update;
mod viewer;
mod vinput;
mod xfer;

use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use proto::Msg;
use shared::Shared;

pub const DEFAULT_RELAY: &str = "wss://freeviewer.fleitec.com/fv/ws";

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Diagnostic line into <config>/debug.log. The release build is a GUI binary,
/// so println! is invisible unless somebody redirected stdout.
pub fn dbg_line(s: &str) {
    use std::io::Write;
    println!("{}", s);
    let path = ident::config_dir().join("debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", s);
    }
}

fn rt() -> &'static tokio::runtime::Runtime {
    RT.get().expect("tokio runtime")
}

/// Capture and input work in physical pixels, so the process must not be
/// scaled by Windows.
#[cfg(windows)]
fn make_dpi_aware() {
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}
#[cfg(not(windows))]
fn make_dpi_aware() {}

fn main() -> eframe::Result<()> {
    make_dpi_aware();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let relay = std::env::var("FV_RELAY").unwrap_or_else(|_| DEFAULT_RELAY.to_string());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _ = RT.set(runtime);

    let secret = ident::load_or_create_secret();
    let password = std::env::var("FV_PASSWORD")
        .ok()
        .or_else(ident::fixed_password)
        .unwrap_or_else(ident::random_password);
    let shared = Arc::new(Shared::new(relay, password));

    // Tausch mit Administrator-Rechten (vom Updater gestartet):
    //   freeviewer --apply-update <frisch> <ziel> <pid>
    if let Some(i) = std::env::args().position(|a| a == "--apply-update") {
        let args: Vec<String> = std::env::args().collect();
        let src = args.get(i + 1).cloned().unwrap_or_default();
        let dst = args.get(i + 2).cloned().unwrap_or_default();
        let pid: u32 = args
            .get(i + 3)
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        match update::apply_update(
            std::path::Path::new(&src),
            std::path::Path::new(&dst),
            pid,
        ) {
            Ok(()) => println!("Update eingespielt: {}", dst),
            Err(e) => {
                eprintln!("Update fehlgeschlagen: {}", e);
                std::thread::sleep(std::time::Duration::from_secs(4));
            }
        }
        return Ok(());
    }

    // Installieren / Deinstallieren (kommt vom Knopf in den Einstellungen,
    // der sich vorher per UAC Administrator-Rechte holt).
    if std::env::args().any(|a| a == "--install") {
        let with_service = std::env::args().any(|a| a == "--with-service");
        match setup::install(with_service) {
            Ok(()) => {
                println!("installiert nach {}", setup::install_dir().display());
                // die installierte Fassung als normaler Nutzer starten
                let _ = std::process::Command::new("explorer.exe")
                    .arg(setup::installed_exe().as_os_str())
                    .spawn();
                // die Quelle (z. B. Downloads\FreeViewer-Einrichtung.exe) wird
                // nicht mehr gebraucht - kurz nach dem Beenden loeschen
                if let Ok(cur) = std::env::current_exe() {
                    if cur != setup::installed_exe() {
                        let _ = std::process::Command::new("cmd")
                            .args([
                                "/c",
                                &format!(
                                    "ping -n 6 127.0.0.1 >nul & del /f /q \"{}\"",
                                    cur.display()
                                ),
                            ])
                            .spawn();
                    }
                }
            }
            Err(e) => {
                eprintln!("Installation fehlgeschlagen: {}", e);
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
        return Ok(());
    }
    if std::env::args().any(|a| a == "--uninstall") {
        // --all nimmt auch Konfiguration und Identitaets-Sicherung mit
        let alles = std::env::args().any(|a| a == "--all");
        let r = if alles {
            setup::uninstall_all()
        } else {
            setup::uninstall()
        };
        match r {
            Ok(()) => println!("deinstalliert"),
            Err(e) => {
                eprintln!("Deinstallation fehlgeschlagen: {}", e);
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
        return Ok(());
    }

    // print the version:  freeviewer --version
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("{} {}", crate::brand::NAME, update::VERSION);
        return Ok(());
    }

    // Which sound devices would a session use?  freeviewer --audiodev
    if std::env::args().any(|a| a == "--audiodev") {
        print!("{}", audio::list_devices());
        return Ok(());
    }
    // Microphone -> codec -> speaker on this machine:  freeviewer --audioloop 5
    if let Some(i) = std::env::args().position(|a| a == "--audioloop") {
        let secs = std::env::args()
            .nth(i + 1)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(5);
        println!("{}", audio::loopback_test(secs));
        return Ok(());
    }
    // Draws every page once without a window:  freeviewer --uitest
    // egui can run headless, so a broken layout or a panic in the GUI shows
    // up in a build step instead of in front of the user.
    // Kameraliste (Stufe 2d): welche Kameras sieht der Rechner?
    //   freeviewer --kameraliste
    // KAMERA-DIAGNOSE: was bietet das Geraet an, welches Format bekommen
    // wir wirklich, wie sehen Roh- und Endbild aus. Nur so laesst sich ein
    // Farb- oder Zuschnittfehler BELEGEN statt zu raten.
    //   freeviewer --camtest [breite hoehe fps] [ordner]
    if let Some(i) = std::env::args().position(|a| a == "--camtest") {
        let args: Vec<String> = std::env::args().collect();
        let zahl = |n: usize, vor: u32| -> u32 {
            args.get(n).and_then(|v| v.parse::<u32>().ok()).unwrap_or(vor)
        };
        let (w, h, fps) = (zahl(i + 1, 640), zahl(i + 2, 360), zahl(i + 3, 15));
        let ordner = args
            .get(i + 4)
            .cloned()
            .unwrap_or_else(|| ".".to_string());
        println!("{}", meetcam::diagnose(None, w, h, fps, &ordner));
        return Ok(());
    }

    if std::env::args().any(|a| a == "--kameraliste") {
        let l = meetcam::liste();
        println!("KAMERAS {}", l.len());
        for (i, k) in l.iter().enumerate() {
            println!("KAMERA {} name={} id={}", i, k.name, k.id);
        }
        return Ok(());
    }

    // GENERALPROBE (vor dem Release): der Client macht ALLES gleichzeitig -
    // Ton, Kamera, Bildschirm, Fernsteuerungs-Freigabe - und zwar ueber
    // GENAU die Klasse, die auch das Fenster benutzt (NativMeet). So wird
    // gemessen, was die App wirklich tut, nicht ein Sonderweg daneben.
    //   freeviewer --meetalles <raum> <passwort> [name] [sekunden]
    if let Some(i) = std::env::args().position(|a| a == "--meetalles") {
        let args: Vec<String> = std::env::args().collect();
        let raum = args.get(i + 1).cloned().unwrap_or_default();
        let pass = args.get(i + 2).cloned().unwrap_or_default();
        let name = args
            .get(i + 3)
            .cloned()
            .unwrap_or_else(|| "NativAlles".to_string());
        let dauer: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(35);
        let fvid = args
            .get(i + 5)
            .cloned()
            .unwrap_or_else(|| "497628420".to_string());
        let mut m = match meetui::NativMeet::beitreten(
            &meet::base(),
            &raum,
            &pass,
            &name,
            &fvid,
            false,
            None,
            None,
        ) {
            Ok(m) => m,
            Err(e) => {
                println!("ALLES FEHLER Beitritt: {}", e);
                return Ok(());
            }
        };
        println!("ALLES START {}", m.meldung);
        let start = std::time::Instant::now();
        let (mut kamera_an, mut schirm_an, mut frei) = (false, false, false);
        while start.elapsed().as_secs() < dauer {
            m.pumpe();
            // Der Reihe nach zuschalten, damit sich im Browser sehen laesst,
            // WAS gerade dazukommt.
            let s = start.elapsed().as_secs();
            if !kamera_an && s >= 3 {
                kamera_an = true;
                m.kamera_schalten(true);
                println!("ALLES KAMERA {} ({})", m.kamera_an, m.kamera_meldung);
            }
            if !schirm_an && s >= 8 {
                schirm_an = true;
                m.schirm_schalten(true, 0);
                println!("ALLES SCHIRM {} ({})", m.schirm_an, m.schirm_meldung);
            }
            if !frei && s >= 12 {
                frei = true;
                m.steuerung_freigeben(true);
                println!("ALLES STEUERUNG {}", m.steuer_frei);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let z = m.zustand();
        let zahlen = m.zahlen();
        println!(
            "ALLES ERGEBNIS leute={} ton_gesendet={} ton_empfangen={} kamera_gesendet={} schirm_gesendet={} bild_empfangen={} codec={}",
            z.leute.len(),
            zahlen.gesendet,
            zahlen.empfangen,
            m.bild_gesendet,
            m.schirm_gesendet,
            m.bild_zaehler,
            m.bild_codec
        );
        println!(
            "ALLES ZUSTAND kamera={} schirm={} steuerung={} meldung={} kamera_meldung={} schirm_meldung={}",
            m.kamera_an, m.schirm_an, m.steuer_frei, m.meldung, m.kamera_meldung, m.schirm_meldung
        );
        m.verlassen();
        std::thread::sleep(std::time::Duration::from_millis(600));
        println!("ALLES FERTIG");
        return Ok(());
    }

    // Steuertest (Stufe 4): gibt der native Client die Fernsteuerung frei,
    // und meldet er, wenn ein anderer sie freigibt? Der Browser bekommt
    // dadurch seinen "Steuern"-Knopf - und wir hier die Gegenprobe.
    //   freeviewer --meetsteuer <raum> <passwort> [name] [sekunden] [fvid]
    if let Some(i) = std::env::args().position(|a| a == "--meetsteuer") {
        let args: Vec<String> = std::env::args().collect();
        let raum = args.get(i + 1).cloned().unwrap_or_default();
        let pass = args.get(i + 2).cloned().unwrap_or_default();
        let name = args
            .get(i + 3)
            .cloned()
            .unwrap_or_else(|| "NativSteuer".to_string());
        let dauer: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(25);
        let fvid = args
            .get(i + 5)
            .cloned()
            .unwrap_or_else(|| "497628420".to_string());
        // Absichtlich OHNE Nummer beitreten: so laesst sich messen, dass der
        // Knopf beim anderen erst durch die Freigabe entsteht.
        let sig = match meetsig::beitreten(&meet::base(), &raum, &pass, &name, "") {
            Ok(s) => s,
            Err(e) => {
                println!("STEUERTEST FEHLER Signalisierung: {}", e);
                return Ok(());
            }
        };
        let start = std::time::Instant::now();
        let mut freigegeben = false;
        let mut zurueck = false;
        let mut angebote = 0u32;
        while start.elapsed().as_secs() < dauer {
            for e in sig.abholen() {
                match &e {
                    meetsig::Ereignis::Fernsteuerung { peer, fvid } => {
                        angebote += 1;
                        println!(
                            "STEUERTEST ANGEBOT peer={} fvid={}",
                            peer,
                            if fvid.is_empty() { "-" } else { fvid.as_str() }
                        );
                    }
                    // Im Test sofort einlassen - sonst steht der Browser
                    // vor der Tuer und nichts laesst sich messen.
                    meetsig::Ereignis::WarteDazu { peer, .. } => {
                        sig.warteraum("admit", Some(*peer))
                    }
                    meetsig::Ereignis::Dazu(t) => println!("STEUERTEST DAZU {}", t.name),
                    _ => {}
                }
            }
            if !freigegeben && sig.zustand().ich != 0 && start.elapsed().as_secs() >= 3 {
                freigegeben = true;
                sig.fernsteuerung(true, &fvid);
                println!("STEUERTEST FREIGEGEBEN {}", fvid);
            }
            // Mitten im Lauf wieder zuruecknehmen: nur so laesst sich zeigen,
            // dass der Knopf beim anderen auch WIEDER verschwindet - waeren
            // wir dafuer rausgegangen, waere es kein Beweis, sondern nur das
            // uebliche Aufraeumen.
            if freigegeben
                && !zurueck
                && start.elapsed().as_secs() >= dauer.saturating_sub(8).max(9)
            {
                zurueck = true;
                sig.fernsteuerung(false, "");
                println!("STEUERTEST ZURUECKGENOMMEN");
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let z = sig.zustand();
        for t in z.leute.iter() {
            if !t.fvid.is_empty() {
                println!("STEUERTEST STEUERBAR {} {}", t.name, t.fvid);
            }
        }
        println!(
            "STEUERTEST ZUSTAND ich={} leute={} angebote={}",
            z.ich,
            z.leute.len(),
            angebote
        );
        if !zurueck {
            sig.fernsteuerung(false, "");
            std::thread::sleep(std::time::Duration::from_millis(800));
        }
        sig.verlassen();
        std::thread::sleep(std::time::Duration::from_millis(500));
        println!("STEUERTEST FERTIG");
        return Ok(());
    }

    // Empfangstest (Stufe 3b): was kommt beim nativen Client AN - und wird
    // Kamera sauber von Bildschirm getrennt?
    //   freeviewer --meetempfang <raum> <passwort> [name] [sekunden]
    if let Some(i) = std::env::args().position(|a| a == "--meetempfang") {
        let args: Vec<String> = std::env::args().collect();
        let raum = args.get(i + 1).cloned().unwrap_or_default();
        let pass = args.get(i + 2).cloned().unwrap_or_default();
        let name = args
            .get(i + 3)
            .cloned()
            .unwrap_or_else(|| "NativAuge".to_string());
        let dauer: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(30);
        let sig = match meetsig::beitreten(&meet::base(), &raum, &pass, &name, "") {
            Ok(s) => s,
            Err(e) => {
                println!("EMPFANGSTEST FEHLER Signalisierung: {}", e);
                return Ok(());
            }
        };
        let ton = match meetrtc::starten() {
            Ok(t) => t,
            Err(e) => {
                println!("EMPFANGSTEST FEHLER RTC: {}", e);
                return Ok(());
            }
        };
        let mut kameras = meetvideo::Dekodierer::neu();
        let mut schirme = meetvideo::Dekodierer::neu();
        let mut angeboten = false;
        let (mut kam_rahmen, mut schirm_rahmen) = (0u64, 0u64);
        let start = std::time::Instant::now();
        while start.elapsed().as_secs() < dauer {
            for e in sig.abholen() {
                match &e {
                    meetsig::Ereignis::Willkommen { .. } => {
                        if !angeboten {
                            angeboten = true;
                            sig.roh(serde_json::json!({"t":"offer","sdp":ton.angebot}));
                        }
                    }
                    meetsig::Ereignis::WarteDazu { peer, .. } => sig.warteraum("admit", Some(*peer)),
                    meetsig::Ereignis::Spur {
                        mid,
                        peer,
                        bildschirm,
                        art,
                    } => {
                        ton.spur_art(mid, *peer, *bildschirm);
                        println!("SPUR mid={} peer={} art={} bildschirm={}", mid, peer, art, bildschirm);
                    }
                    meetsig::Ereignis::Sdp { art, sdp } if art == "answer" => {
                        ton.antwort(sdp);
                        sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                        sig.roh(serde_json::json!({"t":"publish","mid":ton.vid,"screen":false}));
                        println!("EREIGNIS Antwort eingespielt");
                    }
                    meetsig::Ereignis::Sdp { art, sdp } if art == "offer" => ton.server_angebot(sdp),
                    meetsig::Ereignis::Getrennt(m) => println!("EREIGNIS Getrennt: {}", m),
                    _ => {}
                }
            }
            if let Some(a) = ton.offene_antwort() {
                sig.roh(serde_json::json!({"t":"answer","sdp":a}));
            }
            for te in ton.abholen() {
                if let meetrtc::TonEreignis::Bild {
                    quelle,
                    bildschirm,
                    daten,
                    ..
                } = te
                {
                    if bildschirm {
                        schirm_rahmen += 1;
                        schirme.rahmen(quelle, &daten);
                    } else {
                        kam_rahmen += 1;
                        kameras.rahmen(quelle, &daten);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let masse = |d: &meetvideo::Dekodierer| -> String {
            d.bilder
                .iter()
                .map(|(peer, (w, h, _))| {
                    format!("peer{}={}x{} hell={:.2}", peer, w, h, d.helligkeit(*peer))
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!(
            "ZAHLEN kamera_rahmen={} kamera_bilder={} [{}] schirm_rahmen={} schirm_bilder={} [{}] fehler={}",
            kam_rahmen,
            kameras.gezaehlt,
            masse(&kameras),
            schirm_rahmen,
            schirme.gezaehlt,
            masse(&schirme),
            if schirme.letzter_fehler.is_empty() { kameras.letzter_fehler.clone() } else { schirme.letzter_fehler.clone() }
        );
        ton.beenden();
        sig.verlassen();
        std::thread::sleep(std::time::Duration::from_millis(300));
        println!("EMPFANGSTEST FERTIG");
        return Ok(());
    }

    // Bildschirmtest (Stufe 3): den ECHTEN Bildschirm als eigene Spur ins
    // Meeting schicken - ohne Browser.
    //   freeviewer --meetschirm <raum> <passwort> [name] [sekunden] [schirm-nr]
    if let Some(i) = std::env::args().position(|a| a == "--meetschirm") {
        let args: Vec<String> = std::env::args().collect();
        let raum = args.get(i + 1).cloned().unwrap_or_default();
        let pass = args.get(i + 2).cloned().unwrap_or_default();
        let name = args
            .get(i + 3)
            .cloned()
            .unwrap_or_else(|| "NativSchirm".to_string());
        let dauer: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(25);
        let nr: usize = args.get(i + 5).and_then(|s| s.parse().ok()).unwrap_or(0);
        for (n, m) in meetschirm::liste().iter().enumerate() {
            println!(
                "SCHIRM {} name={} {}x{} primaer={}",
                n, m.name, m.breite, m.hoehe, m.primaer
            );
        }
        let auf = match meetschirm::oeffnen(nr, 1920, 1080, 15) {
            Ok(a) => a,
            Err(e) => {
                println!("SCHIRMTEST FEHLER Aufnahme: {}", e);
                return Ok(());
            }
        };
        println!("SCHIRM OFFEN {} {}x{}", auf.name, auf.breite, auf.hoehe);
        let mut koder = match meetvideo::Kodierer::neu(auf.breite, auf.hoehe, 15, 3_500_000) {
            Ok(k) => k,
            Err(e) => {
                println!("SCHIRMTEST FEHLER Kodierer: {}", e);
                return Ok(());
            }
        };
        let sig = match meetsig::beitreten(&meet::base(), &raum, &pass, &name, "") {
            Ok(s) => s,
            Err(e) => {
                println!("SCHIRMTEST FEHLER Signalisierung: {}", e);
                return Ok(());
            }
        };
        let ton = match meetrtc::starten() {
            Ok(t) => t,
            Err(e) => {
                println!("SCHIRMTEST FEHLER RTC: {}", e);
                return Ok(());
            }
        };
        let mut angeboten = false;
        let mut pakete: u64 = 0;
        let mut gesendet: u64 = 0;
        let mut hell_summe = 0f64;
        let mut bewegung = 0f64;
        let mut vorbild: Vec<u8> = Vec::new();
        let start = std::time::Instant::now();
        let mut naechstes = std::time::Instant::now();
        let mut schluessel = std::time::Instant::now();
        while start.elapsed().as_secs() < dauer {
            for e in sig.abholen() {
                match &e {
                    meetsig::Ereignis::Willkommen { .. } => {
                        if !angeboten {
                            angeboten = true;
                            sig.roh(serde_json::json!({"t":"offer","sdp":ton.angebot}));
                        }
                    }
                    meetsig::Ereignis::WarteDazu { peer, .. } => sig.warteraum("admit", Some(*peer)),
                    meetsig::Ereignis::Spur { mid, peer, .. } => ton.spur(mid, *peer),
                    meetsig::Ereignis::Sdp { art, sdp } if art == "answer" => {
                        ton.antwort(sdp);
                        sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                        sig.roh(serde_json::json!({"t":"publish","mid":ton.vid,"screen":false}));
                        sig.roh(serde_json::json!({"t":"publish","mid":ton.vid2,"screen":true}));
                        sig.roh(serde_json::json!({"t":"screen","on":true}));
                        println!("EREIGNIS Antwort eingespielt, Bildschirm angemeldet");
                    }
                    meetsig::Ereignis::Sdp { art, sdp } if art == "offer" => ton.server_angebot(sdp),
                    // Fehler NICHT verschlucken - sonst sieht man am Ende nur
                    // "nichts gesendet" und raet, woran es lag.
                    meetsig::Ereignis::Getrennt(m) => println!("EREIGNIS Getrennt: {}", m),
                    meetsig::Ereignis::Fehler { code, text } => {
                        println!("EREIGNIS Fehler {}: {}", code, text)
                    }
                    meetsig::Ereignis::Abgewiesen(m) => println!("EREIGNIS Abgewiesen: {}", m),
                    _ => {}
                }
            }
            if let Some(a) = ton.offene_antwort() {
                sig.roh(serde_json::json!({"t":"answer","sdp":a}));
            }
            for te in ton.abholen() {
                match te {
                    meetrtc::TonEreignis::Verbunden => println!("EREIGNIS RTC verbunden"),
                    meetrtc::TonEreignis::Fehler(f) => println!("EREIGNIS RTC-Fehler: {}", f),
                    _ => {}
                }
            }
            if std::time::Instant::now() >= naechstes {
                naechstes += std::time::Duration::from_millis(66);
                if let Some(b) = auf.neuestes() {
                    let y = &b.nv12[..(b.breite * b.hoehe) as usize];
                    hell_summe += y.iter().map(|v| *v as f64).sum::<f64>() / y.len() as f64;
                    if vorbild.len() == y.len() {
                        let d: f64 = y
                            .iter()
                            .zip(vorbild.iter())
                            .map(|(a, c)| (*a as i32 - *c as i32).unsigned_abs() as f64)
                            .sum::<f64>()
                            / y.len() as f64;
                        bewegung += d;
                    }
                    vorbild = y.to_vec();
                    if schluessel.elapsed() >= std::time::Duration::from_secs(4) {
                        schluessel = std::time::Instant::now();
                        koder.schluesselbild();
                    }
                    gesendet += 1;
                    match koder.nv12_rahmen(&b.nv12) {
                        Ok(teile) => {
                            for t in teile {
                                pakete += 1;
                                ton.schirm_senden(t.data);
                            }
                        }
                        Err(e) => println!("KODIER-FEHLER {}", e),
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let z = ton.zahlen();
        println!(
            "ZAHLEN schirm={} groesse={}x{} aufgenommen={} verwendet={} kodiert={} pakete={} schirm_gesendet={} bild_empfangen={} bytes_raus={} helligkeit={:.1} bewegung={:.2} fehler={}",
            auf.name,
            auf.breite,
            auf.hoehe,
            auf.aufgenommen(),
            gesendet,
            koder.bilder,
            pakete,
            z.schirm_gesendet,
            z.bild_empfangen,
            z.bytes_raus,
            if gesendet > 0 { hell_summe / gesendet as f64 } else { 0.0 },
            if gesendet > 1 { bewegung / (gesendet - 1) as f64 } else { 0.0 },
            auf.fehler()
        );
        auf.stoppen();
        sig.roh(serde_json::json!({"t":"screen","on":false}));
        std::thread::sleep(std::time::Duration::from_millis(200));
        ton.beenden();
        sig.verlassen();
        std::thread::sleep(std::time::Duration::from_millis(400));
        println!("SCHIRMTEST FERTIG");
        return Ok(());
    }

    // Kameratest (Stufe 2d): ECHTES Kamerabild als H.264 ins Meeting.
    //   freeviewer --meetkamera <raum> <passwort> [name] [sekunden] [kamera]
    if let Some(i) = std::env::args().position(|a| a == "--meetkamera") {
        let args: Vec<String> = std::env::args().collect();
        let raum = args.get(i + 1).cloned().unwrap_or_default();
        let pass = args.get(i + 2).cloned().unwrap_or_default();
        let name = args
            .get(i + 3)
            .cloned()
            .unwrap_or_else(|| "NativKamera".to_string());
        let dauer: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(25);
        let welche = args.get(i + 5).cloned().filter(|s| !s.is_empty());
        for (n, k) in meetcam::liste().iter().enumerate() {
            println!("KAMERA {} name={}", n, k.name);
        }
        let kam = match meetcam::oeffnen(welche, meetvideo::BREITE, meetvideo::HOEHE, 30) {
            Ok(k) => k,
            Err(e) => {
                println!("KAMERATEST FEHLER Kamera: {}", e);
                return Ok(());
            }
        };
        println!("KAMERA OFFEN name={} {}x{}", kam.name, kam.breite, kam.hoehe);
        let mut koder = match meetvideo::Kodierer::neu(kam.breite, kam.hoehe, 15, 1_500_000) {
            Ok(k) => k,
            Err(e) => {
                println!("KAMERATEST FEHLER Kodierer: {}", e);
                return Ok(());
            }
        };
        let sig = match meetsig::beitreten(&meet::base(), &raum, &pass, &name, "") {
            Ok(s) => s,
            Err(e) => {
                println!("KAMERATEST FEHLER Signalisierung: {}", e);
                return Ok(());
            }
        };
        let ton = match meetrtc::starten() {
            Ok(t) => t,
            Err(e) => {
                println!("KAMERATEST FEHLER Ton: {}", e);
                return Ok(());
            }
        };
        let mut angeboten = false;
        let mut pakete: u64 = 0;
        let mut gesendet: u64 = 0;
        // Zwei Messwerte, damit "es kam ein Bild" belegbar ist: mittlere
        // Helligkeit (nicht schwarz) und Unterschied zum Vorbild (bewegt).
        let mut hell_summe = 0f64;
        let mut bewegung = 0f64;
        let mut vorbild: Vec<u8> = Vec::new();
        let start = std::time::Instant::now();
        let mut naechstes = std::time::Instant::now();
        while start.elapsed().as_secs() < dauer {
            for e in sig.abholen() {
                match &e {
                    meetsig::Ereignis::Willkommen { .. } => {
                        if !angeboten {
                            angeboten = true;
                            sig.roh(serde_json::json!({"t":"offer","sdp":ton.angebot}));
                        }
                    }
                    meetsig::Ereignis::WarteDazu { peer, .. } => sig.warteraum("admit", Some(*peer)),
                    meetsig::Ereignis::Spur { mid, peer, .. } => ton.spur(mid, *peer),
                    meetsig::Ereignis::Sdp { art, sdp } if art == "answer" => {
                        ton.antwort(sdp);
                        sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                        sig.roh(serde_json::json!({"t":"publish","mid":ton.vid,"screen":false}));
                        println!("EREIGNIS Antwort eingespielt");
                    }
                    meetsig::Ereignis::Sdp { art, sdp } if art == "offer" => ton.server_angebot(sdp),
                    _ => {}
                }
            }
            if let Some(a) = ton.offene_antwort() {
                sig.roh(serde_json::json!({"t":"answer","sdp":a}));
            }
            for te in ton.abholen() {
                if let meetrtc::TonEreignis::Verbunden = te {
                    println!("EREIGNIS Ton verbunden");
                }
            }
            // 15 Bilder je Sekunde senden - das neueste, das die Kamera hat.
            if std::time::Instant::now() >= naechstes {
                naechstes += std::time::Duration::from_millis(66);
                if let Some(b) = kam.neuestes() {
                    let y = &b.nv12[..(b.breite * b.hoehe) as usize];
                    hell_summe += y.iter().map(|v| *v as f64).sum::<f64>() / y.len() as f64;
                    if vorbild.len() == y.len() {
                        let d: f64 = y
                            .iter()
                            .zip(vorbild.iter())
                            .map(|(a, c)| (*a as i32 - *c as i32).unsigned_abs() as f64)
                            .sum::<f64>()
                            / y.len() as f64;
                        bewegung += d;
                    }
                    vorbild = y.to_vec();
                    gesendet += 1;
                    match koder.nv12_rahmen(&b.nv12) {
                        Ok(teile) => {
                            for t in teile {
                                pakete += 1;
                                ton.bild_senden(t.data);
                            }
                        }
                        Err(e) => println!("KODIER-FEHLER {}", e),
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let z = ton.zahlen();
        println!(
            "ZAHLEN kamera={} aufgenommen={} verwendet={} kodiert={} pakete={} bild_gesendet={} bild_empfangen={} bytes_raus={} helligkeit={:.1} bewegung={:.2} fehler={}",
            kam.name,
            kam.aufgenommen(),
            gesendet,
            koder.bilder,
            pakete,
            z.bild_gesendet,
            z.bild_empfangen,
            z.bytes_raus,
            if gesendet > 0 { hell_summe / gesendet as f64 } else { 0.0 },
            if gesendet > 1 { bewegung / (gesendet - 1) as f64 } else { 0.0 },
            kam.fehler()
        );
        kam.stoppen();
        ton.beenden();
        sig.verlassen();
        std::thread::sleep(std::time::Duration::from_millis(400));
        println!("KAMERATEST FERTIG");
        return Ok(());
    }

    // Bildtest (Stufe 2b): sendet ein bewegtes Testmuster als H.264 ins
    // Meeting. Damit laesst sich die ganze Kette messen, ohne Kamera.
    //   freeviewer --meetbild <raum> <passwort> [name] [sekunden]
    if let Some(i) = std::env::args().position(|a| a == "--meetbild") {
        let args: Vec<String> = std::env::args().collect();
        let raum = args.get(i + 1).cloned().unwrap_or_default();
        let pass = args.get(i + 2).cloned().unwrap_or_default();
        let name = args.get(i + 3).cloned().unwrap_or_else(|| "NativBild".to_string());
        let dauer: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(25);
        let mut koder = match meetvideo::Kodierer::neu(
            meetvideo::BREITE,
            meetvideo::HOEHE,
            15,
            1_500_000,
        ) {
            Ok(k) => k,
            Err(e) => {
                println!("BILDTEST FEHLER Kodierer: {}", e);
                return Ok(());
            }
        };
        let mut muster = meetvideo::Muster::neu(meetvideo::BREITE, meetvideo::HOEHE);
        let sig = match meetsig::beitreten(&meet::base(), &raum, &pass, &name, "") {
            Ok(s) => s,
            Err(e) => {
                println!("BILDTEST FEHLER Signalisierung: {}", e);
                return Ok(());
            }
        };
        let ton = match meetrtc::starten() {
            Ok(t) => t,
            Err(e) => {
                println!("BILDTEST FEHLER Ton: {}", e);
                return Ok(());
            }
        };
        let mut angeboten = false;
        let mut pakete: u64 = 0;
        let start = std::time::Instant::now();
        let mut naechstes = std::time::Instant::now();
        while start.elapsed().as_secs() < dauer {
            for e in sig.abholen() {
                match &e {
                    meetsig::Ereignis::Willkommen { .. } => {
                        if !angeboten {
                            angeboten = true;
                            sig.roh(serde_json::json!({"t":"offer","sdp":ton.angebot}));
                        }
                    }
                    meetsig::Ereignis::WarteDazu { peer, .. } => sig.warteraum("admit", Some(*peer)),
                    meetsig::Ereignis::Spur { mid, peer, .. } => ton.spur(mid, *peer),
                    meetsig::Ereignis::Sdp { art, sdp } if art == "answer" => {
                        ton.antwort(sdp);
                        sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                        sig.roh(serde_json::json!({"t":"publish","mid":ton.vid,"screen":false}));
                        println!("EREIGNIS Antwort eingespielt");
                    }
                    meetsig::Ereignis::Sdp { art, sdp } if art == "offer" => ton.server_angebot(sdp),
                    _ => println!("EREIGNIS {:?}", e),
                }
            }
            if let Some(a) = ton.offene_antwort() {
                sig.roh(serde_json::json!({"t":"answer","sdp":a}));
            }
            for te in ton.abholen() {
                if let meetrtc::TonEreignis::Verbunden = te {
                    println!("EREIGNIS Ton verbunden");
                }
            }
            // 15 Bilder je Sekunde
            while std::time::Instant::now() >= naechstes {
                naechstes += std::time::Duration::from_millis(66);
                let rgb = muster.naechstes();
                match koder.rahmen(&rgb) {
                    Ok(teile) => {
                        for t in teile {
                            pakete += 1;
                            ton.bild_senden(t.data);
                        }
                    }
                    Err(e) => println!("KODIER-FEHLER {}", e),
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let z = ton.zahlen();
        println!(
            "ZAHLEN verbunden={} bilder_kodiert={} pakete={} bild_gesendet={} bild_empfangen={} bytes_raus={}",
            z.verbunden, koder.bilder, pakete, z.bild_gesendet, z.bild_empfangen, z.bytes_raus
        );
        ton.beenden();
        sig.verlassen();
        std::thread::sleep(std::time::Duration::from_millis(400));
        println!("BILDTEST FERTIG");
        return Ok(());
    }

    // Praxistest mit ECHTEN Geraeten (Stufe 1c):
    //   freeviewer --meetmik <raum> <passwort> [name] [sekunden]
    // Nimmt das Standardmikrofon, spielt die anderen ueber den Lautsprecher
    // und schaltet die Echo-Unterdrueckung dazwischen.
    if let Some(i) = std::env::args().position(|a| a == "--meetmik") {
        let args: Vec<String> = std::env::args().collect();
        let raum = args.get(i + 1).cloned().unwrap_or_default();
        let pass = args.get(i + 2).cloned().unwrap_or_default();
        let name = args.get(i + 3).cloned().unwrap_or_else(|| "NativMikro".to_string());
        let dauer: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(30);
        let (ein_liste, aus_liste) = meetaudio::geraete_liste();
        println!("MIKROFONE {:?}", ein_liste);
        println!("LAUTSPRECHER {:?}", aus_liste);
        let geraete = match meetaudio::geraete_starten(None, None) {
            Ok(g) => g,
            Err(e) => {
                println!("MIKROTEST FEHLER Geraete: {}", e);
                return Ok(());
            }
        };
        println!("EINGANG {} / AUSGANG {}", geraete.eingang, geraete.ausgang);
        let sig = match meetsig::beitreten(&meet::base(), &raum, &pass, &name, "") {
            Ok(s) => s,
            Err(e) => {
                println!("MIKROTEST FEHLER Signalisierung: {}", e);
                return Ok(());
            }
        };
        let ton = match meetrtc::starten() {
            Ok(t) => t,
            Err(e) => {
                println!("MIKROTEST FEHLER Ton: {}", e);
                return Ok(());
            }
        };
        let mut angeboten = false;
        let start = std::time::Instant::now();
        let mut mikro_rahmen: u64 = 0;
        let mut mikro_pegel = 0.0f32;
        while start.elapsed().as_secs() < dauer {
            for e in sig.abholen() {
                match &e {
                    meetsig::Ereignis::Willkommen { .. } => {
                        if !angeboten {
                            angeboten = true;
                            sig.roh(serde_json::json!({"t":"offer","sdp":ton.angebot}));
                        }
                    }
                    meetsig::Ereignis::WarteDazu { peer, .. } => sig.warteraum("admit", Some(*peer)),
                    meetsig::Ereignis::Spur { mid, peer, .. } => ton.spur(mid, *peer),
                    meetsig::Ereignis::Sdp { art, sdp } if art == "answer" => {
                        ton.antwort(sdp);
                        sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                    }
                    meetsig::Ereignis::Sdp { art, sdp } if art == "offer" => ton.server_angebot(sdp),
                    _ => println!("EREIGNIS {:?}", e),
                }
            }
            if let Some(a) = ton.offene_antwort() {
                sig.roh(serde_json::json!({"t":"answer","sdp":a}));
            }
            // Mikrofon -> Netz
            while let Ok(rahmen) = geraete.mikro.try_recv() {
                let spitze = rahmen
                    .iter()
                    .map(|v| (*v as f32 / 32768.0).abs())
                    .fold(0.0f32, f32::max);
                mikro_pegel = mikro_pegel.max(spitze);
                mikro_rahmen += 1;
                ton.senden(rahmen);
            }
            // Netz -> Lautsprecher
            for te in ton.abholen() {
                match te {
                    meetrtc::TonEreignis::Rahmen { quelle, pcm } => {
                        if let Ok(mut m) = geraete.lautsprecher.lock() {
                            m.dazu(quelle, &pcm);
                        }
                    }
                    meetrtc::TonEreignis::Bild { .. } => {}
                    meetrtc::TonEreignis::Verbunden => println!("EREIGNIS Ton verbunden"),
                    meetrtc::TonEreignis::Fehler(f) => println!("EREIGNIS Ton-Fehler {}", f),
                    meetrtc::TonEreignis::Ende(f) => println!("EREIGNIS Ton-Ende {}", f),
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let z = ton.zahlen();
        println!(
            "ZAHLEN verbunden={} gesendet={} empfangen={} mikro_rahmen={} mikro_pegel={:.3} pegel_rein={:.3}",
            z.verbunden, z.gesendet, z.empfangen, mikro_rahmen, mikro_pegel, z.pegel_rein
        );
        ton.beenden();
        sig.verlassen();
        std::thread::sleep(std::time::Duration::from_millis(400));
        println!("MIKROTEST FERTIG");
        return Ok(());
    }

    // Nativer TON-Test (Stufe 1b): beitreten, echte WebRTC-Verbindung zum
    // Medienserver aufbauen und einen Testton senden bzw. den Ton der
    // anderen messen.
    //   freeviewer --meetton <raum> <passwort> [name] [sekunden]
    if let Some(i) = std::env::args().position(|a| a == "--meetton") {
        let args: Vec<String> = std::env::args().collect();
        let raum = args.get(i + 1).cloned().unwrap_or_default();
        let pass = args.get(i + 2).cloned().unwrap_or_default();
        let name = args.get(i + 3).cloned().unwrap_or_else(|| "NativTon".to_string());
        let dauer: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(20);
        let sig = match meetsig::beitreten(&meet::base(), &raum, &pass, &name, "") {
            Ok(s) => s,
            Err(e) => {
                println!("TONTEST FEHLER Signalisierung: {}", e);
                return Ok(());
            }
        };
        let ton = match meetrtc::starten() {
            Ok(t) => t,
            Err(e) => {
                println!("TONTEST FEHLER Ton: {}", e);
                return Ok(());
            }
        };
        println!("ANGEBOT {} Zeichen, mid {}", ton.angebot.len(), ton.mid);
        let mut angeboten = false;
        let mut phase = 0.0f32;
        let start = std::time::Instant::now();
        let mut naechster = std::time::Instant::now();
        let mut pegel_max = 0.0f32;
        while start.elapsed().as_secs() < dauer {
            for e in sig.abholen() {
                match &e {
                    meetsig::Ereignis::Willkommen { .. } => {
                        if !angeboten {
                            angeboten = true;
                            sig.roh(serde_json::json!({"t":"offer","sdp":ton.angebot}));
                            println!("EREIGNIS Angebot verschickt");
                        }
                    }
                    meetsig::Ereignis::WarteDazu { peer, .. } => sig.warteraum("admit", Some(*peer)),
                    meetsig::Ereignis::Sdp { art, sdp } if art == "answer" => {
                        ton.antwort(sdp);
                        sig.roh(serde_json::json!({"t":"publish","mid":ton.mid,"screen":false}));
                        println!("EREIGNIS Antwort eingespielt ({} Zeichen)", sdp.len());
                    }
                    meetsig::Ereignis::Spur { mid, peer, .. } => {
                        ton.spur(mid, *peer);
                        println!("EREIGNIS Spur {} gehoert zu {}", mid, peer);
                    }
                    meetsig::Ereignis::Sdp { art, sdp } if art == "offer" => {
                        ton.server_angebot(sdp);
                        println!("EREIGNIS Server-Angebot bekommen");
                    }
                    _ => println!("EREIGNIS {:?}", e),
                }
            }
            if let Some(a) = ton.offene_antwort() {
                sig.roh(serde_json::json!({"t":"answer","sdp":a}));
                println!("EREIGNIS eigene Antwort verschickt");
            }
            for te in ton.abholen() {
                match te {
                    meetrtc::TonEreignis::Verbunden => println!("EREIGNIS Ton verbunden"),
                    meetrtc::TonEreignis::Fehler(f) => println!("EREIGNIS Ton-Fehler {}", f),
                    meetrtc::TonEreignis::Ende(f) => println!("EREIGNIS Ton-Ende {}", f),
                    meetrtc::TonEreignis::Rahmen { .. } => {}
                    meetrtc::TonEreignis::Bild {
                        quelle,
                        bildschirm,
                        daten,
                        schluesselbild,
                        codec,
                    } => {
                        println!(
                            "EREIGNIS {} von {} ({} Bytes, {}{})",
                            if bildschirm { "Bildschirm" } else { "Bild" },
                            quelle,
                            daten.len(),
                            codec,
                            if schluesselbild { ", Schluesselbild" } else { "" }
                        );
                    }
                }
            }
            // alle 20 ms ein Rahmen Testton (440 Hz)
            while std::time::Instant::now() >= naechster {
                naechster += std::time::Duration::from_millis(20);
                ton.senden(meetrtc::testton(&mut phase, 440.0));
            }
            let z = ton.zahlen();
            pegel_max = pegel_max.max(z.pegel_rein);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let z = ton.zahlen();
        println!(
            "ZAHLEN verbunden={} gesendet={} empfangen={} bild_gesendet={} bild_empfangen={} bytes_raus={} bytes_rein={} pegel_max={:.3}",
            z.verbunden, z.gesendet, z.empfangen, z.bild_gesendet, z.bild_empfangen,
            z.bytes_raus, z.bytes_rein, pegel_max
        );
        ton.beenden();
        sig.verlassen();
        std::thread::sleep(std::time::Duration::from_millis(500));
        println!("TONTEST FERTIG");
        return Ok(());
    }

    // Nativer Meeting-Test (Stufe 1, ohne Oberflaeche):
    //   freeviewer --meettest <raum> <passwort> [name] [sekunden]
    // Tritt bei, meldet jedes Ereignis, schickt einen Chat und geht wieder.
    if let Some(i) = std::env::args().position(|a| a == "--meettest") {
        let args: Vec<String> = std::env::args().collect();
        let raum = args.get(i + 1).cloned().unwrap_or_default();
        let pass = args.get(i + 2).cloned().unwrap_or_default();
        let name = args.get(i + 3).cloned().unwrap_or_else(|| "Nativ".to_string());
        let dauer: u64 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(12);
        match meetsig::beitreten(&meet::base(), &raum, &pass, &name, "") {
            Ok(s) => {
                let start = std::time::Instant::now();
                let mut chat_geschickt = false;
                while start.elapsed().as_secs() < dauer {
                    for e in s.abholen() {
                        println!("EREIGNIS {:?}", e);
                        // Im Test lassen wir Wartende sofort herein, damit sich
                        // der Weg Browser -> Warteraum -> nativer Gastgeber
                        // wirklich pruefen laesst.
                        if let meetsig::Ereignis::WarteDazu { peer, .. } = e {
                            s.warteraum("admit", Some(peer));
                        }
                        // Kommt jemand herein, begruessen wir ihn - so laesst
                        // sich pruefen, dass Chat auch WIRKLICH bei den
                        // anderen ankommt (frueher Gesagtes wird nicht
                        // nachgereicht, das ist Absicht des Servers).
                        if let meetsig::Ereignis::Dazu(_) = e {
                            s.chat("Willkommen im nativen Meeting");
                        }
                    }
                    if !chat_geschickt && s.zustand().ich != 0 {
                        s.chat("Hallo aus dem nativen Client");
                        s.hand(true);
                        chat_geschickt = true;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                println!("ZUSTAND {:?}", s.zustand());
                s.verlassen();
                std::thread::sleep(std::time::Duration::from_millis(600));
                println!("MEETTEST FERTIG");
            }
            Err(e) => println!("MEETTEST FEHLER {}", e),
        }
        return Ok(());
    }

    if std::env::args().any(|a| a == "--uitest") {
        let tmp = std::env::temp_dir().join("fv-uitest");
        let _ = std::fs::create_dir_all(&tmp);
        std::env::set_var("FV_CONFIG", &tmp);
        let ctx = egui::Context::default();
        install_theme(&ctx);
        let mut app = App::new(shared.clone(), false, None);
        app.book.started("123456789", "geheim", true);
        app.book.rename("123456789", "Test-PC");
        app.book.started("987654321", "", false);
        app.selected = Some("123456789".to_string());
        app.partner_id = "123456789".to_string();
        *shared.my_id.lock().unwrap() = "497628420".to_string();
        *shared.knock.lock().unwrap() = Some(shared::Knock {
            from: "Pait Laptop".to_string(),
            code: "1234".to_string(),
            at: std::time::Instant::now(),
        });
        let mut ok = true;
        for (name, view, stab) in [
            ("Start", View::Start, SettingsTab::General),
            ("Geraete", View::Devices, SettingsTab::General),
            ("Einstellungen", View::Settings, SettingsTab::General),
            ("Konto", View::Settings, SettingsTab::Account),
            ("Meet", View::Meet, SettingsTab::General),
        ] {
            app.view = view;
            app.stab = stab;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(1100.0, 720.0),
                )),
                ..Default::default()
            };
            let out = ctx.run(input, |ctx| {
                app.knock_ui(ctx);
                egui::CentralPanel::default().show(ctx, |ui| app.home_ui(ui));
            });
            let shapes = out.shapes.len();
            println!("{}: {} Formen gezeichnet", name, shapes);
            if shapes < 10 {
                ok = false;
            }
        }
        // Die Sitzungsansicht braucht ein Bild und einmal beide Leisten:
        // im Fenster und schwebend im Vollbild.
        shared.connected.store(true, Ordering::Relaxed);
        *shared.remote_size.lock().unwrap() = (1920, 1080);
        for (name, full) in [("Sitzung", false), ("Sitzung Vollbild", true)] {
            app.full = full;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(1100.0, 720.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input.clone(), |ctx| app.session_ui(ctx));
            let out = ctx.run(input, |ctx| app.session_ui(ctx));
            let shapes = out.shapes.len();
            println!("{}: {} Formen gezeichnet", name, shapes);
            if shapes < 10 {
                ok = false;
            }
        }
        shared.connected.store(false, Ordering::Relaxed);

        // ---- Das Meetingfenster. Beitritts-Schirm einmal ueber den echten
        // Weg (mit Geraeteabfrage), das laufende Meeting ueber eine
        // zusammengebaute `Sicht` - so bekommt man volle Buehne, Chat,
        // Warteraum und Bildschirmfreigabe zu sehen, ohne Server und Kamera.
        app.headless = true;
        app.meet_win_open(meet::Meeting {
            id: "482-913-770".to_string(),
            titel: "Wochenbesprechung".to_string(),
            passwort: "test1234".to_string(),
            termin_text: String::new(),
        });
        let meetschirm = |b: f32, h: f32| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(b, h),
            )),
            ..Default::default()
        };
        {
            // Zweimal zeichnen: der erste Rahmen legt Texturen und Ids an.
            let _ = ctx.run(meetschirm(920.0, 640.0), |ctx| app.meet_win_ui(ctx));
            let out = ctx.run(meetschirm(920.0, 640.0), |ctx| app.meet_win_ui(ctx));
            let shapes = out.shapes.len();
            println!("Meet Beitritt: {} Formen gezeichnet", shapes);
            if shapes < 30 {
                ok = false;
            }
        }
        // Warteraum und Beitritts-Schirm gehoeren mit auf den Pruefstand.
        {
            let _ = ctx.run(meetschirm(920.0, 640.0), |ctx| {
                meetfenster::warteschirm_ui(ctx, "Wochenbesprechung", "Gleich gehts los.");
            });
            let out = ctx.run(meetschirm(920.0, 640.0), |ctx| {
                meetfenster::warteschirm_ui(ctx, "Wochenbesprechung", "Gleich gehts los.");
            });
            println!("Meet Warteschirm: {} Formen gezeichnet", out.shapes.len());
            if out.shapes.len() < 12 {
                ok = false;
            }
        }
        // Alle Beispielzustaende durchrendern - dieselben, die --meetdemo
        // spaeter als echtes Bild zeigt.
        let bilder = meetfenster::pruefbilder(&ctx);
        for i in 0..meetfenster::BEISPIELE {
            let (name, sicht, mut z) = meetfenster::beispiel(i);
            let (b, h) = if name.starts_with("Info") { (560.0, 430.0) } else { (1100.0, 720.0) };
            // Zweimal: der erste Rahmen legt Ids und Texturen an.
            let _ = ctx.run(meetschirm(b, h), |ctx| {
                meetfenster::meeting_ui(ctx, &sicht, &mut z, &bilder);
            });
            let out = ctx.run(meetschirm(b, h), |ctx| {
                meetfenster::meeting_ui(ctx, &sicht, &mut z, &bilder);
            });
            let shapes = out.shapes.len();
            println!("Meet {}: {} Formen gezeichnet", name, shapes);
            if shapes < 40 {
                ok = false;
            }
        }
        app.meet_win.offen = false;

        let _ = std::fs::remove_dir_all(&tmp);
        println!("{}", if ok { "UITEST OK" } else { "UITEST FAIL" });
        return Ok(());
    }

    // Abgleich mit dem Konto einmal von Hand:  freeviewer --synctest [token]
    // Ohne Token wird der genommen, der in account.json steht. Zeigt genau,
    // was hoch- und was zurueckkam - fuer Fehlersuche ohne Fenster.
    if std::env::args().any(|a| a == "--synctest") {
        let arg = std::env::args()
            .skip_while(|a| a != "--synctest")
            .nth(1)
            .unwrap_or_default();
        let token = if arg.is_empty() || arg.starts_with("--") {
            account::load().map(|s| s.token).unwrap_or_default()
        } else {
            arg
        };
        if token.is_empty() {
            println!("SYNCTEST: kein Token - erst anmelden");
            return Ok(());
        }
        let book = partners::Book::load();
        let mine = book.to_sync();
        println!("SYNCTEST: {} Geraete gehen hoch", mine.len());
        match account::sync(&shared.relay_url, &token, mine) {
            Ok(r) => {
                println!(
                    "SYNCTEST OK: Konto {}, Stand {}, {} Geraete zurueck",
                    r.username,
                    r.rev,
                    r.devices.len()
                );
                for d in r.devices.iter().take(20) {
                    println!(
                        "  {} {} {}{}",
                        d.id,
                        if d.name.is_empty() { "-" } else { &d.name },
                        d.at,
                        if d.deleted { " (entfernt)" } else { "" }
                    );
                }
            }
            Err(e) => println!("SYNCTEST FEHLER: {}", e),
        }
        return Ok(());
    }

    // ---- the Windows service -------------------------------------------
    // "freeviewer --service" is what the service control manager starts.
    if std::env::args().any(|a| a == "--service") {
        // Der Dienst spielt Updates selbst ein - er laeuft als SYSTEM und
        // braucht dafuer keine Administrator-Nachfrage.
        #[cfg(windows)]
        update::service_watcher();
        if let Err(e) = service::run() {
            service::log(&format!("Dienst konnte nicht starten: {}", e));
        }
        return Ok(());
    }
    // install / remove, asking for admin rights when we do not have them yet
    for (flag, install) in [("--install-service", true), ("--uninstall-service", false)] {
        if std::env::args().any(|a| a == flag) {
            if !service::is_elevated() {
                println!("Administrator-Rechte anfordern...");
                match service::elevate(flag) {
                    Ok(()) => println!("Windows fragt jetzt nach der Bestaetigung."),
                    Err(e) => println!("FAIL: {}", e),
                }
                return Ok(());
            }
            let r = if install {
                service::install()
            } else {
                service::uninstall()
            };
            match r {
                Ok(()) => println!(
                    "{}",
                    if install {
                        "Dienst installiert und gestartet."
                    } else {
                        "Dienst gestoppt und entfernt."
                    }
                ),
                Err(e) => println!("FAIL: {}", e),
            }
            return Ok(());
        }
    }
    // state of service and autostart:  freeviewer --status
    if std::env::args().any(|a| a == "--status") {
        println!("Version:    {}", update::VERSION);
        println!("Config:     {}", ident::config_dir().display());
        println!("Autostart:  {}", match autostart::current() {
            Some(c) => c,
            None => "aus".to_string(),
        });
        println!("Dienst:     installiert={} laeuft={}", service::installed(), service::running());
        match service::published() {
            Some(p) => println!("Agent:      ID {} Passwort {} (vor {} s, Desktop {})",
                p.id, p.password,
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs()).unwrap_or(0).saturating_sub(p.at),
                p.desktop),
            None => println!("Agent:      nichts veroeffentlicht"),
        }
        return Ok(());
    }
    // switch autostart from a script:  freeviewer --autostart on|off
    if let Some(pos) = std::env::args().position(|a| a == "--autostart") {
        let arg: Vec<String> = std::env::args().collect();
        match arg.get(pos + 1).map(|s| s.as_str()) {
            Some("on") | Some("off") => {
                let on = arg[pos + 1] == "on";
                match autostart::set(on) {
                    Ok(()) => println!("Autostart {}", if on { "an" } else { "aus" }),
                    Err(e) => println!("FAIL: {}", e),
                }
            }
            _ => {}
        }
        println!("Autostart jetzt: {:?} (zeigt hierher: {})", autostart::current(), autostart::points_here());
        return Ok(());
    }

    // update check/installation from the command line:  freeviewer --update
    if std::env::args().any(|a| a == "--update") {
        update::cleanup();
        println!("lokal: {}", update::VERSION);
        match update::check() {
            Ok(rel) => {
                println!(
                    "relay: {} ({} Bytes, {}...)",
                    rel.version,
                    rel.size,
                    &rel.sha256[..12]
                );
                if update::newer(&rel.version, update::VERSION) {
                    println!("installiere...");
                    match update::download(&rel) {
                        Ok(tmp) => {
                            println!("geladen + Pruefsumme ok: {}", tmp.display());
                            if std::env::args().any(|a| a == "--dry") {
                                let _ = std::fs::remove_file(&tmp);
                                println!("DRY: nicht ersetzt");
                                return Ok(());
                            }
                            match update::swap(&tmp) {
                                Ok(p) => println!("UPDATE OK -> {}", p.display()),
                                Err(e) => println!("FAIL: {}", e),
                            }
                        }
                        Err(e) => println!("FAIL: {}", e),
                    }
                } else {
                    println!("schon aktuell");
                }
            }
            Err(e) => println!("FAIL: {}", e),
        }
        return Ok(());
    }

    // which screens can this machine share?   freeviewer --monitors
    if std::env::args().any(|a| a == "--monitors") {
        for (i, m) in hostside::monitor_list(true).iter().enumerate() {
            println!(
                "{}: {} {}x{}{}",
                i,
                m.name,
                m.w,
                m.h,
                if m.primary { "  (primaer)" } else { "" }
            );
        }
        return Ok(());
    }

    // capture self test:  freeviewer --captest   (writes <config>/captest.txt)
    if std::env::args().any(|a| a == "--captest") {
        let report = hostside::capture_selftest(20);
        let path = ident::config_dir().join("captest.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }

    // GPU vs CPU scaling:  freeviewer --gputest [rounds]
    if std::env::args().any(|a| a == "--gputest") {
        let rounds: u32 = std::env::args()
            .skip_while(|a| a != "--gputest")
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        let report = hostside::gpu_selftest(rounds);
        let path = ident::config_dir().join("gputest.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }

    // H.264 codec self test:  freeviewer --h264test [rounds]
    if std::env::args().any(|a| a == "--h264test") {
        let rounds: u32 = std::env::args()
            .skip_while(|a| a != "--h264test")
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        let report = h264::selftest(rounds);
        let path = ident::config_dir().join("h264test.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }

    // live H.264 vs JPEG on the real screen:  freeviewer --videotest [rounds]
    if std::env::args().any(|a| a == "--videotest") {
        let rounds: u32 = std::env::args()
            .skip_while(|a| a != "--videotest")
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        let report = hostside::video_selftest(rounds);
        let path = ident::config_dir().join("videotest.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }

    // can we punch a hole?  freeviewer --p2ptest
    if std::env::args().any(|a| a == "--p2ptest") {
        let report = p2p::selftest();
        let path = ident::config_dir().join("p2ptest.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }

    // delta/full-frame benchmark:  freeviewer --deltatest [rounds]
    if std::env::args().any(|a| a == "--deltatest") {
        let rounds: u32 = std::env::args()
            .skip_while(|a| a != "--deltatest")
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);
        let report = hostside::delta_selftest(rounds);
        let path = ident::config_dir().join("deltatest.txt");
        let _ = std::fs::write(&path, &report);
        println!("{}", report);
        return Ok(());
    }
    // in --connect test mode we only act as a viewer (otherwise this process
    // would register the same machine identity and kick the real host offline)
    let viewer_only = std::env::args()
        .any(|a| a == "--connect" || a == "--inputtest" || a == "--ask");
    // Started by the service? Then we ARE the host of this machine.
    let is_agent = std::env::args().any(|a| a == "--agent");
    // Two processes with the same identity would kick each other off the
    // relay, so the GUI keeps its hands off while the service does the job.
    let service_owns_host = !is_agent && service::running();
    // Der Mac darf Host sein, sobald die Anwendung mit einer Developer-ID
    // signiert ist - ohne die vergibt macOS 15 die noetigen Rechte nicht
    // dauerhaft (bei jedem Selbst-Update waeren sie wieder weg). Wir starten
    // den Host trotzdem und sagen dem Nutzer, welche zwei Haken er braucht.
    if !viewer_only && !service_owns_host {
        let host_shared = shared.clone();
        let host_secret = secret.clone();
        rt().spawn(async move {
            hostside::run_host(host_shared, host_secret, is_agent).await;
        });
    }
    if cfg!(target_os = "macos") && !viewer_only && !service_owns_host {
        shared.set_host_status(i18n::t("host.mac_permissions"));
    }
    if is_agent {
        // let the user's GUI show ID and password of this host
        service::publish_loop(shared.clone());
        service::log(&format!("Agent laeuft: {}", service::desktop_report()));
        // follow the input desktop (lock screen, UAC prompt, ...)
        service::watch_desktop();
    }
    if service_owns_host {
        shared.set_host_status("Der Dienst betreibt den Host - auch am Anmeldebildschirm");
    }
    if !is_agent && !viewer_only {
        // ID und Passwort vom Dienst-Agenten uebernehmen, sobald er sie
        // veroeffentlicht - egal ob der Dienst schon beim Start lief oder
        // die Oberflaeche erst spaeter ersetzt wurde. Nur frische Angaben
        // (< 90 s) gelten, damit eine alte state.json nichts ueberschreibt.
        let sh = shared.clone();
        std::thread::spawn(move || loop {
            if let Some(p) = service::published() {
                let frisch = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
                    .saturating_sub(p.at)
                    < 90;
                if frisch && !p.id.is_empty() {
                    *sh.my_id.lock().unwrap() = p.id;
                    *sh.password.lock().unwrap() = p.password;
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        });
    }

    // headless host mode (no window) - handy for servers and for testing
    if std::env::args().any(|a| a == "--headless") {
        println!("{} headless host, relay = {}", crate::brand::NAME, shared.relay_url);
        println!("password = {}", shared.password.lock().unwrap());
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let id = shared.my_id.lock().unwrap().clone();
            if !id.is_empty() {
                println!(
                    "ID {} | {} | {}",
                    id,
                    shared.host_status.lock().unwrap(),
                    shared.host_peer.lock().unwrap()
                );
            }
        }
    }

    // headless viewer mode for testing:
    //   freeviewer --connect <id> <password> [frames] [--game]
    let argv: Vec<String> = std::env::args().collect();
    if let Some(pos) = argv.iter().position(|a| a == "--connect") {
        let id = argv.get(pos + 1).cloned().unwrap_or_default();
        let pw = argv.get(pos + 2).cloned().unwrap_or_default();
        let want: u64 = argv
            .get(pos + 3)
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let game = argv.iter().any(|a| a == "--game");
        // extra checks for this scripted session
        let mon_arg: Option<u8> = argv
            .iter()
            .position(|a| a == "--monitor")
            .and_then(|i| argv.get(i + 1))
            .and_then(|s| s.parse().ok());
        let file_arg: Option<String> = argv
            .iter()
            .position(|a| a == "--sendfile")
            .and_then(|i| argv.get(i + 1))
            .cloned();
        let sh = shared.clone();
        let idc = id.clone();
        rt().spawn(async move { viewer::run_viewer(sh, idc, pw).await });
        let start = std::time::Instant::now();
        let mut last = 0u64;
        let mut mode_sent = false;
        let mut extras_done = false;
        let mut mon_switched = false;
        let mut mons_printed = false;
        loop {
            std::thread::sleep(Duration::from_millis(200));
            if !mons_printed {
                let mons = shared.monitors.lock().unwrap().clone();
                if !mons.is_empty() {
                    mons_printed = true;
                    for (i, m) in mons.iter().enumerate() {
                        println!("monitor {}: {} {}x{}", i, m.name, m.w, m.h);
                    }
                }
            }
            if !extras_done && shared.connected.load(Ordering::Relaxed) {
                extras_done = true;
                if let Some(idx) = mon_arg {
                    println!("switching to monitor {}", idx);
                    shared.send_input(Msg::SetMonitor { index: idx });
                }
                if let Some(f) = file_arg.clone() {
                    let path = std::path::PathBuf::from(&f);
                    match shared.xfer.lock().unwrap().as_mut() {
                        Some(x) => {
                            println!("sending file {}", f);
                            x.send_path(path);
                        }
                        None => println!("FAIL: kein Transfer-Modul"),
                    }
                }
            }
            if mon_arg.is_some() && !mon_switched && shared.connected.load(Ordering::Relaxed) {
                let act = shared.active_monitor.load(Ordering::Relaxed);
                if Some(act) == mon_arg {
                    let (rw, rh) = *shared.remote_size.lock().unwrap();
                    println!("monitor active {} -> {}x{}", act, rw, rh);
                    mon_switched = true;
                }
            }
            if file_arg.is_some() {
                let done = shared
                    .xfers
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|x| !x.incoming && x.finished)
                    .cloned();
                if let Some(p) = done {
                    if p.error.is_empty() {
                        println!("FILE OK: {} ({} Bytes)", p.name, p.done);
                        std::process::exit(0);
                    }
                    println!("FILE FAIL: {} - {}", p.name, p.error);
                    std::process::exit(1);
                }
                if start.elapsed() > Duration::from_secs(120) {
                    println!("FILE FAIL: Zeitueberschreitung");
                    std::process::exit(1);
                }
                continue;
            }
            if game && !mode_sent && shared.connected.load(Ordering::Relaxed) {
                shared.send_input(Msg::SetMode {
                    mode: proto::MODE_GAME,
                });
                shared.mode.store(proto::MODE_GAME, Ordering::Relaxed);
                mode_sent = true;
                println!("mode -> game");
            }
            let seq = shared
                .frame
                .lock()
                .unwrap()
                .as_ref()
                .map(|f| (f.seq, f.width, f.height))
                .unwrap_or((0, 0, 0));
            if seq.0 != last {
                last = seq.0;
                println!("frame {} {}x{}", seq.0, seq.1, seq.2);
            }
            if last >= want {
                let st = *shared.stats.lock().unwrap();
                let cur = *shared.remote_cursor.lock().unwrap();
                let udp = shared.udp_frames.load(Ordering::Relaxed);
                println!(
                    "OK: {} Frames in {:.1}s, {:.0} fps, {:.0} kbit/s, {:.0} ms rtt, Cursor {:?}",
                    last,
                    start.elapsed().as_secs_f32(),
                    st.fps,
                    st.kbps,
                    st.latency_ms,
                    cur
                );
                println!(
                    "Transport: {} ({} Bilder direkt per UDP, Rest ueber den Relay)",
                    if shared.direct.load(Ordering::Relaxed) {
                        "direkt (P2P)"
                    } else {
                        "Relay"
                    },
                    udp
                );
                std::process::exit(0);
            }
            let status = shared.viewer_status.lock().unwrap().clone();
            if status.starts_with("Fehler") {
                println!("FAIL: {}", status);
                std::process::exit(1);
            }
            if start.elapsed() > Duration::from_secs(30) {
                println!("FAIL: {}", shared.viewer_status.lock().unwrap());
                std::process::exit(1);
            }
        }
    }

    // scripted "please confirm" test:  freeviewer --ask <id> [frames]
    if let Some(pos) = argv.iter().position(|a| a == "--ask") {
        let id = argv.get(pos + 1).cloned().unwrap_or_default();
        let want: u64 = argv
            .get(pos + 2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let sh = shared.clone();
        let idc = id.clone();
        rt().spawn(async move { viewer::run_viewer_ask(sh, idc).await });
        let start = std::time::Instant::now();
        let mut last = 0u64;
        loop {
            std::thread::sleep(Duration::from_millis(200));
            let seq = shared
                .frame
                .lock()
                .unwrap()
                .as_ref()
                .map(|f| f.seq)
                .unwrap_or(0);
            if seq != last {
                last = seq;
                println!("frame {}", seq);
            }
            if last >= want {
                println!(
                    "OK: {} Frames in {:.1}s, Code {}",
                    last,
                    start.elapsed().as_secs_f32(),
                    shared.session_code.lock().unwrap()
                );
                std::process::exit(0);
            }
            let status = shared.viewer_status.lock().unwrap().clone();
            if status.starts_with("Fehler") {
                println!("FAIL: {}", status);
                std::process::exit(1);
            }
            if start.elapsed() > Duration::from_secs(45) {
                println!("FAIL: Zeitueberschreitung - {}", status);
                std::process::exit(1);
            }
        }
    }

    // scripted input self test:  freeviewer --inputtest <id> <password>
    if let Some(pos) = argv.iter().position(|a| a == "--inputtest") {
        selftest::input_selftest(shared.clone(), &argv, pos);
    }

    vinput::init(shared.clone());
    // Ist der Dienst installiert, macht ER das Update (als SYSTEM, ohne
    // Nachfrage). Oberflaeche und Agent sehen dann nur nach und melden es -
    // sonst fragten beide staendig nach Administrator-Rechten.
    update::watcher(shared.clone(), !service::installed());

    // Bild der eigenen Oberflaeche:
    //   freeviewer --shot C:\bild.jpg [--view start|devices|settings] [--demo]
    // Die App zeichnet sich selbst und speichert das Ergebnis - das klappt auch
    // dann, wenn der Bildschirm gesperrt ist und ein Fensterfoto nur weiss waere.
    if let Some(i) = std::env::args().position(|a| a == "--shot") {
        let path = std::env::args()
            .nth(i + 1)
            .unwrap_or_else(|| "freeviewer-shot.jpg".to_string());
        let want = std::env::args().skip_while(|a| a != "--view").nth(1);
        let demo = std::env::args().any(|a| a == "--demo");
        let knock = std::env::args().any(|a| a == "--knock");
        let shot_shared = shared.clone();
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1180.0, 800.0])
                .with_visible(true)
                .with_icon(app_icon())
                .with_title(crate::brand::NAME),
            ..Default::default()
        };
        return eframe::run_native(
            crate::brand::NAME,
            options,
            Box::new(move |cc| {
                install_theme(&cc.egui_ctx);
                let mut app = App::new(shot_shared.clone(), false, None);
                if demo {
                    app.book.started("123456789", "geheim", true);
                    app.book.rename("123456789", "Buero-PC");
                    app.book.started("987654321", "", false);
                    app.selected = Some("123456789".to_string());
                    app.partner_id = "123456789".to_string();
                    *shot_shared.my_id.lock().unwrap() = "497628420".to_string();
                }
                if let Some(i) = std::env::args().position(|a| a == "--meetdemo") {
                    app.meet_demo = Some(
                        std::env::args()
                            .nth(i + 1)
                            .and_then(|x| x.parse().ok())
                            .unwrap_or(0),
                    );
                    app.headless = true;
                }
                if knock {
                    *shot_shared.knock.lock().unwrap() = Some(shared::Knock {
                        from: "Laptop von Justin".to_string(),
                        code: "1234".to_string(),
                        at: std::time::Instant::now(),
                    });
                }
                app.view = match want.as_deref() {
                    Some("devices") => View::Devices,
                    Some("meet") => View::Meet,
                    Some("settings") => View::Settings,
                    Some("settings2") => View::Settings,
                    _ => View::Start,
                };
                if let Some(tab) = std::env::args().skip_while(|a| a != "--tab").nth(1) {
                    app.stab = match tab.as_str() {
                        "access" => SettingsTab::Access,
                        "account" => SettingsTab::Account,
                        "audio" => SettingsTab::Audio,
                        "look" => SettingsTab::Look,
                        "deploy" => SettingsTab::Deploy,
                        "feedback" => SettingsTab::Feedback,
                        "about" => SettingsTab::About,
                        _ => SettingsTab::General,
                    };
                }
                if let Some(mi) = std::env::args().position(|a| a == "--meetshot") {
                    // Das eigene Meeting-Fenster oeffnen und DAVON das Bild.
                    // Mit "--meetshot bild.jpg RAUM PASS" wird wirklich
                    // beigetreten - dann zeigt das Bild das echte Meeting und
                    // nicht nur den Beitritts-Schirm.
                    // Aufruf:  --shot bild.jpg --meetshot [RAUM PASS]
                    let a2 = std::env::args().nth(mi + 1).unwrap_or_default();
                    let a3 = std::env::args().nth(mi + 2).unwrap_or_default();
                    let echt = a2.contains('-') && !a3.is_empty();
                    app.meet_win_open(meet::Meeting {
                        id: if echt { a2.clone() } else { "482-913-770".to_string() },
                        titel: "Fenster-Test".to_string(),
                        passwort: if echt { a3.clone() } else { "test1234".to_string() },
                        termin_text: String::new(),
                    });
                    app.meet_shot_join = echt;
                    // Mit echtem Beitritt muss die Verbindung erst stehen.
                    app.shot_warte = if echt { 900 } else { 60 };
                    // shot MUSS mitgesetzt sein - der Bildmacher haengt daran.
                    app.shot = Some(std::path::PathBuf::from(path.clone()));
                    app.meet_shot = Some(std::path::PathBuf::from(path));
                } else {
                    app.shot = Some(std::path::PathBuf::from(path));
                }
                Ok(Box::new(app))
            }),
        );
    }

    // Started by the autostart entry (or by the service): no window in the
    // user's face, only the tray icon. The host runs either way.
    let start_hidden = std::env::args().any(|a| a == "--tray" || a == "--background");
    // Kam eine freeviewer://-Adresse mit? Dann in den Briefkasten legen -
    // egal ob diese Fassung sie selbst abholt oder die schon laufende.
    if let Some(url) = std::env::args().find(|a| link::is_ours(a)) {
        if link::parse(&url).is_some() {
            link::drop_in(&url);
        }
    }

    // Frisch aus dem Browser geladen? Dann steckt der Einrichtungs-Auftrag
    // (Code + Wunschname) am Ende der Datei - das Geraet richtet sich selbst
    // ein, sobald die Oberflaeche ihre ID kennt.
    let embedded = if setup::running_installed() {
        None
    } else {
        link::embedded_setup()
    };

    // A second window of the same user is pointless - bring the first one to
    // the front instead. Der Dienst-Agent zaehlt nicht als zweites Fenster,
    // er laeuft bewusst neben der Oberflaeche.
    if !tray::claim_single_instance(is_agent) {
        if !is_agent {
            println!("{} laeuft bereits - Fenster nach vorne geholt.", crate::brand::NAME);
        }
        return Ok(());
    }
    // Das Schema fuer diesen Nutzer eintragen (portabel, ohne Adminrechte).
    // Bei der richtigen Installation kommt zusaetzlich der maschinenweite
    // Eintrag dazu.
    if !link::points_here() {
        let _ = link::register(false);
    }
    autostart::refresh();
    if is_agent {
        // Zwei gleiche Tablett-Symbole (Agent + Oberflaeche) taugen nichts:
        // der Agent zeigt seins nur, solange keine Oberflaeche laeuft.
        let tray_shared = shared.clone();
        std::thread::spawn(move || loop {
            if tray::gui_running() {
                if tray::showing() {
                    tray::remove();
                }
            } else if !tray::showing() {
                tray::start(tray_shared.clone());
            }
            std::thread::sleep(Duration::from_secs(4));
        });
    } else {
        tray::start(shared.clone());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([700.0, 470.0])
            .with_visible(!start_hidden)
            .with_icon(app_icon())
            .with_title(crate::brand::NAME),
        ..Default::default()
    };

    eframe::run_native(
        crate::brand::NAME,
        options,
        Box::new(move |cc| {
            install_theme(&cc.egui_ctx);
            Ok(Box::new(App::new(shared, start_hidden, embedded.clone())))
        }),
    )
}

struct App {
    shared: Arc<Shared>,
    partner_id: String,
    partner_pw: String,
    tex: Option<egui::TextureHandle>,
    last_seq: u64,
    last_mods: egui::Modifiers,
    viewer_task: Option<tokio::task::JoinHandle<()>>,
    hint: String,
    /// Is a permanent password stored for this machine?
    pw_fixed: bool,
    /// Everyone we connected to before.
    book: partners::Book,
    /// Store the password of the next connection?
    remember_pw: bool,
    /// Entry currently being renamed: (id, buffer).
    renaming: Option<(String, String)>,
    /// Running session: partner id + when it started.
    session: Option<(String, std::time::Instant)>,
    /// Until when the "how do I get out" hint stays on screen.
    hint_until: Option<std::time::Instant>,
    /// Started into the tray, so the window has to disappear once winit has
    /// created it.
    start_hidden: bool,
    first_frame: bool,
    /// Does the Run key point at us? Re-read every few seconds, because the
    /// tray menu can change it behind the GUI's back.
    autostart: bool,
    autostart_checked: std::time::Instant,
    /// The "I am still running down there" balloon is shown only once.
    told_about_tray: bool,
    /// Is the Windows service installed and running?
    service_on: bool,
    /// Until when a start into the tray keeps forcing the window away.
    hide_until: Option<std::time::Instant>,
    /// Which of the three pages is on screen.
    view: View,
    /// Filter of the device list.
    search: String,
    /// Device shown in the detail column.
    selected: Option<String>,
    /// Who is online right now (asked at the relay).
    presence: Arc<presence::Watch>,
    /// --shot: wohin das Bild der eigenen Oberflaeche geschrieben wird.
    shot: Option<std::path::PathBuf>,
    /// --meetshot: dasselbe fuer das eigene Meeting-Fenster (Testhaken).
    meet_shot: Option<std::path::PathBuf>,
    /// Framezaehler und Scharfschaltung des Meetshots.
    meet_shot_n: u32,
    meet_shot_arm: u32,
    /// --meetshot mit Raum/Passwort: automatisch beitreten.
    meet_shot_join: bool,
    /// Nach wie vielen Rahmen das Bild gemacht wird (Beitritt braucht Zeit).
    shot_warte: u32,
    /// Gezeichnete Frames seit dem Start im --shot-Modus.
    /// Gewaehltes Aussehen (Vorlage, Akzent, Groesse, Rundung).
    look: theme::Appearance,
    /// Gewaehlter Bereich in den Einstellungen.
    stab: SettingsTab,
    /// Feedback: Art, Text, Kontakt.
    fb_bug: bool,
    fb_text: String,
    fb_contact: String,
    /// Geraeteliste: gewaehlter Ordner und offenes Bearbeiten-Feld.
    folder: String,
    /// Offenes "Geraet hinzufuegen"-Fenster.
    add_dev: Option<AddDev>,
    edit_dev: Option<DevEdit>,
    /// Wann die Titelleiste zuletzt eingefaerbt wurde.
    caption_tick: std::time::Instant,
    shot_n: u32,
    /// Feste Passwoerter dieses Rechners (Einstellungen -> Zugriff).
    pw_list: Vec<pwlist::Entry>,
    /// Eingabefelder fuer einen neuen Eintrag.
    pw_new: String,
    pw_new_label: String,
    /// Welcher Eintrag gerade im Klartext zu sehen ist.
    pw_show: Option<usize>,
    /// Soll beim Installieren gleich der Dienst mit eingerichtet werden?
    install_service: bool,
    /// Wann zuletzt im Briefkasten nachgesehen wurde.
    link_check: std::time::Instant,
    /// Meet: Eingaben und was der Server zuletzt gesagt hat.
    meet_title: String,
    meet_id: String,
    meet_pw: String,
    meet_last: Option<meet::Meeting>,
    meet_list: Arc<std::sync::Mutex<Vec<meet::Meeting>>>,
    meet_busy: Arc<std::sync::atomic::AtomicBool>,
    meet_err: Arc<std::sync::Mutex<String>>,
    meet_loaded: bool,
    /// Beim Beitritt anbieten, dass andere diesen PC steuern duerfen.
    meet_offer_control: bool,
    /// Das eigene Meet-Fenster (Zoom-Ablauf in einem separaten Viewport).
    meet_win: MeetWin,
    /// Natives Meeting (Stufe 1: Ton, Chat, Warteraum) - laeuft ohne Browser.
    nativ_meet: Option<meetui::NativMeet>,
    /// Bildkacheln des nativen Meetings: Teilnehmer -> (Stand, Textur).
    nativ_bilder: std::collections::HashMap<u64, (u64, egui::TextureHandle)>,
    /// Geteilte Bildschirme: Teilnehmer -> (Stand, Textur). Eigener Speicher,
    /// weil ein Teilnehmer Kamera UND Bildschirm gleichzeitig schickt.
    nativ_schirme: std::collections::HashMap<u64, (u64, egui::TextureHandle)>,
    /// Eigenes Kamerabild im nativen Meeting: (Stand, Textur).
    nativ_eigen: Option<(u64, egui::TextureHandle)>,
    /// Kamera und Mikrofon VOR dem Beitritt (Selbstvorschau, Pegel).
    vorprobe: meetui::Vorprobe,
    /// Was nur die Oberflaeche angeht: Seitenleiste, Reiter, Kameraplatz.
    meet_z: meetfenster::Fensterzustand,
    /// Wie viele Chatzeilen der Nutzer schon gesehen hat (Zaehler am Knopf).
    meet_gelesen: usize,
    /// Wann die Kamera NACH dem Beitritt anlaufen darf. Die Vorschau haelt
    /// das Geraet bis zum Beitritt fest; unter Windows braucht die Kamera
    /// einen Moment, bis sie wieder frei ist - sofort neu oeffnen scheitert.
    meet_kam_start: Option<std::time::Instant>,
    /// Kein echtes Fenstersystem (--uitest): dann keine Extra-Fenster oeffnen.
    headless: bool,
    /// --meetdemo N: statt der Oberflaeche einen Beispielzustand des
    /// Meetings zeichnen. Nur dafuer da, echte Bilder machen zu koennen -
    /// der Bauserver hat weder Kamera noch zweiten Teilnehmer.
    meet_demo: Option<usize>,
    /// Vollbild wie in der Windows-Fernverbindung: kein Fensterrahmen, die
    /// Bedienleiste schwebt ueber dem Bild und laesst sich verschieben.
    full: bool,
    /// Wo die schwebende Leiste steht (0 = ganz links, 1 = ganz rechts).
    bar_x: f32,
    /// Wie breit sie zuletzt war (fuer die Positionsrechnung).
    bar_w: f32,
    /// Angeheftet = bleibt immer sichtbar, sonst zieht sie sich zurueck.
    bar_pinned: bool,
    /// Wann die Leiste zuletzt beachtet wurde (fuer das Zurueckziehen).
    bar_seen: std::time::Instant,
    /// Wann das letzte Bild ankam - fuer den Hinweis "kein Bild".
    frame_at: std::time::Instant,
    /// Konto: angemeldete Sitzung, Eingabefelder, Zustand des Abgleichs.
    acc: Option<account::Session>,
    acc_user: String,
    acc_pass: String,
    acc_msg: String,
    acc_busy: Arc<std::sync::atomic::AtomicBool>,
    acc_out: Arc<std::sync::Mutex<Option<account::SyncOut>>>,
    acc_next: std::time::Instant,
    /// Einrichten-Tab: Einmal-Links fuer neue Geraete.
    dep_pw: String,
    dep_hint_name: String,
    /// Wie viele Geraete sich mit einem Link einrichten duerfen (1-25).
    dep_count: u32,
    dep_link: String,
    dep_msg: String,
    dep_pending: Vec<account::SetupPending>,
    dep_claimed: Vec<account::SetupClaimed>,
    dep_seen: std::collections::HashSet<String>,
    dep_booted: bool,
    dep_out: Arc<std::sync::Mutex<Option<account::SetupOut>>>,
    dep_next: std::time::Instant,
    /// Einrichtungs-Dialog (freeviewer://setup/<code>).
    setup_dlg: Option<SetupDlg>,
    /// Passwort-Nachfrage vor dem Verbinden (Partner-ID).
    pw_ask: Option<String>,
    /// Sicherheits-Frage vor dem Deinstallieren.
    uninstall_ask: bool,
    /// Lizenz: Eingabefeld, Meldung und ob das Fenster auf ist (nur X-Remote).
    #[cfg(feature = "license")]
    lic_key: String,
    #[cfg(feature = "license")]
    lic_msg: String,
    #[cfg(feature = "license")]
    lic_show: bool,
    /// Einrichtungs-Auftrag aus dem Browser-Download (Code, Wunschname) -
    /// wartet auf die eigene ID, dann geht alles von selbst.
    auto_setup: Option<(String, String)>,
    /// Wann zuletzt nach dem Update-Laeuft-Merker gesehen wurde.
    upd_flag_at: std::time::Instant,
    /// Steht der Merker gerade? (fuer das Lade-Fenster)
    upd_running: bool,
    /// Tongeraete, einmal eingelesen (die Abfrage kostet Zeit).
    snd_in: Vec<String>,
    snd_out: Vec<String>,
    snd_default: (String, String),
}

impl App {
    fn new(shared: Arc<Shared>, start_hidden: bool, auto_setup: Option<(String, String)>) -> Self {
        let watch = presence::Watch::new(shared.relay_url.clone());
        watch.start();
        Self {
            shared,
            partner_id: String::new(),
            partner_pw: String::new(),
            tex: None,
            last_seq: 0,
            last_mods: egui::Modifiers::default(),
            viewer_task: None,
            hint: String::new(),
            pw_fixed: ident::has_fixed_password(),
            book: partners::Book::load(),
            remember_pw: false,
            renaming: None,
            session: None,
            hint_until: None,
            start_hidden,
            first_frame: true,
            autostart: autostart::enabled(),
            autostart_checked: std::time::Instant::now(),
            told_about_tray: false,
            service_on: service::running(),
            hide_until: None,
            view: View::Start,
            search: String::new(),
            selected: None,
            presence: watch,
            shot: None,
            meet_shot: None,
            meet_shot_n: 0,
            meet_shot_arm: 0,
            meet_shot_join: false,
            shot_warte: 6,
            look: theme::load(),
            stab: SettingsTab::General,
            fb_bug: true,
            fb_text: String::new(),
            fb_contact: String::new(),
            folder: String::new(),
            add_dev: None,
            edit_dev: None,
            caption_tick: std::time::Instant::now() - Duration::from_secs(9),
            shot_n: 0,
            pw_list: pwlist::load(),
            pw_new: String::new(),
            pw_new_label: String::new(),
            pw_show: None,
            install_service: true,
            link_check: std::time::Instant::now() - Duration::from_secs(2),
            meet_title: String::new(),
            meet_id: String::new(),
            meet_pw: String::new(),
            meet_last: None,
            meet_list: Arc::new(std::sync::Mutex::new(Vec::new())),
            meet_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            meet_err: Arc::new(std::sync::Mutex::new(String::new())),
            meet_loaded: false,
            meet_offer_control: true,
            meet_win: MeetWin::default(),
            nativ_meet: None,
            nativ_bilder: std::collections::HashMap::new(),
            nativ_schirme: std::collections::HashMap::new(),
            nativ_eigen: None,
            vorprobe: meetui::Vorprobe::default(),
            meet_z: meetfenster::Fensterzustand::default(),
            meet_gelesen: 0,
            meet_kam_start: None,
            headless: false,
            meet_demo: None,
            full: false,
            bar_x: 0.5,
            bar_w: 700.0,
            bar_pinned: true,
            bar_seen: std::time::Instant::now(),
            frame_at: std::time::Instant::now(),
            acc: account::load(),
            acc_user: String::new(),
            acc_pass: String::new(),
            acc_msg: String::new(),
            acc_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            acc_out: Arc::new(std::sync::Mutex::new(None)),
            acc_next: std::time::Instant::now(),
            dep_pw: ident::random_password(),
            dep_hint_name: String::new(),
            dep_count: 1,
            dep_link: String::new(),
            dep_msg: String::new(),
            dep_pending: Vec::new(),
            dep_claimed: Vec::new(),
            dep_seen: std::collections::HashSet::new(),
            dep_booted: false,
            dep_out: Arc::new(std::sync::Mutex::new(None)),
            dep_next: std::time::Instant::now(),
            setup_dlg: None,
            pw_ask: None,
            uninstall_ask: false,
            // Ohne gueltige Lizenz beim Start einmal danach fragen.
            #[cfg(feature = "license")]
            lic_key: String::new(),
            #[cfg(feature = "license")]
            lic_msg: String::new(),
            #[cfg(feature = "license")]
            lic_show: !matches!(license::status(), license::Status::Active(_)),
            auto_setup,
            upd_flag_at: std::time::Instant::now() - Duration::from_secs(9),
            upd_running: false,
            snd_in: Vec::new(),
            snd_out: Vec::new(),
            snd_default: (String::new(), String::new()),
        }
    }

    /// Everything around the tray icon: hide instead of quit, pick up what
    /// the menu did, keep the checkbox in sync.
    fn tray_ui(&mut self, ctx: &egui::Context) {
        if self.first_frame {
            self.first_frame = false;
            if self.start_hidden {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                tray::hide_window();
                self.hide_until =
                    Some(std::time::Instant::now() + Duration::from_secs(3));
            }
        }
        // eframe pushes the window back on screen during the first frames, so
        // insist for a moment - but only for that moment, otherwise the tray
        // menu could never open the window again.
        if let Some(until) = self.hide_until {
            if std::time::Instant::now() < until {
                if tray::is_hidden() {
                    tray::hide_window();
                }
            } else {
                self.hide_until = None;
            }
        }
        if let Some(err) = tray::take_error() {
            self.shared.set_update_status(err);
        }
        if self.autostart_checked.elapsed() > Duration::from_secs(2) {
            self.autostart_checked = std::time::Instant::now();
            self.autostart = autostart::enabled();
            self.service_on = service::running();
        }
        // The X button folds the window away instead of killing the host -
        // that is what "runs in the background" means. Quitting for real is
        // in the tray menu.
        if ctx.input(|i| i.viewport().close_requested()) && tray::is_running() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::hide_window();
            if !self.told_about_tray {
                self.told_about_tray = true;
                tray::balloon(
                    &format!("{} laeuft weiter", crate::brand::NAME),
                    "Das Fenster ist nur zugeklappt. Ueber das Symbol im Infobereich kommst du zurueck - oder beendest FreeViewer ganz.",
                );
            }
        }
        if tray::is_hidden() {
            // no picture to draw, but the menu must stay responsive
            ctx.request_repaint_after(Duration::from_millis(400));
        }
    }

    fn start_session(&mut self) {
        if self.is_me(&self.partner_id.clone()) {
            self.hint = i18n::t("start.self").to_string();
            return;
        }
        let id: String = self
            .partner_id
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        if id.len() < 9 {
            self.shared
                .set_viewer_status(i18n::t("start.bad_id"));
            return;
        }
        let sh = self.shared.clone();
        let pw = self.partner_pw.clone();
        self.tex = None;
        self.last_seq = 0;
        self.book.started(&id, &pw, self.remember_pw);
        self.session = Some((id.clone(), std::time::Instant::now()));
        self.hint_until = Some(std::time::Instant::now() + Duration::from_secs(8));
        self.shared.mode.store(proto::MODE_ADMIN, Ordering::Relaxed);
        self.viewer_task = Some(rt().spawn(async move {
            viewer::run_viewer(sh, id, pw).await;
        }));
    }

    /// Connect without a password: the other side gets a question and has to
    /// allow the session by hand.
    fn start_ask_session(&mut self) {
        if self.is_me(&self.partner_id.clone()) {
            self.hint = i18n::t("start.self").to_string();
            return;
        }
        let id: String = self
            .partner_id
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        if id.len() < 9 {
            self.shared
                .set_viewer_status(i18n::t("start.bad_id"));
            return;
        }
        let sh = self.shared.clone();
        self.tex = None;
        self.last_seq = 0;
        self.book.started(&id, "", false);
        self.session = Some((id.clone(), std::time::Instant::now()));
        self.hint_until = Some(std::time::Instant::now() + Duration::from_secs(8));
        self.shared.mode.store(proto::MODE_ADMIN, Ordering::Relaxed);
        self.viewer_task = Some(rt().spawn(async move {
            viewer::run_viewer_ask(sh, id).await;
        }));
    }

    /// Books the time of a finished session into the address book.
    fn close_session(&mut self) {
        self.full = false;
        if let Some((id, started)) = self.session.take() {
            self.book.ended(&id, started.elapsed().as_secs());
        }
        self.hint_until = None;
    }

    fn stop_session(&mut self) {
        self.close_session();
        vinput::set_active(false);
        // release modifiers that might still be held down on the remote side
        for code in [proto::KEY_SHIFT, proto::KEY_CTRL, proto::KEY_ALT] {
            self.shared.send_input(Msg::Key {
                code,
                named: true,
                down: false,
            });
        }
        if let Some(t) = self.viewer_task.take() {
            t.abort();
        }
        self.shared.connected.store(false, Ordering::Relaxed);
        self.shared.connecting.store(false, Ordering::Relaxed);
        *self.shared.input_tx.lock().unwrap() = None;
        *self.shared.frame.lock().unwrap() = None;
        self.shared.set_viewer_status("Getrennt");
        self.tex = None;
        self.last_seq = 0;
        self.last_mods = egui::Modifiers::default();
    }

    fn set_mode(&mut self, mode: u8) {
        self.shared.mode.store(mode, Ordering::Relaxed);
        self.shared.send_input(Msg::SetMode { mode });
        if mode == proto::MODE_GAME {
            vinput::set_active(true);
            self.hint =
                "Spielmodus: Maus + Tastatur werden komplett uebertragen. Rechte Strg = freigeben."
                    .to_string();
        } else {
            vinput::set_active(false);
            self.hint = "Fernwartung: scharfes Bild, absolute Maus.".to_string();
        }
    }

    /// Wartet eine freeviewer://-Adresse? Hoechstens einmal pro Sekunde
    /// nachsehen - die Oberflaeche zeichnet viel oefter.
    fn handle_link(&mut self, ctx: &egui::Context) {
        if self.link_check.elapsed() < Duration::from_millis(900) {
            return;
        }
        self.link_check = std::time::Instant::now();
        let action = match link::take() {
            Some(a) => a,
            None => return,
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        match action {
            link::Action::Control(id) => {
                self.view = View::Start;
                self.hint = i18n::tf("link.control", &partners::pretty_id(&id));
                self.connect_to(&id);
            }
            link::Action::Setup(code) => {
                // Einmal-Link vom Besitzer: diesen Rechner einrichten und
                // anschliessend richtig installieren (mit Dienst).
                self.setup_dlg = Some(SetupDlg {
                    code,
                    name: presence::machine_name(),
                    ..Default::default()
                });
            }
            link::Action::Meet { room, pass } => {
                self.view = View::Meet;
                self.meet_id = room.clone();
                self.meet_pw = pass.clone();
                // Einladungslinks landen jetzt im eigenen Meeting-Fenster
                // (Geraete waehlen, Einladung kopieren), nicht mehr sofort
                // im Browser.
                self.meet_win_open(meet::Meeting {
                    id: room.clone(),
                    titel: String::new(),
                    passwort: pass.clone(),
                    termin_text: String::new(),
                });
            }
        }
    }

    /// Files dropped onto the window go to the other side of the session.
    fn handle_drops(&mut self, ctx: &egui::Context) {
        // Solange etwas ueber dem Fenster haengt: zeigen, dass hier abgelegt
        // werden darf. Ohne das raet man, ob Ziehen und Ablegen geht.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let p = theme::palette();
            let screen = ctx.content_rect();
            let pt = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("fv_drop_hint"),
            ));
            pt.rect_filled(screen, 0.0, p.accent.gamma_multiply(0.16));
            pt.rect_stroke(
                screen.shrink(10.0),
                10.0,
                egui::Stroke::new(2.0, p.accent),
                egui::StrokeKind::Inside,
            );
            let live = self.shared.xfer.lock().unwrap().is_some();
            pt.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                if live {
                    i18n::t("sess.drop")
                } else {
                    i18n::t("sess.drop_none")
                },
                egui::FontId::proportional(19.0),
                p.text,
            );
        }
        let files: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if files.is_empty() {
            return;
        }
        let mut guard = self.shared.xfer.lock().unwrap();
        match guard.as_mut() {
            Some(x) => {
                let n = files.len();
                for p in files {
                    x.send_path(p);
                }
                self.hint = i18n::tf("sess.drop_sent", &n.to_string());
            }
            None => {
                self.hint = i18n::t("sess.drop_none").to_string();
            }
        }
    }

    fn pick_and_send(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title("Datei(en) an die Gegenstelle senden")
            .pick_files();
        if let Some(files) = files {
            let mut guard = self.shared.xfer.lock().unwrap();
            if let Some(x) = guard.as_mut() {
                for p in files {
                    x.send_path(p);
                }
            } else {
                self.hint = "Keine aktive Sitzung".to_string();
            }
        }
    }

    fn open_drop_dir(&self) {
        let dir = self.shared.drop_dir.lock().unwrap().clone();
        let _ = std::fs::create_dir_all(&dir);
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        }
    }

    /// Bottom bar with every running/finished transfer of this session.
    fn transfer_ui(&mut self, ctx: &egui::Context) {
        let list = self.shared.xfers.lock().unwrap().clone();
        if list.is_empty() {
            return;
        }
        egui::TopBottomPanel::bottom("xfer_bar").show(ctx, |ui| {
            let mut clear = false;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Dateien").strong());
                if ui.small_button("Ordner oeffnen").clicked() {
                    self.open_drop_dir();
                }
                if ui.small_button("Liste leeren").clicked() {
                    clear = true;
                }
            });
            egui::ScrollArea::vertical()
                .max_height(96.0)
                .show(ui, |ui| {
                    for p in &list {
                        ui.horizontal(|ui| {
                            ui.label(if p.incoming { "<-" } else { "->" });
                            ui.label(&p.name);
                            if !p.error.is_empty() {
                                ui.colored_label(
                                    egui::Color32::from_rgb(230, 120, 120),
                                    &p.error,
                                );
                            } else if p.finished {
                                ui.colored_label(
                                    egui::Color32::from_rgb(120, 220, 120),
                                    "fertig",
                                );
                            } else {
                                ui.add(
                                    egui::ProgressBar::new(p.percent())
                                        .desired_width(160.0)
                                        .show_percentage(),
                                );
                                ui.label(format!(
                                    "{:.1}/{:.1} MB",
                                    p.done as f32 / 1_048_576.0,
                                    p.size as f32 / 1_048_576.0
                                ));
                            }
                        });
                    }
                });
            if clear {
                self.shared
                    .xfers
                    .lock()
                    .unwrap()
                    .retain(|p| !p.finished && p.error.is_empty());
            }
        });
    }

    /// Zeile fuer die Selbstaktualisierung.
    fn update_ui(&mut self, ui: &mut egui::Ui) {
        let pending = self.shared.update.lock().unwrap().clone();
        let status = self.shared.update_status.lock().unwrap().clone();
        let dienst = service::installed();
        ui.horizontal(|ui| {
            if icons::text_button(ui, "refresh", i18n::t("upd.check"), false).clicked() {
                let sh = self.shared.clone();
                sh.set_update_status(i18n::t("upd.checking"));
                std::thread::spawn(move || match update::check() {
                    Ok(rel) => {
                        if update::newer(&rel.version, update::VERSION) {
                            sh.set_update_status(format!(
                                "{} {}",
                                i18n::t("upd.found"),
                                rel.version
                            ));
                            *sh.update.lock().unwrap() = Some(rel);
                        } else {
                            sh.set_update_status(format!(
                                "{} (v{})",
                                i18n::t("upd.current"),
                                update::VERSION
                            ));
                        }
                    }
                    Err(e) => sh.set_update_status(format!("{}: {}", i18n::t("upd.failed"), e)),
                });
            }
            let mut auto = self.shared.auto_update.load(Ordering::Relaxed);
            if check(ui, &mut auto, i18n::t("upd.auto")).changed() {
                self.shared.auto_update.store(auto, Ordering::Relaxed);
                ident::set_auto_update(auto);
            }
        });
        if let Some(rel) = pending {
            ui.add_space(4.0);
            if ghost_button(ui, &format!("Update {} installieren", rel.version)).clicked() {
                if dienst {
                    // der Dienst spielt ein - als SYSTEM, ohne Nachfrage
                    update::bitte_dienst();
                    self.shared
                        .set_update_status(i18n::t("upd.by_service"));
                } else {
                    self.shared
                        .set_update_status(format!("Installiere {} ...", rel.version));
                    let sh = self.shared.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = update::install(&rel) {
                            sh.set_update_status(format!("Update fehlgeschlagen: {}", e));
                        }
                    });
                }
            }
        }
        if !status.is_empty() {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(status).color(theme::muted()).size(11.5));
        }
    }
    fn pull_frame(&mut self, ctx: &egui::Context) {
        let img = {
            let guard = self.shared.frame.lock().unwrap();
            match guard.as_ref() {
                Some(f) if f.seq != self.last_seq => {
                    self.last_seq = f.seq;
                    Some(egui::ColorImage::from_rgba_unmultiplied(
                        [f.width as usize, f.height as usize],
                        &f.rgba,
                    ))
                }
                _ => None,
            }
        };
        if let Some(ci) = img {
            self.frame_at = std::time::Instant::now();
            match self.tex.as_mut() {
                Some(t) => t.set(ci, egui::TextureOptions::LINEAR),
                None => {
                    self.tex = Some(ctx.load_texture("remote", ci, egui::TextureOptions::LINEAR))
                }
            }
        }
    }

    // ----------------------------------------------------------- Oberflaeche

    /// Kopfzeile mit den drei Bereichen.
    // ----------------------------------------------------------- Oberfläche

    /// Kopfzeile: Marke links, die drei Bereiche als Pillen, Status rechts.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let my_id = self.shared.my_id.lock().unwrap().clone();
        let online = !my_id.is_empty();
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            logo(ui);
            ui.add_space(2.0);
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(egui::RichText::new(crate::brand::NAME).size(17.0).strong());
                ui.label(
                    egui::RichText::new(format!("Version {}", update::VERSION))
                        .size(11.0)
                        .color(theme::muted()),
                );
            });
            ui.add_space(10.0);
            for (v, name) in [
                (View::Start, i18n::t("nav.start")),
                (View::Devices, i18n::t("nav.devices")),
                (View::Settings, i18n::t("nav.settings")),
            ] {
                if tab(ui, name, self.view == v).clicked() {
                    self.view = v;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                status_pill(ui, online, if online { i18n::t("pill.ready") } else { i18n::t("pill.connecting") });
            });
        });
        ui.add_space(6.0);
        let line = ui.available_rect_before_wrap();
        ui.painter().hline(
            line.left()..=line.right(),
            line.top(),
            egui::Stroke::new(1.0, theme::line()),
        );
        ui.add_space(8.0);
    }

    fn home_ui(&mut self, ui: &mut egui::Ui) {
        match self.view {
            View::Start => self.start_view(ui),
            View::Devices => self.devices_view(ui),
            View::Meet => self.meet_view(ui),
            View::Settings => self.settings_view(ui),

        }
    }
    /// Startseite: links dieser PC, rechts der Weg nach draußen.
    /// Startseite: links dieser PC, rechts der Weg nach draußen. Bewusst
    /// wenig Text - was erklärt werden muss, steht im Tooltip.
    fn start_view(&mut self, ui: &mut egui::Ui) {
        let my_id = self.shared.my_id.lock().unwrap().clone();
        let host_status = self.shared.host_status.lock().unwrap().clone();
        let host_peer = self.shared.host_peer.lock().unwrap().clone();
        let connecting = self.shared.connecting.load(Ordering::Relaxed);
        let p = theme::palette();

        ui.columns(2, |cols| {
            // ------------------------------------------- dieser Rechner
            let ui = &mut cols[0];
            section(ui, i18n::t("start.share"));
            card(ui, |ui| {
                label_small(ui, i18n::t("start.your_id"));
                ui.horizontal(|ui| {
                    let id_text = if my_id.len() >= 9 {
                        partners::pretty_id(&my_id)
                    } else {
                        "– – –".to_string()
                    };
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(id_text).size(26.0).strong().color(p.text),
                        )
                        .selectable(true),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !my_id.is_empty()
                            && icon_ghost(ui, "copy", i18n::t("start.copy")).clicked()
                        {
                            ui.ctx().copy_text(partners::pretty_id(&my_id));
                            self.hint = i18n::t("start.copy").to_string();
                        }
                    });
                });

                ui.add_space(8.0);
                label_small(ui, i18n::t("start.password"));
                // Das Sitzungspasswort wird gewuerfelt, nicht getippt. Feste
                // Passwoerter stehen in den Einstellungen unter "Zugriff".
                let pw_now = self.shared.password.lock().unwrap().clone();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(pw_now.clone())
                                .size(20.0)
                                .family(egui::FontFamily::Monospace)
                                .color(p.text),
                        )
                        .selectable(true),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_ghost(ui, "refresh", i18n::t("start.new_pw")).clicked() {
                            *self.shared.password.lock().unwrap() = ident::random_password();
                            if self.pw_fixed {
                                let fresh = self.shared.password.lock().unwrap().clone();
                                let _ = ident::set_fixed_password(Some(fresh.as_str()));
                            }
                        }
                        if icon_ghost(ui, "copy", i18n::t("start.copy")).clicked() {
                            ui.ctx().copy_text(pw_now.clone());
                        }
                    });
                });
                ui.label(
                    egui::RichText::new(i18n::t("start.pw_random"))
                        .size(11.0)
                        .color(p.muted),
                );

                ui.add_space(8.0);
                divider(ui);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    dot(ui, !my_id.is_empty());
                    ui.label(egui::RichText::new(host_status).size(12.0).color(p.muted));
                });
                if host_peer != i18n::t("start.nosession") && !host_peer.is_empty() {
                    ui.label(egui::RichText::new(host_peer).size(12.0).color(p.muted));
                }
            });


            // ------------------------------------------------ verbinden
            let ui = &mut cols[1];
            section(ui, i18n::t("start.control"));
            card(ui, |ui| {
                label_small(ui, i18n::t("start.partner"));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.partner_id)
                            .font(egui::FontId::new(17.0, egui::FontFamily::Monospace))
                            .desired_width(190.0)
                            .margin(egui::Margin::symmetric(8, 4)),
                    );
                    let recent = self.book.sorted();
                    if !recent.is_empty() {
                        egui::ComboBox::from_id_salt("letzte_ids")
                            .selected_text(i18n::t("start.recent_short"))
                            .width(86.0)
                            .show_ui(ui, |ui| {
                                for e in recent.iter().take(8) {
                                    if ui.selectable_label(false, e.label()).clicked() {
                                        self.partner_id = e.id.clone();
                                        self.partner_pw =
                                            self.book.password(&e.id).unwrap_or_default();
                                    }
                                }
                            });
                    }
                });
                ui.add_space(6.0);
                label_small(ui, i18n::t("start.password"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.partner_pw)
                        .password(true)
                        .desired_width(190.0)
                        .margin(egui::Margin::symmetric(8, 4))
                        .hint_text(i18n::t("start.pw_hint")),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, i18n::t("start.connect"), !connecting).clicked() {
                        self.start_session();
                    }
                    if ui
                        .add_enabled(
                            !connecting,
                            ghost(egui::vec2(120.0, 30.0), i18n::t("start.ask")),
                        )
                        .on_hover_text(i18n::t("start.ask_tip"))
                        .clicked()
                    {
                        self.start_ask_session();
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let mut remember = self.remember_pw;
                    if check(ui, &mut remember, i18n::t("start.remember")).changed() {
                        self.remember_pw = remember;
                    }
                    let mut game = self.shared.game_mode();
                    if check(ui, &mut game, i18n::t("start.game"))
                        .on_hover_text(i18n::t("start.game_tip"))
                        .changed()
                    {
                        self.set_mode(if game {
                            proto::MODE_GAME
                        } else {
                            proto::MODE_ADMIN
                        });
                    }
                });
            });

            // letzte Verbindungen, kurz
            let recent = self.book.sorted();
            if !recent.is_empty() {
                ui.add_space(8.0);
                section(ui, i18n::t("start.recent"));
                let mut go: Option<String> = None;
                card(ui, |ui| {
                    for e in recent.iter().take(4) {
                        let on = self.presence.online(&e.id);
                        ui.horizontal(|ui| {
                            icons::show(ui, device_symbol(&e.label()), 15.0, row_color(on, false));
                            ui.label(
                                egui::RichText::new(e.label())
                                    .size(12.5)
                                    .color(row_color(on, true)),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_ghost(ui, "connect", i18n::t("dev.connect")).clicked() {
                                    go = Some(e.id.clone());
                                }
                                status_pill(
                                    ui,
                                    on,
                                    if on {
                                        i18n::t("st.online")
                                    } else {
                                        i18n::t("st.offline")
                                    },
                                );
                            });
                        });
                    }
                });
                if let Some(id) = go {
                    self.connect_to(&id);
                }
            }
        });

        if !self.hint.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&self.hint).color(p.accent).size(12.0));
        }
    }

    /// Mikrofon und Ton als zwei Symbolschalter - im Dashboard und in der
    /// Sitzungsleiste dieselbe Bedienung.
    fn voice_buttons(&mut self, ui: &mut egui::Ui, size: f32) {
        let v = self.shared.voice.clone();
        let mic = v.mic.load(Ordering::Relaxed);
        let snd = v.speaker.load(Ordering::Relaxed);
        if icon_toggle(
            ui,
            if mic { "mic" } else { "mic-off" },
            mic,
            size,
            if mic {
                i18n::t("sess.mic_on")
            } else {
                i18n::t("sess.mic_off")
            },
        )
        .clicked()
        {
            v.mic.store(!mic, Ordering::Relaxed);
        }
        if icon_toggle(
            ui,
            if snd { "sound" } else { "sound-off" },
            snd,
            size,
            if snd {
                i18n::t("sess.snd_on")
            } else {
                i18n::t("sess.snd_off")
            },
        )
        .clicked()
        {
            v.speaker.store(!snd, Ordering::Relaxed);
        }
    }

    /// Verbindet mit einem Eintrag aus der Liste.
    /// Ist das die eigene ID? Dann bringt eine Verbindung nichts - der
    /// Bildschirm wuerde sich selbst zeigen, bis nichts mehr geht.
    /// Wie ein Geraet in der Liste heisst. Eigener Name schlaegt alles;
    /// sonst nimmt die Liste den Namen, den der Rechner selbst beim Relay
    /// hinterlegt hat ("Flei-ONE") - eine nackte Nummer sagt niemandem etwas.
    fn dev_label(&self, e: &partners::Partner) -> String {
        if !e.name.trim().is_empty() {
            return e.name.clone();
        }
        if let Some(p) = self.presence.get(&e.id) {
            let n = p.name.trim();
            if !n.is_empty() {
                return n.to_string();
            }
        }
        partners::pretty_id(&e.id)
    }

    fn is_me(&self, id: &str) -> bool {
        let ziffern: String = id.chars().filter(|c| c.is_ascii_digit()).collect();
        let me = self.shared.my_id.lock().unwrap().clone();
        !ziffern.is_empty() && ziffern == me
    }

    fn connect_to(&mut self, id: &str) {
        if self.is_me(id) {
            self.hint = i18n::t("start.self").to_string();
            return;
        }
        self.partner_id = id.to_string();
        self.partner_pw = self.book.password(id).unwrap_or_default();
        self.selected = Some(id.to_string());
        if self.partner_pw.is_empty() {
            // Kein gespeichertes Passwort: nicht still auf "Anfrage"
            // umschalten - erst nach dem Passwort fragen. Wer keins weiss,
            // kann im Dialog immer noch eine Anfrage schicken.
            self.pw_ask = Some(id.to_string());
        } else {
            self.start_session();
        }
    }

    /// Schmale Symbolspalte links - die drei Bereiche des Programms.
    fn rail(&mut self, ctx: &egui::Context) {
        let p = theme::palette();
        // Frueher war die Leiste im hellen Modus komplett im Akzent gefaerbt -
        // die weissen Symbole sind darin verschwunden. Jetzt ist sie immer
        // eine ruhige Flaeche, gefaerbt wird nur, was gerade gewaehlt ist.
        let rail_bg = p.card;
        egui::SidePanel::left("fv_rail")
            .exact_width(56.0)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(rail_bg)
                    .inner_margin(egui::Margin::symmetric(0, 10)),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    logo(ui);
                    ui.add_space(10.0);
                    for (v, name, tip) in [
                        (View::Start, "home", i18n::t("nav.start")),
                        (View::Devices, "devices", i18n::t("nav.devices")),
                        (View::Meet, "meet", i18n::t("nav.meet")),
                        (View::Settings, "settings", i18n::t("nav.settings")),
                    ] {
                        let sel = self.view == v;
                        // Erst die Flaeche belegen, dann selbst zeichnen: nur so
                        // reagiert JEDER Knopf auf die Maus und nicht bloss der,
                        // auf dem man gerade steht.
                        let (rect, r) =
                            ui.allocate_exact_size(egui::vec2(38.0, 34.0), egui::Sense::click());
                        let hov = r.hovered();
                        let fill = if sel {
                            p.accent.gamma_multiply(if p.dark { 0.20 } else { 0.14 })
                        } else if hov {
                            p.card_hi
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        if fill != egui::Color32::TRANSPARENT {
                            ui.painter().rect_filled(rect, 9.0, fill);
                        }
                        if sel {
                            // schmaler Balken links: zeigt die Seite auch ohne Farbe
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(
                                    egui::pos2(rect.min.x - 7.0, rect.center().y - 9.0),
                                    egui::vec2(3.0, 18.0),
                                ),
                                2.0,
                                p.accent,
                            );
                        }
                        let fg = if sel {
                            p.accent
                        } else if hov {
                            p.text
                        } else {
                            p.muted
                        };
                        ui.put(rect, icons::image(name, 19.0, fg));
                        let r = zeigefinger(r).on_hover_text(tip);
                        if r.clicked() {
                            self.view = v;
                        }
                        ui.add_space(4.0);
                    }
                });
            });
    }

    /// Konferenzen. Das Verzeichnis liegt auf dem Meet-Server, darum laufen
    /// alle Abfragen in einem eigenen Faden - eine haengende Leitung darf die
    /// Oberflaeche nicht einfrieren.
    fn meet_refresh(&mut self) {
        if self.meet_busy.load(Ordering::Relaxed) {
            return;
        }
        self.meet_busy.store(true, Ordering::Relaxed);
        let list = self.meet_list.clone();
        let err = self.meet_err.clone();
        let busy = self.meet_busy.clone();
        std::thread::spawn(move || {
            match meet::list() {
                Ok(v) => {
                    *list.lock().unwrap() = v;
                    err.lock().unwrap().clear();
                }
                Err(e) => *err.lock().unwrap() = format!("{}", e),
            }
            busy.store(false, Ordering::Relaxed);
        });
    }

    fn meet_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        if !self.meet_loaded {
            self.meet_loaded = true;
            self.meet_refresh();
        }

        ui.columns(2, |cols| {
            // ---------------------------------------------- neues Meeting
            let ui = &mut cols[0];
            section(ui, i18n::t("meet.new"));
            card(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.meet_title)
                        .desired_width(240.0)
                        .hint_text(i18n::t("meet.title_hint"))
                        .margin(egui::Margin::symmetric(8, 5)),
                );
                ui.add_space(8.0);
                if ghost_button(ui, i18n::t("meet.start")).clicked() {
                    match meet::create(&self.meet_title) {
                        Ok(m) => {
                            self.meet_id = m.id.clone();
                            self.meet_pw = m.passwort.clone();
                            self.meet_last = Some(m.clone());
                            // Sofort ins eigene Meeting-Fenster wechseln.
                            self.meet_win_open(m);
                            self.hint = i18n::t("meet.created").to_string();
                            self.meet_loaded = false;
                        }
                        Err(e) => self.hint = format!("{}: {}", i18n::t("meet.err"), e),
                    }
                }
                if let Some(m) = self.meet_last.clone() {
                    ui.add_space(10.0);
                    divider(ui);
                    ui.add_space(8.0);
                    label_small(ui, i18n::t("meet.id"));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&m.id)
                                .size(24.0)
                                .family(egui::FontFamily::Monospace)
                                .color(p.text),
                        )
                        .selectable(true),
                    );
                    ui.add_space(4.0);
                    label_small(ui, i18n::t("meet.pw"));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&m.passwort)
                                .size(18.0)
                                .family(egui::FontFamily::Monospace)
                                .color(p.text),
                        )
                        .selectable(true),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ghost_button(ui, i18n::t("meet.join")).clicked() {
                            self.meet_win_open(m.clone());
                        }
                        ui.add_space(6.0);
                        if ghost_button(ui, i18n::t("meet.copy_invite")).clicked() {
                            ui.ctx().copy_text(meet::invite(&m));
                            self.hint = i18n::t("meet.copied").to_string();
                        }
                    });
                }
            });

            // ---------------------------------------------- beitreten
            let ui = &mut cols[1];
            section(ui, i18n::t("meet.join_head"));
            card(ui, |ui| {
                label_small(ui, i18n::t("meet.id"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.meet_id)
                        .desired_width(180.0)
                        .hint_text("123-456-789")
                        .font(egui::FontId::new(16.0, egui::FontFamily::Monospace))
                        .margin(egui::Margin::symmetric(8, 5)),
                );
                ui.add_space(6.0);
                label_small(ui, i18n::t("meet.pw"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.meet_pw)
                        .desired_width(180.0)
                        .password(true)
                        .margin(egui::Margin::symmetric(8, 5)),
                );
                ui.add_space(10.0);
                check(ui, &mut self.meet_offer_control, i18n::t("meet.offer"))
                    .on_hover_text(i18n::t("meet.offer_tip"));
                ui.add_space(8.0);
                if ghost_button(ui, i18n::t("meet.join")).clicked() {
                    let id = self.meet_id.trim().to_string();
                    if id.is_empty() {
                        self.hint = i18n::t("meet.need_id").to_string();
                    } else {
                        self.meet_win_open(meet::Meeting {
                            id,
                            titel: String::new(),
                            passwort: self.meet_pw.clone(),
                            termin_text: String::new(),
                        });
                    }
                }
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(i18n::t("meet.note"))
                        .size(11.5)
                        .color(p.muted),
                );
            });

            ui.add_space(12.0);
            section(ui, i18n::t("meet.running"));
            card(ui, |ui| {
                let err = self.meet_err.lock().unwrap().clone();
                let list = self.meet_list.lock().unwrap().clone();
                if !err.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("{}: {}", i18n::t("meet.err"), err))
                            .size(12.0)
                            .color(p.violet),
                    );
                } else if list.is_empty() {
                    ui.label(
                        egui::RichText::new(i18n::t("meet.none"))
                            .size(12.0)
                            .color(p.muted),
                    );
                }
                for m in list.iter() {
                    ui.horizontal(|ui| {
                        dot(ui, true);
                        ui.label(
                            egui::RichText::new(&m.id)
                                .size(13.0)
                                .family(egui::FontFamily::Monospace)
                                .color(p.text),
                        );
                        if !m.titel.is_empty() {
                            ui.label(egui::RichText::new(&m.titel).size(12.0).color(p.muted));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_ghost(ui, "connect", i18n::t("meet.join")).clicked() {
                                self.meet_id = m.id.clone();
                            }
                        });
                    });
                    ui.add_space(3.0);
                }
                ui.add_space(6.0);
                if ghost_button(ui, i18n::t("meet.refresh")).clicked() {
                    self.meet_loaded = false;
                }
            });
        });
    }

    /// Oeffnet das eigene Meeting-Fenster fuer dieses Meeting (Zoom-Ablauf:
    /// erst Geraete pruefen und Einladung kopieren, dann beitreten).
    fn meet_win_open(&mut self, m: meet::Meeting) {
        self.meet_win.offen = true;
        if self.meet_win.name.trim().is_empty() {
            // Vorschlag: der Rechnername - so heisst man im Browser auch.
            self.meet_win.name = presence::device_name();
        }
        self.meet_win.beigetreten = false;
        self.meet_win.toast = None;
        self.meet_win.meeting = Some(m);
        self.meet_win.tn.lock().unwrap().clear();
        // Gleich einmal nachsehen, wer schon da ist.
        self.meet_win.tn_next = std::time::Instant::now();
        if !self.meet_win.geraete_geladen {
            self.meet_win.geraete_geladen = true;
            let slot = self.meet_win.geraete.clone();
            std::thread::spawn(move || {
                let (mics, mut cams) = meet::geraete();
                // Fuer die Auswahl IM Meeting zaehlen die Namen, die der
                // Kamera-Oeffner selbst kennt (Media Foundation). Die
                // PnP-Liste ist nur ein Rueckfall, wenn die leer bleibt.
                let echte = crate::meetcam::liste();
                if !echte.is_empty() {
                    cams = echte.into_iter().map(|g| g.name).collect();
                }
                let (_, spks) = crate::meetaudio::geraete_liste();
                *slot.lock().unwrap() = Some((mics, cams, spks));
            });
        }
    }

    /// Teilnehmerliste nachladen - alle 5 s, in einem Hintergrundfaden,
    /// damit eine haengende Leitung das Fenster nicht einfriert.
    fn meet_win_poll(&mut self) {
        if !self.meet_win.offen
            || self.meet_win.tn_busy.load(Ordering::Relaxed)
            || std::time::Instant::now() < self.meet_win.tn_next
        {
            return;
        }
        let (id, pass) = match self.meet_win.meeting.as_ref() {
            Some(m) => (m.id.clone(), m.passwort.clone()),
            None => return,
        };
        if pass.is_empty() {
            // Ohne Passwort verraet der Server die Liste nicht.
            return;
        }
        self.meet_win.tn_busy.store(true, Ordering::Relaxed);
        self.meet_win.tn_next = std::time::Instant::now() + Duration::from_secs(5);
        let tn = self.meet_win.tn.clone();
        let busy = self.meet_win.tn_busy.clone();
        std::thread::spawn(move || {
            if let Ok(v) = meet::teilnehmer(&id, &pass) {
                *tn.lock().unwrap() = v;
            }
            busy.store(false, Ordering::Relaxed);
        });
    }

    /// Das eigene Meeting-Fenster - so wie der FreeMeet-Browser-Client.
    ///
    /// Vor dem Beitritt steht die mittige Karte mit lebender Selbstvorschau,
    /// danach das Meeting: Marken oben, Buehne in der Mitte, Seitenleiste
    /// rechts, Steuerleiste unten. GEZEICHNET wird in `meetfenster` - hier
    /// wird nur zusammengetragen, was es braucht, und ausgefuehrt, was der
    /// Nutzer angeklickt hat. Diese Trennung ist der Grund, warum sich jede
    /// Ansicht in `--uitest` ohne Server, Kamera und Soundkarte durchrendern
    /// laesst.
    /// Meetingfenster zu = Meeting verlassen.
    ///
    /// WARUM nicht nur `offen = false`: das Fenster haelt echte Geraete.
    /// Vorher lief nach dem Schliessen die Kamera weiter (Leuchte an) und
    /// der Server hielt uns weiter fuer anwesend - genau das, was Justin
    /// gemeldet hat. Das Kreuz muss dasselbe tun wie "Verlassen".
    fn meet_win_schliessen(&mut self) {
        // Die Vorschau vor dem Beitritt haelt Kamera UND Mikrofon.
        self.vorprobe.aus();
        if let Some(n) = self.nativ_meet.take() {
            n.verlassen();
        }
        self.nativ_bilder.clear();
        self.nativ_schirme.clear();
        self.nativ_eigen = None;
        self.meet_win.beigetreten = false;
        self.meet_win.nativ_start = false;
        self.meet_kam_start = None;
        // Auch das kleine Fenster muss zu, sonst bleibt es allein stehen.
        self.meet_z = meetfenster::Fensterzustand::default();
        self.meet_win.offen = false;
    }

    fn meet_win_ui(&mut self, ctx: &egui::Context) {
        // Geraeteliste uebernehmen, sobald der Hintergrundfaden fertig ist.
        if !self.meet_win.geraete_da {
            if let Some((mics, cams, spks)) = self.meet_win.geraete.lock().unwrap().clone() {
                self.meet_win.mics = mics;
                self.meet_win.cams = cams;
                self.meet_win.spks = spks;
                self.meet_win.geraete_da = true;
            }
        }
        self.meet_win_poll();
        let m = match self.meet_win.meeting.clone() {
            Some(m) => m,
            None => return,
        };
        if self.nativ_meet.is_none() {
            self.meet_beitritt_ui(ctx, &m);
        } else {
            self.meet_lauf_ui(ctx, &m);
        }
        self.meet_toast(ctx);
    }

    /// Der Schirm vor dem Beitritt (`#vorbereiten` im Browser).
    fn meet_beitritt_ui(&mut self, ctx: &egui::Context, m: &meet::Meeting) {
        // Kamera und Mikrofon der Vorschau dem Wunsch nachfuehren. Nur bei
        // ECHTER Aenderung anfassen - sonst wuerde ein Geraetefehler jedes
        // Bild einen neuen Oeffnungsversuch ausloesen.
        let will_kamera = !self.meet_win.ohne_video;
        if will_kamera != self.vorprobe.wunsch_kamera {
            self.vorprobe.kamera(will_kamera);
        }
        let mic = if self.meet_win.mic_sel == 0 {
            None
        } else {
            self.meet_win.mics.get(self.meet_win.mic_sel - 1).cloned()
        };
        let will_mikro = !self.meet_win.stumm;
        if will_mikro != self.vorprobe.wunsch_mikro || mic != self.vorprobe.mikro_geraet {
            self.vorprobe.mikro(will_mikro, mic.clone());
        }
        self.vorprobe.pumpe();

        // Vorschaubild in eine Textur giessen (nur wenn es ein neues gibt).
        match self.vorprobe.eigen.as_ref() {
            Some((w, h, px)) => {
                let stand = self.vorprobe.stand;
                let frisch = self
                    .nativ_eigen
                    .as_ref()
                    .map(|(alt, _)| *alt != stand)
                    .unwrap_or(true);
                if frisch {
                    let bild =
                        egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], px);
                    let tex = ctx.load_texture("meeteigen", bild, egui::TextureOptions::LINEAR);
                    self.nativ_eigen = Some((stand, tex));
                }
            }
            None => self.nativ_eigen = None,
        }

        let mut b = meetfenster::Beitritt {
            raum: m.id.clone(),
            titel: m.titel.clone(),
            name: self.meet_win.name.clone(),
            mics: self.meet_win.mics.clone(),
            cams: self.meet_win.cams.clone(),
            mic_sel: self.meet_win.mic_sel,
            cam_sel: self.meet_win.cam_sel,
            mikro_an: will_mikro,
            kamera_an: will_kamera,
            geraete_da: self.meet_win.geraete_da,
            pegel: self.vorprobe.pegel,
            hinweis: self.vorprobe.meldung.clone(),
            laeuft: self.meet_win.nativ_start,
        };
        let tex = self.nativ_eigen.as_ref().map(|(_, t)| t.clone());
        let aktion = meetfenster::beitritt_ui(ctx, &mut b, tex.as_ref());
        self.meet_win.name = b.name;
        self.meet_win.mic_sel = b.mic_sel;
        self.meet_win.cam_sel = b.cam_sel;

        match aktion {
            Some(meetfenster::Beitrittsaktion::MikroUm) => {
                self.meet_win.stumm = !self.meet_win.stumm;
            }
            Some(meetfenster::Beitrittsaktion::KameraUm) => {
                self.meet_win.ohne_video = !self.meet_win.ohne_video;
            }
            Some(meetfenster::Beitrittsaktion::Zurueck) => {
                self.meet_win_schliessen();
            }
            Some(meetfenster::Beitrittsaktion::Beitreten) => {
                self.meet_win.nativ_start = true;
            }
            None => {}
        }

        // Beitreten passiert ERST nachdem der Rahmen gezeichnet ist: die
        // Vorschau muss die Kamera vorher loslassen, sonst bekommt das
        // Meeting sie nicht.
        if std::mem::take(&mut self.meet_win.nativ_start) {
            self.vorprobe.aus();
            self.nativ_eigen = None;
            let me = self.shared.my_id.lock().unwrap().clone();
            let name = if self.meet_win.name.trim().is_empty() {
                presence::device_name()
            } else {
                self.meet_win.name.trim().to_string()
            };
            let mic = if self.meet_win.mic_sel == 0 {
                None
            } else {
                self.meet_win.mics.get(self.meet_win.mic_sel - 1).cloned()
            };
            match meetui::NativMeet::beitreten(
                &meet::base(),
                &m.id,
                &m.passwort,
                &name,
                &me,
                self.meet_offer_control,
                mic,
                None,
            ) {
                Ok(mut n) => {
                    if self.meet_win.stumm {
                        n.stumm_schalten(true);
                    }
                    self.meet_kam_start = if self.meet_win.ohne_video {
                        None
                    } else {
                        Some(std::time::Instant::now() + std::time::Duration::from_millis(600))
                    };
                    self.nativ_meet = Some(n);
                    self.meet_win.beigetreten = true;
                    self.meet_gelesen = 0;
                    self.meet_z = meetfenster::Fensterzustand::default();
                    self.meet_win.tn_next = std::time::Instant::now();
                }
                Err(e) => {
                    self.meet_win.toast = Some((format!("{}", e), std::time::Instant::now()));
                }
            }
        }
    }

    /// Das laufende Meeting.
    fn meet_lauf_ui(&mut self, ctx: &egui::Context, m: &meet::Meeting) {
        let sicht: meetfenster::Sicht;
        let bilder: meetfenster::Bilder;
        {
            let n = match self.nativ_meet.as_mut() {
                Some(n) => n,
                None => return,
            };
            n.pumpe();
            // Kamera erst jetzt anwerfen - die Vorschau hat sie gerade
            // losgelassen und braucht dafuer einen Augenblick.
            if let Some(wann) = self.meet_kam_start {
                if std::time::Instant::now() >= wann {
                    self.meet_kam_start = None;
                    n.kamera_schalten(true);
                }
            }
            let z = n.zustand();
            // Nur beim Bildermachen (--meetshot): Wartende gleich hereinlassen.
            // Sonst saehe man auf dem Beweisbild nie einen zweiten Teilnehmer.
            if self.meet_shot_join && !z.wartende.is_empty() {
                n.alle_einlassen();
            }
            let zahlen = n.zahlen();

            // ---- Texturen nachladen (nur bei wirklich neuem Bild)
            match n.eigen.as_ref() {
                Some((w, h, px)) => {
                    let frisch = self
                        .nativ_eigen
                        .as_ref()
                        .map(|(alt, _)| *alt != n.eigen_stand)
                        .unwrap_or(true);
                    if frisch {
                        let bild = egui::ColorImage::from_rgba_unmultiplied(
                            [*w as usize, *h as usize],
                            px,
                        );
                        let tex = ctx.load_texture("meeteigen", bild, egui::TextureOptions::LINEAR);
                        self.nativ_eigen = Some((n.eigen_stand, tex));
                    }
                }
                None => self.nativ_eigen = None,
            }
            for (peer, (w, h, px)) in n.bilder.bilder.iter() {
                let stand = *n.bilder.stand.get(peer).unwrap_or(&0);
                let frisch = self
                    .nativ_bilder
                    .get(peer)
                    .map(|(alt, _)| *alt != stand)
                    .unwrap_or(true);
                if frisch {
                    let bild =
                        egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], px);
                    let tex =
                        ctx.load_texture(format!("meetcam{}", peer), bild, egui::TextureOptions::LINEAR);
                    self.nativ_bilder.insert(*peer, (stand, tex));
                }
            }
            self.nativ_bilder
                .retain(|k, _| n.bilder.bilder.contains_key(k));
            for (peer, (w, h, px)) in n.schirme.bilder.iter() {
                let stand = *n.schirme.stand.get(peer).unwrap_or(&0);
                let frisch = self
                    .nativ_schirme
                    .get(peer)
                    .map(|(alt, _)| *alt != stand)
                    .unwrap_or(true);
                if frisch {
                    let bild =
                        egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], px);
                    let tex = ctx.load_texture(
                        format!("meetschirm{}", peer),
                        bild,
                        egui::TextureOptions::LINEAR,
                    );
                    self.nativ_schirme.insert(*peer, (stand, tex));
                }
            }
            self.nativ_schirme
                .retain(|k, _| n.schirme.bilder.contains_key(k));

            // ---- Sicht zusammenstellen
            let ich_name = if self.meet_win.name.trim().is_empty() {
                presence::device_name()
            } else {
                self.meet_win.name.trim().to_string()
            };
            let mut leute = vec![meetfenster::Person {
                id: z.ich,
                name: ich_name,
                stumm: n.stumm,
                kamera_aus: !n.kamera_an,
                hand: n.hand,
                gastgeber: z.gastgeber != 0 && z.gastgeber == z.ich,
                fvid: String::new(),
                ich: true,
                spricht: !n.stumm && n.pegel > 0.06,
            }];
            for t in z.leute.iter() {
                leute.push(meetfenster::Person {
                    id: t.id,
                    name: t.name.clone(),
                    stumm: t.ton_aus,
                    kamera_aus: t.bild_aus,
                    hand: t.hand,
                    gastgeber: z.gastgeber != 0 && z.gastgeber == t.id,
                    fvid: t.fvid.clone(),
                    ich: false,
                    spricht: n.spricht(t.id),
                });
            }
            let chat: Vec<meetfenster::Chatzeile> = n
                .chat
                .iter()
                .map(|(von, text)| meetfenster::Chatzeile {
                    von: *von,
                    name: if *von == 0 {
                        String::new()
                    } else {
                        n.name_von(*von)
                    },
                    text: text.clone(),
                    eigen: *von == z.ich,
                })
                .collect();
            // Chat gilt als gelesen, solange die Seitenleiste ihn zeigt.
            if self.meet_z.seite_offen && self.meet_z.reiter == meetfenster::Reiter::Chat {
                self.meet_gelesen = chat.len();
            }
            let ungelesen = chat.len().saturating_sub(self.meet_gelesen) as u32;

            let mut protokoll = n.protokoll.clone();
            for meldung in [&n.meldung, &n.kamera_meldung, &n.schirm_meldung] {
                if !meldung.is_empty() {
                    protokoll.push(meldung.clone());
                }
            }
            protokoll.push(format!(
                "Ton: {} raus / {} rein · Bild: {} raus / {} rein",
                zahlen.gesendet, zahlen.empfangen, zahlen.bild_gesendet, zahlen.bild_empfangen
            ));
            if n.schirm_an {
                protokoll.push(format!(
                    "Bildschirm geteilt: {} Bilder gesendet",
                    n.schirm_gesendet
                ));
            }
            protokoll.push(format!("Meine Nummer: {}", n.meine_fvid()));

            let bandbreite = if n.kbit_rein + n.kbit_raus > 0 {
                format!("{} kbit/s", n.kbit_rein + n.kbit_raus)
            } else {
                String::new()
            };
            let tippen: Vec<String> = n.tippen.iter().map(|id| n.name_von(*id)).collect();
            // Der Servertext zum Warteraum steht in der letzten Meldung.
            let warte_text = if z.im_warteraum { n.meldung.clone() } else { String::new() };
            let schirme: Vec<(u64, String)> = self
                .nativ_schirme
                .keys()
                .map(|id| (*id, n.name_von(*id)))
                .collect();

            let ton_namen = n.ton_namen();
            sicht = meetfenster::Sicht {
                raum: if z.raum.is_empty() { m.id.clone() } else { z.raum.clone() },
                titel: if z.titel.is_empty() { m.titel.clone() } else { z.titel.clone() },
                gastgeber: n.bin_gastgeber(),
                verbindung: if zahlen.verbunden {
                    "direkt verbunden".to_string()
                } else if z.verbunden {
                    "Medien verbinden …".to_string()
                } else {
                    "verbinde …".to_string()
                },
                verbindung_ton: if zahlen.verbunden {
                    meetfenster::Ton::Gut
                } else {
                    meetfenster::Ton::Warn
                },
                // Nativ laeuft noch KEINE Ende-zu-Ende-Schicht (meetsig meldet
                // caps.e2e=false). Das ehrlich anzeigen statt zu schoenen.
                e2e: false,
                bandbreite,
                leute,
                wartende: z.wartende.clone(),
                warteraum_an: z.warteraum_an,
                chat,
                protokoll,
                stumm: n.stumm,
                kamera_an: n.kamera_an,
                schirm_an: n.schirm_an,
                hand: n.hand,
                steuer_frei: n.steuer_frei,
                schirme,
                ungelesen,
                tippen,
                im_warteraum: z.im_warteraum,
                warte_text: warte_text.clone(),
                cams: self.meet_win.cams.clone(),
                mics: self.meet_win.mics.clone(),
                spks: self.meet_win.spks.clone(),
                cam_sel: self.meet_win.cam_sel,
                mic_sel: self.meet_win.mic_sel,
                spk_sel: self.meet_win.spk_sel,
                kamera_name: n.kamera_name(),
                ton_ein: ton_namen.0,
                ton_aus: ton_namen.1,
            };
            bilder = meetfenster::Bilder {
                eigen: self.nativ_eigen.as_ref().map(|(_, t)| t.clone()),
                kameras: self
                    .nativ_bilder
                    .iter()
                    .map(|(k, (_, t))| (*k, t.clone()))
                    .collect(),
                schirme: self
                    .nativ_schirme
                    .iter()
                    .map(|(k, (_, t))| (*k, t.clone()))
                    .collect(),
            };
        }

        // Vor der Tuer: der Warteschirm statt eines leeren Raums. Ohne ihn
        // saesse der Gast vor einer schwarzen Flaeche und haelt das Programm
        // fuer kaputt - genau der Fall, der bei der Probe aufgefallen ist.
        if sicht.im_warteraum {
            if meetfenster::warteschirm_ui(ctx, &sicht.titel, &sicht.warte_text) {
                if let Some(n) = self.nativ_meet.take() {
                    n.verlassen();
                }
                self.meet_win.beigetreten = false;
                self.meet_kam_start = None;
            }
            return;
        }
        let mut aktionen = meetfenster::meeting_ui(ctx, &sicht, &mut self.meet_z, &bilder);
        // Bild im Bild: eigenes kleines Fenster, das oben bleibt - sonst
        // sieht man beim Teilen des eigenen Bildschirms niemanden mehr.
        if self.meet_z.pip && !self.headless {
            let mut zu = false;
            // Der Schalter liegt im Fensterzustand, die Schliesse aber im
            // Viewport-Rueckruf - deshalb hier eine Kopie, die danach
            // zurueckgeschrieben wird (sonst liehe sich `self` doppelt aus).
            let mut selbst = self.meet_z.pip_selbst;
            let mut pip_aktionen: Vec<meetfenster::Aktion> = Vec::new();
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("fv_meet_pip"),
                egui::ViewportBuilder::default()
                    .with_title("Meeting")
                    .with_inner_size([260.0, 330.0])
                    .with_always_on_top(),
                |pctx, _class| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(theme::bg()))
                        .show(pctx, |ui| {
                            pip_aktionen
                                .extend(meetfenster::pip_inhalt(ui, &sicht, &bilder, &mut selbst));
                        });
                    if pctx.input(|i| i.viewport().close_requested()) {
                        zu = true;
                    }
                    pctx.request_repaint_after(Duration::from_millis(100));
                },
            );
            self.meet_z.pip_selbst = selbst;
            aktionen.extend(pip_aktionen);
            if zu {
                self.meet_z.pip = false;
            }
        }

        let mut steuern_zu: Option<(String, String)> = None;
        let mut verlassen = false;
        for a in aktionen {
            let n = match self.nativ_meet.as_mut() {
                Some(n) => n,
                None => break,
            };
            match a {
                meetfenster::Aktion::Stumm(v) => n.stumm_schalten(v),
                meetfenster::Aktion::Kamera(v) => n.kamera_schalten(v),
                meetfenster::Aktion::Schirm(v) => n.schirm_schalten(v, 0),
                meetfenster::Aktion::Hand(v) => n.hand_heben(v),
                meetfenster::Aktion::Steuerung(v) => n.steuerung_freigeben(v),
                meetfenster::Aktion::Senden(t) => {
                    n.eingabe = t;
                    n.senden();
                }
                meetfenster::Aktion::Tippt(v) => n.tippt(v),
                meetfenster::Aktion::Einlassen(id) => n.einlassen(id),
                meetfenster::Aktion::Abweisen(id) => n.abweisen(id),
                meetfenster::Aktion::AlleEinlassen => n.alle_einlassen(),
                meetfenster::Aktion::Warteraum(v) => n.warteraum(v),
                meetfenster::Aktion::AlleStumm => n.gastgeber_aktion("mute-all", None),
                meetfenster::Aktion::Stummschalten(id) => n.gastgeber_aktion("mute", Some(id)),
                meetfenster::Aktion::Rauswerfen(id) => n.gastgeber_aktion("kick", Some(id)),
                meetfenster::Aktion::Beenden => {
                    n.gastgeber_aktion("end", None);
                    verlassen = true;
                }
                meetfenster::Aktion::Verlassen => verlassen = true,
                meetfenster::Aktion::Steuern(fvid, wer) => steuern_zu = Some((fvid, wer)),
                meetfenster::Aktion::KameraGeraet(i) => {
                    self.meet_win.cam_sel = i;
                    let g = if i == 0 {
                        None
                    } else {
                        self.meet_win.cams.get(i - 1).cloned()
                    };
                    n.kamera_waehlen(g);
                }
                meetfenster::Aktion::MikroGeraet(i) => {
                    self.meet_win.mic_sel = i;
                    let mik = if i == 0 {
                        None
                    } else {
                        self.meet_win.mics.get(i - 1).cloned()
                    };
                    let spk = if self.meet_win.spk_sel == 0 {
                        None
                    } else {
                        self.meet_win.spks.get(self.meet_win.spk_sel - 1).cloned()
                    };
                    n.ton_waehlen(mik, spk);
                }
                meetfenster::Aktion::LautsprecherGeraet(i) => {
                    self.meet_win.spk_sel = i;
                    let spk = if i == 0 {
                        None
                    } else {
                        self.meet_win.spks.get(i - 1).cloned()
                    };
                    let mik = if self.meet_win.mic_sel == 0 {
                        None
                    } else {
                        self.meet_win.mics.get(self.meet_win.mic_sel - 1).cloned()
                    };
                    n.ton_waehlen(mik, spk);
                }
                meetfenster::Aktion::GeraeteNeuLesen => {
                    // Neu einlesen laeuft wieder im Hintergrund - die
                    // PnP-Abfrage dauert unter Windows fast eine Sekunde.
                    self.meet_win.geraete_da = false;
                    *self.meet_win.geraete.lock().unwrap() = None;
                    let slot = self.meet_win.geraete.clone();
                    std::thread::spawn(move || {
                        let (mics, mut cams) = meet::geraete();
                        let echte = crate::meetcam::liste();
                        if !echte.is_empty() {
                            cams = echte.into_iter().map(|g| g.name).collect();
                        }
                        let (_, spks) = crate::meetaudio::geraete_liste();
                        *slot.lock().unwrap() = Some((mics, cams, spks));
                    });
                }
                meetfenster::Aktion::EinladungKopieren => {
                    ctx.copy_text(meet::invite(m));
                    self.meet_win.toast = Some((
                        i18n::t("meet.toast_copied").to_string(),
                        std::time::Instant::now(),
                    ));
                }
            }
        }
        if let Some((fvid, wer)) = steuern_zu {
            // Aus dem Meeting heraus direkt in die Sitzung: Hauptfenster nach
            // vorn, Start-Seite, verbinden.
            self.view = View::Start;
            self.hint = i18n::tf("link.control", &partners::pretty_id(&fvid));
            self.meet_win.toast = Some((format!("Steuere {}", wer), std::time::Instant::now()));
            self.connect_to(&fvid);
        }
        if verlassen {
            if let Some(n) = self.nativ_meet.take() {
                n.verlassen();
            }
            self.nativ_bilder.clear();
            self.nativ_schirme.clear();
            self.nativ_eigen = None;
            self.meet_win.beigetreten = false;
            self.meet_z.pip = false;
            self.meet_kam_start = None;
        }
    }

    /// Bild der eigenen Oberflaeche machen und beenden (--shot).
    fn shot_ui(&mut self, ctx: &egui::Context) {
if let Some(path) = self.shot.clone() {
            self.shot_n += 1;
            // Sobald die Geraeteliste da ist, den Beitritt ausloesen.
            if self.meet_shot_join && self.meet_shot_arm == 0 && self.meet_win.geraete_da {
                self.meet_shot_arm = self.shot_n;
                self.meet_win.nativ_start = true;
            }
            if self.shot_n == self.shot_warte {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::UserData::default(),
                ));
            }
            if self.shot_n > self.shot_warte {
                let img = ctx.input(|i| {
                    i.events.iter().rev().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(img) = img {
                    save_shot(&img, &path);
                    std::process::exit(0);
                }
                if self.shot_n > self.shot_warte + 150 {
                    eprintln!("SHOT: kein Bild bekommen");
                    std::process::exit(2);
                }
            }
            ctx.request_repaint();
        }
    }

    /// Nur fuer Bilder: einen Beispielzustand des Meetings zeichnen
    /// (`--meetdemo N --shot datei.jpg`). Ohne diesen Weg gaebe es nie ein
    /// echtes Bild der Buehne - der Bauserver hat weder Kamera noch zweiten
    /// Teilnehmer.
    fn meet_demo_ui(&mut self, ctx: &egui::Context, nr: usize) {
        // BEISPIELE+1: der Warteschirm.
        if nr == meetfenster::BEISPIELE + 1 {
            meetfenster::warteschirm_ui(
                ctx,
                "Wochenbesprechung",
                "Der Gastgeber wurde benachrichtigt.",
            );
            return;
        }
        // Ab BEISPIELE: der Beitritts-Schirm.
        if nr >= meetfenster::BEISPIELE {
            let bilder = meetfenster::pruefbilder(ctx);
            let mut b = meetfenster::beispiel_beitritt();
            meetfenster::beitritt_ui(ctx, &mut b, bilder.eigen.as_ref());
            return;
        }
        let bilder = meetfenster::pruefbilder(ctx);
        let (_name, sicht, mut z) = meetfenster::beispiel(nr);
        meetfenster::meeting_ui(ctx, &sicht, &mut z, &bilder);
    }

    /// Kurzer, sichtbarer Hinweis ("Link kopiert").
    fn meet_toast(&mut self, ctx: &egui::Context) {
        let p = theme::palette();
        if let Some((text, seit)) = self.meet_win.toast.clone() {
            let alt = seit.elapsed().as_secs_f32();
            if alt > 2.4 {
                self.meet_win.toast = None;
                return;
            }
            let deck = if alt > 1.8 {
                ((2.4 - alt) / 0.6).clamp(0.0, 1.0)
            } else {
                1.0
            };
            egui::Area::new(egui::Id::new("meet_toast"))
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -90.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(p.accent.gamma_multiply(deck))
                        .corner_radius(egui::CornerRadius::same(14))
                        .inner_margin(egui::Margin::symmetric(16, 8))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&text)
                                    .size(13.0)
                                    .color(p.on_accent.gamma_multiply(deck)),
                            );
                        });
                });
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    /// Kopfzeile: Seitenname, Suche, eigener Zustand.
    fn header(&mut self, ctx: &egui::Context) {
        let p = theme::palette();
        let my_id = self.shared.my_id.lock().unwrap().clone();
        let online = !my_id.is_empty();
        egui::TopBottomPanel::top("fv_header")
            .exact_height(48.0)
            .frame(
                egui::Frame::NONE
                    .fill(p.card)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let title = match self.view {
                        View::Start => i18n::t("nav.start"),
                        View::Devices => i18n::t("nav.devices"),
                        View::Meet => i18n::t("nav.meet"),
                        View::Settings => i18n::t("nav.settings"),
                    };
                    ui.label(egui::RichText::new(title).size(16.0).strong().color(p.text));
                    ui.add_space(12.0);
                    icons::show(ui, "search", 14.0, p.muted);
                    let mut q = self.search.clone();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut q)
                                .desired_width(230.0)
                                .hint_text(i18n::t("hdr.search"))
                                .margin(egui::Margin::symmetric(8, 4)),
                        )
                        .changed()
                    {
                        self.search = q;
                        if !self.search.is_empty() {
                            self.view = View::Devices;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        status_pill(
                            ui,
                            online,
                            if online {
                                i18n::t("st.ready")
                            } else {
                                i18n::t("st.connecting")
                            },
                        );
                        if !my_id.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(partners::pretty_id(&my_id))
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        }
                    });
                });
            });
    }

    /// Geräte: links die Ordner, in der Mitte die Liste, rechts das gewählte
    /// Gerät mit allem, was man daran einstellen kann.
    fn devices_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        let all = self.book.sorted();
        self.presence
            .watch(all.iter().map(|x| x.id.clone()).collect());
        // Fuzzy: Gross-/Kleinschreibung und Trennzeichen sind egal -
        // "flei one" findet "FLEI-ONE", "497 628" findet die ID.
        let q = partners::search_norm(&self.search);
        let folder = self.folder.clone();
        let hit = |x: &partners::Partner| {
            let text = q.is_empty()
                || partners::search_norm(&x.label()).contains(&q)
                || partners::search_norm(&x.id).contains(&q);
            let fold = folder.is_empty() || x.group == folder;
            text && fold
        };

        let mut connect: Option<String> = None;
        let mut ask: Option<String> = None;
        let mut fav: Option<String> = None;
        let mut remove: Option<String> = None;
        let mut edit: Option<String> = None;

        let detail_w = if self.selected.is_some() { 288.0 } else { 0.0 };

        ui.horizontal_top(|ui| {
            // -------------------------------------------------- Ordner
            ui.vertical(|ui| {
                ui.set_width(148.0);
                label_small(ui, i18n::t("dev.folder"));
                let groups = self.book.groups();
                let mut rows: Vec<(String, String, usize)> = vec![(
                    String::new(),
                    i18n::t("dev.folder_all").to_string(),
                    all.len(),
                )];
                for g in groups.iter() {
                    let n = all.iter().filter(|x| &x.group == g).count();
                    rows.push((g.clone(), g.clone(), n));
                }
                for (key, label, n) in rows {
                    let sel = self.folder == key;
                    let r = ui.add(
                        egui::Button::image_and_text(
                            icons::image(
                                if key.is_empty() { "devices" } else { "files" },
                                14.0,
                                if sel { p.accent } else { p.muted },
                            ),
                            egui::RichText::new(format!("{}  {}", label, n))
                                .size(12.0)
                                .color(if sel { p.text } else { p.muted }),
                        )
                        .fill(if sel {
                            p.accent.gamma_multiply(0.14)
                        } else {
                            egui::Color32::TRANSPARENT
                        })
                        .stroke(egui::Stroke::NONE)
                        .min_size(egui::vec2(140.0, 28.0)),
                    );
                    if r.clicked() {
                        self.folder = key.clone();
                    }
                    ui.add_space(2.0);
                }
            });

            ui.add_space(10.0);

            // -------------------------------------------------- Liste
            ui.vertical(|ui| {
                let w = (ui.available_width() - detail_w - 12.0).max(320.0);
                ui.set_width(w);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(i18n::tf("dev.count", &all.len().to_string()))
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_ghost(ui, "plus", i18n::t("dev.add_tip")).clicked() {
                            // Was im Suchfeld steht ist meistens genau die ID,
                            // die hinzugefuegt werden soll - also uebernehmen
                            // und den Rest im Fenster ausfuellen lassen.
                            let id: String =
                                self.search.chars().filter(|c| c.is_ascii_digit()).collect();
                            self.add_dev = Some(AddDev {
                                id,
                                ..Default::default()
                            });
                        }
                        if icon_ghost(ui, "refresh", i18n::t("dev.refresh_tip")).clicked() {
                            self.presence
                                .watch(all.iter().map(|x| x.id.clone()).collect());
                        }
                    });
                });
                ui.add_space(4.0);

                egui::Frame::NONE
                    .fill(p.card)
                    .stroke(egui::Stroke::new(1.0, p.line))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(0, 5))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new(i18n::t("dev.name")).size(10.0).color(p.muted),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_space(28.0);
                                ui.label(
                                    egui::RichText::new(i18n::t("dev.status"))
                                        .size(10.0)
                                        .color(p.muted),
                                );
                                ui.add_space(28.0);
                                ui.label(
                                    egui::RichText::new(i18n::t("dev.id")).size(10.0).color(p.muted),
                                );
                            });
                        });
                        ui.add_space(3.0);
                        divider(ui);
                        ui.add_space(3.0);

                        let mut groups: Vec<(String, Vec<partners::Partner>)> = Vec::new();
                        if all.len() > 3 && folder.is_empty() && q.is_empty() {
                            groups.push((
                                i18n::t("dev.group_recent").to_string(),
                                all.iter().filter(|x| hit(x)).take(3).cloned().collect(),
                            ));
                        }
                        let favs: Vec<partners::Partner> =
                            all.iter().filter(|x| hit(x) && x.favorite).cloned().collect();
                        if !favs.is_empty() {
                            groups.push((i18n::t("dev.group_fav").to_string(), favs));
                        }
                        groups.push((
                            i18n::t("dev.group_online").to_string(),
                            all.iter()
                                .filter(|x| hit(x) && self.presence.online(&x.id))
                                .cloned()
                                .collect(),
                        ));
                        groups.push((
                            i18n::t("dev.group_offline").to_string(),
                            all.iter()
                                .filter(|x| hit(x) && !self.presence.online(&x.id))
                                .cloned()
                                .collect(),
                        ));

                        let mut shown = 0usize;
                        for (title, list) in groups {
                            if list.is_empty() {
                                continue;
                            }
                            shown += list.len();
                            let gid = ui.make_persistent_id(("fv_group", title.clone()));
                            let open = ui.data_mut(|d| *d.get_temp_mut_or(gid, true));
                            let head = ui.add(
                                egui::Button::image_and_text(
                                    icons::image(
                                        if open { "chevron-down" } else { "chevron-right" },
                                        12.0,
                                        p.muted,
                                    ),
                                    egui::RichText::new(format!("{}  {}", title, list.len()))
                                        .size(12.0)
                                        .strong()
                                        .color(p.text),
                                )
                                .fill(p.card_hi)
                                .stroke(egui::Stroke::NONE)
                                .corner_radius(0)
                                .min_size(egui::vec2(ui.available_width(), 24.0)),
                            );
                            if head.clicked() {
                                ui.data_mut(|d| d.insert_temp(gid, !open));
                            }
                            if !open {
                                continue;
                            }
                            for x in list.iter() {
                                let on = self.presence.online(&x.id);
                                let sel = self.selected.as_deref() == Some(x.id.as_str());
                                egui::Frame::NONE
                                    .fill(if sel {
                                        p.row_sel
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    })
                                    .inner_margin(egui::Margin::symmetric(10, 3))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            let shown = self.dev_label(x);
                                            icons::show(
                                                ui,
                                                device_symbol(&shown),
                                                15.0,
                                                row_color(on, false),
                                            );
                                            let name = ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&shown)
                                                        .size(12.5)
                                                        .color(row_color(on, true)),
                                                )
                                                .sense(egui::Sense::click()),
                                            );
                                            if name.clicked() {
                                                self.selected = Some(x.id.clone());
                                            }
                                            if name.double_clicked() {
                                                connect = Some(x.id.clone());
                                            }
                                            if x.favorite {
                                                icons::show(ui, "star", 11.0, p.accent);
                                            }
                                            if self.is_me(&x.id) {
                                                ui.label(
                                                    egui::RichText::new(i18n::t("dev.me"))
                                                        .size(10.5)
                                                        .color(p.accent),
                                                );
                                            }
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.menu_image_button(
                                                        icons::image("dots", 13.0, p.muted),
                                                        |ui| {
                                                            if ui
                                                                .button(i18n::t("dev.connect"))
                                                                .clicked()
                                                            {
                                                                connect = Some(x.id.clone());
                                                                ui.close();
                                                            }
                                                            if ui
                                                                .button(i18n::t("dev.ask_connect"))
                                                                .clicked()
                                                            {
                                                                ask = Some(x.id.clone());
                                                                ui.close();
                                                            }
                                                            if ui.button(i18n::t("dev.edit")).clicked()
                                                            {
                                                                edit = Some(x.id.clone());
                                                                ui.close();
                                                            }
                                                            if ui
                                                                .button(if x.favorite {
                                                                    i18n::t("dev.fav_del")
                                                                } else {
                                                                    i18n::t("dev.fav_add")
                                                                })
                                                                .clicked()
                                                            {
                                                                fav = Some(x.id.clone());
                                                                ui.close();
                                                            }
                                                            if ui
                                                                .button(i18n::t("dev.remove"))
                                                                .clicked()
                                                            {
                                                                remove = Some(x.id.clone());
                                                                ui.close();
                                                            }
                                                        },
                                                    );
                                                    ui.add_space(4.0);
                                                    status_pill(
                                                        ui,
                                                        on,
                                                        if on {
                                                            i18n::t("st.online")
                                                        } else {
                                                            i18n::t("st.offline")
                                                        },
                                                    );
                                                    ui.add_space(10.0);
                                                    ui.label(
                                                        egui::RichText::new(partners::pretty_id(
                                                            &x.id,
                                                        ))
                                                        .size(12.0)
                                                        .color(if on {
                                                            p.muted
                                                        } else {
                                                            p.muted.gamma_multiply(0.7)
                                                        }),
                                                    );
                                                },
                                            );
                                        });
                                    });
                            }
                        }

                        if shown == 0 {
                            ui.add_space(8.0);
                            ui.vertical_centered(|ui| {
                                icons::show(ui, "devices", 24.0, p.muted);
                                ui.label(
                                    egui::RichText::new(if all.is_empty() {
                                        i18n::t("dev.empty")
                                    } else {
                                        i18n::t("dev.nohit")
                                    })
                                    .size(12.0)
                                    .color(p.muted),
                                );
                            });
                            ui.add_space(8.0);
                        }
                    });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    dot(ui, true);
                    ui.label(
                        egui::RichText::new(i18n::t("dev.secure"))
                            .size(11.0)
                            .color(p.muted),
                    );
                });
            });

            // -------------------------------------------------- Detail
            if let Some(id) = self.selected.clone() {
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_width(276.0);
                    let entry = self.book.get(&id).cloned().unwrap_or_default();
                    let on = self.presence.online(&id);
                    let shown = self.dev_label(&entry);
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            icons::show(ui, device_symbol(&shown), 18.0, row_color(on, false));
                            ui.label(
                                egui::RichText::new(&shown)
                                    .size(15.0)
                                    .strong()
                                    .color(p.text),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_ghost(ui, "dots", i18n::t("dev.edit")).clicked() {
                                    edit = Some(id.clone());
                                }
                            });
                        });
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            status_pill(
                                ui,
                                on,
                                if on {
                                    i18n::t("st.online")
                                } else {
                                    i18n::t("st.offline")
                                },
                            );
                            ui.label(
                                egui::RichText::new(partners::pretty_id(&entry.id))
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        });
                        ui.add_space(8.0);
                        if accent_button(ui, i18n::t("dev.connect"), true).clicked() {
                            connect = Some(id.clone());
                        }
                        ui.add_space(4.0);
                        if ui
                            .add(ghost(egui::vec2(220.0, 28.0), i18n::t("dev.ask_connect")))
                            .on_hover_text(i18n::t("start.ask_tip"))
                            .clicked()
                        {
                            ask = Some(id.clone());
                        }
                        ui.add_space(8.0);
                        divider(ui);
                        ui.add_space(5.0);
                        info_row(
                            ui,
                            i18n::t("start.password"),
                            if entry.secret.is_some() {
                                i18n::t("dev.pw_stored")
                            } else {
                                i18n::t("dev.pw_none")
                            },
                        );
                        info_row(ui, i18n::t("dev.folder"), if entry.group.is_empty() { "-" } else { &entry.group });
                        info_row(ui, i18n::t("dev.stats"), &entry.count.to_string());
                        info_row(ui, i18n::t("dev.last"), &entry.ago());
                        if !entry.note.is_empty() {
                            ui.add_space(3.0);
                            ui.label(egui::RichText::new(&entry.note).size(11.5).color(p.muted));
                        }
                    });

                    // ---- Bearbeiten
                    if self.edit_dev.as_ref().map(|e| e.id.clone()) == Some(id.clone()) {
                        let mut e = self.edit_dev.clone().unwrap_or_default();
                        let mut save = false;
                        let mut cancel = false;
                        ui.add_space(8.0);
                        label_small(ui, i18n::t("dev.edit"));
                        card(ui, |ui| {
                            label_small(ui, i18n::t("dev.newname"));
                            ui.add(
                                egui::TextEdit::singleline(&mut e.name)
                                    .desired_width(230.0)
                                    .margin(egui::Margin::symmetric(8, 4)),
                            );
                            ui.add_space(5.0);
                            label_small(ui, i18n::t("dev.pw_set"));
                            ui.add(
                                egui::TextEdit::singleline(&mut e.password)
                                    .password(true)
                                    .desired_width(230.0)
                                    .margin(egui::Margin::symmetric(8, 4))
                                    .hint_text(i18n::t("start.pw_hint")),
                            );
                            ui.add_space(5.0);
                            label_small(ui, i18n::t("dev.folder"));
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut e.folder)
                                        .desired_width(140.0)
                                        .margin(egui::Margin::symmetric(8, 4))
                                        .hint_text(i18n::t("dev.folder_new")),
                                );
                                let known = self.book.groups();
                                if !known.is_empty() {
                                    egui::ComboBox::from_id_salt("fold_pick")
                                        .selected_text("…")
                                        .width(60.0)
                                        .show_ui(ui, |ui| {
                                            for g in known.iter() {
                                                if ui.selectable_label(false, g).clicked() {
                                                    e.folder = g.clone();
                                                }
                                            }
                                        });
                                }
                            });
                            ui.add_space(5.0);
                            label_small(ui, i18n::t("dev.note"));
                            ui.add(
                                egui::TextEdit::multiline(&mut e.note)
                                    .desired_rows(2)
                                    .desired_width(230.0),
                            );
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if accent_button(ui, i18n::t("dev.save"), true).clicked() {
                                    save = true;
                                }
                                if ghost_button(ui, i18n::t("dev.cancel")).clicked() {
                                    cancel = true;
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if icon_ghost(ui, "mic-off", i18n::t("dev.pw_clear"))
                                            .clicked()
                                        {
                                            self.book.set_password(&id, None);
                                        }
                                    },
                                );
                            });
                        });
                        if save {
                            self.book.rename(&id, &e.name);
                            self.book.set_group(&id, &e.folder);
                            self.book.set_note(&id, &e.note);
                            if !e.password.is_empty() {
                                self.book.set_password(&id, Some(&e.password));
                            }
                            self.edit_dev = None;
                        } else if cancel {
                            self.edit_dev = None;
                        } else {
                            self.edit_dev = Some(e);
                        }
                    }
                });
            }
        });

        if let Some(id) = edit {
            let e = self.book.get(&id).cloned().unwrap_or_default();
            self.selected = Some(id.clone());
            self.edit_dev = Some(DevEdit {
                id,
                name: e.name.clone(),
                password: String::new(),
                note: e.note.clone(),
                folder: e.group.clone(),
            });
        }
        if let Some(id) = connect {
            self.connect_to(&id);
        }
        if let Some(id) = ask {
            self.partner_id = id.clone();
            self.partner_pw.clear();
            self.selected = Some(id);
            self.start_ask_session();
        }
        if let Some(id) = fav {
            self.book.toggle_favorite(&id);
            self.book.save();
        }
        if let Some(id) = remove {
            self.book.remove(&id);
            self.book.save();
            if self.selected.as_deref() == Some(id.as_str()) {
                self.selected = None;
                self.edit_dev = None;
            }
        }
    }

    /// "Geraet hinzufuegen": ein kleines Fenster mit allem, was ein Eintrag
    /// braucht. Vorher tat der Plus-Knopf nur dann etwas, wenn zufaellig
    /// schon eine vollstaendige ID im Suchfeld stand - fuer jeden anderen
    /// war er kaputt.
    fn add_dev_ui(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.add_dev.clone() else {
            return;
        };
        let p = theme::palette();
        let mut open = true;
        let mut save = false;
        egui::Window::new(i18n::t("dev.add_title"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
            .show(ctx, |ui| {
                ui.set_width(320.0);
                ui.add_space(2.0);
                label_small(ui, i18n::t("dev.add_id"));
                let id_field = ui.add(
                    egui::TextEdit::singleline(&mut e.id)
                        .desired_width(f32::INFINITY)
                        .hint_text(i18n::t("dev.add_id_hint")),
                );
                if self.add_dev.is_some() && !id_field.has_focus() && e.id.is_empty() {
                    id_field.request_focus();
                }
                ui.add_space(8.0);
                label_small(ui, i18n::t("dev.add_name"));
                ui.add(
                    egui::TextEdit::singleline(&mut e.name)
                        .desired_width(f32::INFINITY)
                        .hint_text(i18n::t("dev.add_name_hint")),
                );
                ui.add_space(8.0);
                label_small(ui, i18n::t("dev.folder"));
                ui.add(
                    egui::TextEdit::singleline(&mut e.folder)
                        .desired_width(f32::INFINITY)
                        .hint_text(i18n::t("dev.folder_new")),
                );
                ui.add_space(8.0);
                label_small(ui, i18n::t("dev.add_pw"));
                ui.add(
                    egui::TextEdit::singleline(&mut e.password)
                        .password(true)
                        .desired_width(f32::INFINITY)
                        .hint_text(i18n::t("dev.add_pw_hint")),
                );
                if !e.err.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(&e.err).size(11.5).color(p.violet),
                    );
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, i18n::t("dev.add_ok"), true).clicked() {
                        save = true;
                    }
                    if ghost_button(ui, i18n::t("dev.cancel")).clicked() {
                        self.add_dev = None;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            save = true;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.add_dev = None;
            return;
        }
        if !open {
            self.add_dev = None;
            return;
        }
        if !save {
            self.add_dev = Some(e);
            return;
        }
        let id: String = e.id.chars().filter(|c| c.is_ascii_digit()).collect();
        if !(9..=10).contains(&id.len()) {
            e.err = i18n::t("dev.add_bad_id").to_string();
        } else if self.is_me(&id) {
            e.err = i18n::t("dev.add_self").to_string();
        } else if self.book.get(&id).is_some() {
            e.err = i18n::t("dev.add_known").to_string();
        } else {
            self.book.started(&id, "", false);
            if !e.name.trim().is_empty() {
                self.book.rename(&id, &e.name);
            }
            if !e.folder.trim().is_empty() {
                self.book.set_group(&id, &e.folder);
            }
            if !e.password.is_empty() {
                self.book.set_password(&id, Some(&e.password));
            }
            self.book.save();
            self.presence.watch(vec![id.clone()]);
            self.search.clear();
            self.selected = Some(id);
            self.add_dev = None;
            return;
        }
        self.add_dev = Some(e);
    }

    /// Einstellungen: links die Bereiche, rechts der gewählte Bereich.
    fn settings_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        ui.horizontal_top(|ui| {
            // ---- Bereichsliste
            ui.vertical(|ui| {
                ui.set_width(178.0);
                for (tab, icon, label) in [
                    (SettingsTab::General, "settings", i18n::t("set.general")),
                    (SettingsTab::Access, "shield", i18n::t("set.access")),
                    (SettingsTab::Account, "user", i18n::t("set.account")),
                    (SettingsTab::Deploy, "laptop", i18n::t("set.deploy")),
                    (SettingsTab::Audio, "mic", i18n::t("set.audio")),
                    (SettingsTab::Look, "palette", i18n::t("set.look")),
                    (SettingsTab::Feedback, "chat", i18n::t("set.feedback")),
                    (SettingsTab::About, "eye", i18n::t("set.about")),
                ] {
                    let sel = self.stab == tab;
                    // selbst zeichnen, sonst schluckt die feste Fuellung des
                    // egui-Knopfes jede Reaktion auf die Maus
                    let (rect, r) =
                        ui.allocate_exact_size(egui::vec2(166.0, 32.0), egui::Sense::click());
                    let hov = r.hovered();
                    let fill = if sel {
                        p.accent.gamma_multiply(0.14)
                    } else if hov {
                        p.card_hi
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    if fill != egui::Color32::TRANSPARENT {
                        ui.painter().rect_filled(rect, 7.0, fill);
                    }
                    let fg = if sel {
                        p.accent
                    } else if hov {
                        p.text
                    } else {
                        p.muted
                    };
                    let icon_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.min.x + 9.0, rect.center().y - 8.0),
                        egui::vec2(16.0, 16.0),
                    );
                    ui.put(icon_rect, icons::image(icon, 16.0, fg));
                    ui.painter().text(
                        egui::pos2(rect.min.x + 34.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(13.0),
                        if sel { p.text } else { fg },
                    );
                    let r = zeigefinger(r);
                    if r.clicked() {
                        self.stab = tab;
                    }
                    ui.add_space(2.0);
                }
            });
            ui.add_space(14.0);
            // ---- Inhalt
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                match self.stab {
                    SettingsTab::General => self.set_general(ui),
                    SettingsTab::Access => self.set_access(ui),
                    SettingsTab::Account => self.set_account(ui),
                    SettingsTab::Deploy => self.set_deploy(ui),
                    SettingsTab::Audio => self.set_audio(ui),
                    SettingsTab::Look => self.set_look(ui),
                    SettingsTab::Feedback => self.set_feedback(ui),
                    SettingsTab::About => self.set_about(ui),
                }
            });
        });
        if !self.hint.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&self.hint).color(p.accent).size(12.0));
        }
    }

    fn set_general(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            label_small(ui, i18n::t("set.name"));
            // Leer heisst: es steht noch nichts eigenes drin - dann zeigen wir
            // den Rechnernamen, so wie ihn auch die Gegenseite sieht.
            let mut name = {
                let mut n = self.shared.device_name.lock().unwrap().clone();
                if n.trim().is_empty() {
                    n = presence::machine_name();
                    *self.shared.device_name.lock().unwrap() = n.clone();
                }
                n
            };
            if ui
                .add(
                    egui::TextEdit::singleline(&mut name)
                        .desired_width(240.0)
                        .margin(egui::Margin::symmetric(8, 4))
                        .hint_text(i18n::t("set.name_hint")),
                )
                .changed()
            {
                let clean = presence::clean(&name);
                *self.shared.device_name.lock().unwrap() = clean.clone();
                let _ = presence::save_device_name(&clean);
            }
            ui.add_space(8.0);
            label_small(ui, i18n::t("set.lang"));
            let mut a = self.look.clone();
            let cur = i18n::LANGS
                .iter()
                .find(|(c, _)| *c == a.lang)
                .map(|(_, n)| *n)
                .unwrap_or("Deutsch");
            egui::ComboBox::from_id_salt("lang_pick")
                .selected_text(cur)
                .width(160.0)
                .show_ui(ui, |ui| {
                    for (code, name) in i18n::LANGS.iter() {
                        if ui.selectable_label(a.lang == *code, *name).clicked() {
                            a.lang = code.to_string();
                        }
                    }
                });
            if a != self.look {
                self.apply_look(ui.ctx(), a);
            }
        });
        ui.add_space(10.0);
        self.install_card(ui);
    }

    /// Installation: portabel laufen lassen oder richtig einrichten.
    fn install_card(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        label_small(ui, i18n::t("set.install"));
        card(ui, |ui| {
            let installed = setup::is_installed();
            let here = setup::running_installed();
            let state = if here {
                i18n::tf("set.install_here", &setup::install_dir().display().to_string())
            } else if installed {
                i18n::tf("set.install_other", &setup::install_dir().display().to_string())
            } else {
                i18n::t("set.install_portable").to_string()
            };
            ui.horizontal(|ui| {
                dot(ui, here);
                ui.label(egui::RichText::new(state).size(12.5).color(p.text));
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(i18n::t("set.install_note"))
                    .size(11.5)
                    .color(p.muted),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if !installed {
                    if ghost_button(ui, i18n::t("set.install_do")).clicked() {
                        let flags = if self.install_service {
                            "--install --with-service"
                        } else {
                            "--install"
                        };
                        match service::elevate(flags) {
                            Ok(()) => self.hint = i18n::t("set.install_running").to_string(),
                            Err(e) => self.hint = format!("{}", e),
                        }
                    }
                    ui.add_space(8.0);
                    check(
                        ui,
                        &mut self.install_service,
                        i18n::t("set.install_with_service"),
                    )
                    .on_hover_text(i18n::t("set.service_tip"));
                } else {
                    // Reparieren: Dateien frisch drueber (bleibt ohne
                    // zusaetzliche Frage, die Rechte hat man schon gegeben)
                    if ghost_button(ui, i18n::t("set.repair")).clicked() {
                        let flags = if self.service_on {
                            "--install --with-service"
                        } else {
                            "--install"
                        };
                        match service::elevate(flags) {
                            Ok(()) => self.hint = i18n::t("set.repair_running").to_string(),
                            Err(e) => self.hint = format!("{}", e),
                        }
                    }
                    ui.add_space(8.0);
                    if ghost_button(ui, i18n::t("set.uninstall_do")).clicked() {
                        self.uninstall_ask = true;
                    }
                }
            });
        });
    }

    fn set_access(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            let mut boot = self.autostart;
            if check(ui, &mut boot, i18n::t("set.autostart"))
                .on_hover_text(i18n::t("set.autostart_tip"))
                .changed()
            {
                match autostart::set(boot) {
                    Ok(()) => self.autostart = boot,
                    Err(e) => self.hint = format!("{}", e),
                }
            }
            ui.add_space(4.0);
            let mut svc = self.service_on;
            if check(ui, &mut svc, i18n::t("set.service"))
                .on_hover_text(i18n::t("set.service_tip"))
                .changed()
            {
                let flag = if svc {
                    "--install-service"
                } else {
                    "--uninstall-service"
                };
                match service::elevate(flag) {
                    Ok(()) => self.hint = i18n::t("set.service_tip").to_string(),
                    Err(e) => self.hint = format!("{}", e),
                }
            }
            ui.add_space(4.0);
            let mut clip = self.shared.clip_on.load(Ordering::Relaxed);
            if check(ui, &mut clip, i18n::t("set.clip"))
                .on_hover_text(i18n::t("set.clip_tip"))
                .changed()
            {
                self.shared.clip_on.store(clip, Ordering::Relaxed);
                ident::set_clipboard(clip);
            }
            ui.add_space(4.0);
            let mut keep = self.pw_fixed;
            if check(ui, &mut keep, i18n::t("start.keep_pw"))
                .on_hover_text(i18n::t("start.keep_pw_tip"))
                .changed()
            {
                self.pw_fixed = keep;
                let pw = self.shared.password.lock().unwrap().clone();
                let store = if keep { Some(pw.as_str()) } else { None };
                let _ = ident::set_fixed_password(store);
            }
        });
        ui.add_space(10.0);
        self.pw_card(ui);
    }

    /// Feste Passwoerter: beliebig viele, immer als Punkte, mit Bezeichnung -
    /// wie ein Array im Unity-Inspector.
    fn pw_card(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        label_small(ui, i18n::t("set.pw_perm"));
        card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t("set.pw_perm_tip"))
                    .size(11.5)
                    .color(p.muted),
            );
            ui.add_space(8.0);
            let mut remove: Option<usize> = None;
            let mut dirty = false;
            for i in 0..self.pw_list.len() {
                let shown = self.pw_show == Some(i);
                ui.horizontal(|ui| {
                    let mut label = self.pw_list[i].label.clone();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut label)
                                .desired_width(118.0)
                                .hint_text(i18n::t("set.pw_label_hint"))
                                .margin(egui::Margin::symmetric(7, 4)),
                        )
                        .changed()
                    {
                        self.pw_list[i].label = label;
                        dirty = true;
                    }
                    ui.add_space(8.0);
                    if shown {
                        let mut pw = self.pw_list[i].pw.clone();
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut pw)
                                    .font(egui::FontId::new(14.0, egui::FontFamily::Monospace))
                                    .desired_width(160.0)
                                    .margin(egui::Margin::symmetric(7, 4)),
                            )
                            .changed()
                        {
                            self.pw_list[i].pw = pw;
                            dirty = true;
                        }
                    } else {
                        ui.add(egui::Label::new(
                            egui::RichText::new(pwlist::masked(&self.pw_list[i].pw))
                                .size(16.0)
                                .family(egui::FontFamily::Monospace)
                                .color(p.text),
                        ));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_ghost(ui, "trash", i18n::t("set.pw_del")).clicked() {
                            remove = Some(i);
                        }
                        if icon_ghost(
                            ui,
                            "eye",
                            if shown {
                                i18n::t("set.pw_hide")
                            } else {
                                i18n::t("set.pw_show")
                            },
                        )
                        .clicked()
                        {
                            self.pw_show = if shown { None } else { Some(i) };
                        }
                        if icon_ghost(ui, "copy", i18n::t("start.copy")).clicked() {
                            let pw = self.pw_list[i].pw.clone();
                            ui.ctx().copy_text(pw);
                        }
                    });
                });
                ui.add_space(4.0);
            }
            if self.pw_list.is_empty() {
                ui.label(
                    egui::RichText::new(i18n::t("set.pw_empty"))
                        .size(12.0)
                        .color(p.muted),
                );
            }
            if let Some(i) = remove {
                self.pw_list.remove(i);
                self.pw_show = None;
                dirty = true;
            }
            ui.add_space(8.0);
            divider(ui);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.pw_new_label)
                        .desired_width(118.0)
                        .hint_text(i18n::t("set.pw_label_hint"))
                        .margin(egui::Margin::symmetric(7, 4)),
                );
                ui.add_space(8.0);
                let enter = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.pw_new)
                            .desired_width(160.0)
                            .password(true)
                            .hint_text(i18n::t("set.pw_value"))
                            .margin(egui::Margin::symmetric(7, 4)),
                    )
                    .lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                if icon_ghost(ui, "plus", i18n::t("set.pw_add")).clicked() || enter {
                    let pw = self.pw_new.trim().to_string();
                    match pwlist::why_not(&self.pw_list, &pw) {
                        Some(err) => self.hint = i18n::t(err).to_string(),
                        None => {
                            let label = if self.pw_new_label.trim().is_empty() {
                                format!("#{}", self.pw_list.len() + 1)
                            } else {
                                self.pw_new_label.trim().to_string()
                            };
                            self.pw_list.push(pwlist::Entry { label, pw });
                            self.pw_new.clear();
                            self.pw_new_label.clear();
                            dirty = true;
                        }
                    }
                }
            });
            if dirty {
                if let Err(e) = pwlist::save(&self.pw_list) {
                    self.hint = format!("{}", e);
                }
            }
        });
    }

    /// Einstellungen -> Konto. Ohne Anmeldung laeuft alles wie bisher; mit
    /// Anmeldung liegt die Geraeteliste zusaetzlich beim FleiTec-Konto.
    fn set_account(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        self.take_sync_result();
        label_small(ui, i18n::t("acc.title"));
        card(ui, |ui| {
            match self.acc.clone() {
                None => {
                    ui.label(
                        egui::RichText::new(i18n::t("acc.why"))
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.add_space(8.0);
                    label_small(ui, i18n::t("acc.user"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.acc_user)
                            .desired_width(240.0)
                            .margin(egui::Margin::symmetric(8, 4)),
                    );
                    ui.add_space(6.0);
                    label_small(ui, i18n::t("acc.pass"));
                    let enter = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.acc_pass)
                                .desired_width(240.0)
                                .password(true)
                                .margin(egui::Margin::symmetric(8, 4)),
                        )
                        .lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(8.0);
                    let go = accent_button(ui, i18n::t("acc.login"), true).clicked() || enter;
                    if go {
                        self.do_login();
                    }
                }
                Some(sess) => {
                    ui.horizontal(|ui| {
                        icons::show(ui, "user", 16.0, p.accent);
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(&sess.user).size(15.0).strong());
                    });
                    ui.add_space(2.0);
                    let when = if sess.synced == 0 {
                        i18n::t("acc.never").to_string()
                    } else {
                        presence::ago_ms(sess.synced * 1000)
                    };
                    ui.label(
                        egui::RichText::new(format!("{}: {}", i18n::t("acc.last_sync"), when))
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let busy = self.acc_busy.load(Ordering::Relaxed);
                        if accent_button(ui, i18n::t("acc.sync_now"), !busy).clicked() {
                            self.start_sync(true);
                        }
                        if ghost_button(ui, i18n::t("acc.logout")).clicked() {
                            account::forget();
                            self.acc = None;
                            self.acc_msg = i18n::t("acc.logged_out").to_string();
                        }
                    });
                    ui.add_space(6.0);
                    let n = self.book.sorted().len();
                    ui.label(
                        egui::RichText::new(i18n::tf("acc.count", &n.to_string()))
                            .size(12.0)
                            .color(p.muted),
                    );
                }
            }
        });
        if !self.acc_msg.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(&self.acc_msg).size(12.0).color(p.accent));
        }
        ui.add_space(10.0);
        label_small(ui, i18n::t("acc.privacy_head"));
        card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t("acc.privacy"))
                    .size(12.0)
                    .color(p.muted),
            );
        });
    }

    /// Anmelden im Hintergrund - die Oberflaeche darf dabei nicht stehen.
    fn do_login(&mut self) {
        let relay = self.shared.relay_url.clone();
        let user = self.acc_user.trim().to_string();
        let pass = std::mem::take(&mut self.acc_pass);
        if user.is_empty() || pass.is_empty() {
            self.acc_msg = i18n::t("acc.need_both").to_string();
            return;
        }
        self.acc_msg = i18n::t("acc.logging_in").to_string();
        let busy = self.acc_busy.clone();
        let out = self.acc_out.clone();
        if busy.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(move || {
            let res = match account::login(&relay, &user, &pass) {
                Ok(sess) => {
                    let _ = account::save(&sess);
                    account::SyncOut::LoggedIn(sess.user.clone())
                }
                Err(e) => account::SyncOut::Failed(e.to_string()),
            };
            *out.lock().unwrap() = Some(res);
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// Adressbuch hoch, zusammengefuehrte Liste zurueck.
    fn start_sync(&mut self, sichtbar: bool) {
        let Some(sess) = self.acc.clone() else { return };
        if sichtbar {
            self.acc_msg = i18n::t("acc.syncing").to_string();
        }
        self.acc_next = std::time::Instant::now() + Duration::from_secs(120);
        account::sync_async(
            self.shared.relay_url.clone(),
            sess.token.clone(),
            self.book.to_sync(),
            self.acc_out.clone(),
            self.acc_busy.clone(),
        );
    }

    /// Ergebnis eines Hintergrundlaufs abholen (Anmeldung oder Abgleich).
    fn take_sync_result(&mut self) {
        let res = self.acc_out.lock().unwrap().take();
        let Some(res) = res else { return };
        match res {
            account::SyncOut::Ok { devices, at } => {
                let changed = self.book.merge_remote(&devices);
                if let Some(sess) = self.acc.as_mut() {
                    sess.synced = at;
                    let _ = account::save(sess);
                }
                self.acc_msg = if changed {
                    i18n::t("acc.updated").to_string()
                } else {
                    i18n::t("acc.uptodate").to_string()
                };
            }
            account::SyncOut::LoggedOut => {
                account::forget();
                self.acc = None;
                self.acc_msg = i18n::t("acc.expired").to_string();
            }
            account::SyncOut::LoggedIn(user) => {
                // Anmeldung geklappt - die Sitzung liegt schon auf der Platte
                self.acc = account::load();
                self.acc_msg = i18n::tf("acc.hello", &user);
                self.acc_user.clear();
                self.start_sync(false);
            }
            account::SyncOut::Failed(msg) => {
                self.acc_msg = msg;
            }
        }
    }

    /// Einrichten-Tab: per Einmal-Link ein neues Geraet aufsetzen.
    fn set_deploy(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        self.take_setup_result();
        label_small(ui, i18n::t("dep.title"));
        let Some(sess) = self.acc.clone() else {
            card(ui, |ui| {
                ui.label(
                    egui::RichText::new(i18n::t("dep.need_login"))
                        .size(12.0)
                        .color(p.muted),
                );
                ui.add_space(8.0);
                if accent_button(ui, i18n::t("dep.to_account"), true).clicked() {
                    self.stab = SettingsTab::Account;
                }
            });
            return;
        };
        card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t("dep.lead"))
                    .size(12.0)
                    .color(p.muted),
            );
            ui.add_space(8.0);
            label_small(ui, i18n::t("dep.pw"));
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.dep_pw)
                        .desired_width(200.0)
                        .margin(egui::Margin::symmetric(8, 4)),
                );
                if ghost_button(ui, i18n::t("dep.pw_new")).clicked() {
                    self.dep_pw = ident::random_password();
                }
            });
            ui.add_space(6.0);
            label_small(ui, i18n::t("dep.name_hint"));
            ui.add(
                egui::TextEdit::singleline(&mut self.dep_hint_name)
                    .desired_width(200.0)
                    .margin(egui::Margin::symmetric(8, 4))
                    .hint_text(i18n::t("dep.name_hint_ph")),
            );
            ui.add_space(6.0);
            label_small(ui, i18n::t("dep.count"));
            ui.horizontal(|ui| {
                let mut unbegrenzt = self.dep_count == 0;
                if unbegrenzt {
                    // eine graue "0" waere verwirrend - lieber das Zeichen
                    ui.label(
                        egui::RichText::new("∞")
                            .size(19.0)
                            .strong()
                            .color(p.accent),
                    );
                } else {
                    ui.add(
                        egui::DragValue::new(&mut self.dep_count)
                            .range(1..=25)
                            .speed(0.2),
                    );
                }
                ui.add_space(6.0);
                if check(ui, &mut unbegrenzt, i18n::t("dep.unlimited"))
                    .on_hover_text(i18n::t("dep.unlimited_tip"))
                    .changed()
                {
                    // 0 heisst am Relay: so oft wie man will
                    self.dep_count = if unbegrenzt { 0 } else { 1 };
                }
            });
            ui.add_space(8.0);
            let busy = self.acc_busy.load(Ordering::Relaxed);
            let label = if busy {
                i18n::t("dep.making")
            } else {
                i18n::t("dep.make")
            };
            if accent_button(ui, label, !busy && !self.dep_pw.trim().is_empty()).clicked() {
                account::setup_create_async(
                    self.shared.relay_url.clone(),
                    sess.token.clone(),
                    self.dep_pw.trim().to_string(),
                    self.dep_hint_name.trim().to_string(),
                    self.dep_count,
                    self.dep_out.clone(),
                    self.acc_busy.clone(),
                );
            }
            if !self.dep_msg.is_empty() {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(&self.dep_msg).size(12.0).color(p.accent));
            }
        });
        ui.add_space(10.0);
        label_small(ui, i18n::t("dep.status"));
        card(ui, |ui| {
            // Solange der Tab offen ist: alle 5 s in den Posteingang sehen.
            if std::time::Instant::now() >= self.dep_next
                && !self.acc_busy.load(Ordering::Relaxed)
            {
                self.dep_next = std::time::Instant::now() + Duration::from_secs(5);
                account::setup_inbox_async(
                    self.shared.relay_url.clone(),
                    sess.token.clone(),
                    self.dep_out.clone(),
                    self.acc_busy.clone(),
                );
                ui.ctx().request_repaint_after(Duration::from_secs(5));
            }
            if self.dep_pending.is_empty() && self.dep_claimed.is_empty() {
                ui.label(
                    egui::RichText::new(i18n::t("dep.none"))
                        .size(12.0)
                        .color(p.muted),
                );
            }
            let pending = self.dep_pending.clone();
            let mut revoke: Option<String> = None;
            let mut kopiert = false;
            for pend in &pending {
                ui.horizontal(|ui| {
                    icons::show(ui, "refresh", 14.0, p.muted);
                    let was = if pend.name_hint.is_empty() {
                        pend.code.clone()
                    } else {
                        format!("{} ({})", pend.name_hint, pend.code)
                    };
                    let zeile = if pend.max_uses == 0 {
                        format!(
                            "{} - {} ({})",
                            was,
                            i18n::t("dep.waiting"),
                            i18n::tf("dep.used_unlimited", &pend.uses.to_string())
                        )
                    } else if pend.max_uses > 1 {
                        format!(
                            "{} - {} ({})",
                            was,
                            i18n::t("dep.waiting"),
                            i18n::tf(
                                "dep.left",
                                &format!(
                                    "{} ({} von {})",
                                    pend.max_uses.saturating_sub(pend.uses),
                                    pend.uses,
                                    pend.max_uses
                                )
                            )
                        )
                    } else {
                        format!("{} - {}", was, i18n::t("dep.waiting"))
                    };
                    ui.label(
                        egui::RichText::new(zeile).size(12.0).color(p.muted),
                    );
                    // Der Link steht daneben (nicht darunter) - mit Kopieren
                    // und Widerrufen direkt in derselben Zeile.
                    let web = account::setup_web_link(&pend.code);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_ghost(ui, "trash", i18n::t("dep.revoke_tip")).clicked() {
                            revoke = Some(pend.code.clone());
                        }
                        if icon_ghost(ui, "copy", i18n::t("dep.copy")).clicked() {
                            ui.ctx().copy_text(web.clone());
                            kopiert = true;
                        }
                        ui.label(
                            egui::RichText::new(&web)
                                .size(11.5)
                                .color(p.accent),
                        );
                    });
                });
                ui.add_space(6.0);
            }
            if kopiert {
                self.dep_msg = i18n::t("dep.copied").to_string();
            }
            if let Some(code) = revoke {
                account::setup_revoke_async(
                    self.shared.relay_url.clone(),
                    sess.token.clone(),
                    code,
                );
                // gleich neu lesen, damit die Zeile verschwindet
                self.dep_next = std::time::Instant::now();
            }
            let claimed = self.dep_claimed.clone();
            for cl in &claimed {
                ui.horizontal(|ui| {
                    dot(ui, true);
                    ui.label(
                        egui::RichText::new(format!(
                            "{}  {}  -  {}",
                            cl.name,
                            partners::pretty_id(&cl.id),
                            i18n::t("dep.done_row")
                        ))
                        .size(12.0),
                    );
                    if icon_ghost(ui, "copy", &cl.password).clicked() {
                        ui.ctx().copy_text(cl.password.clone());
                    }
                });
                ui.add_space(2.0);
            }
        });
    }

    /// Ergebnisse der Einrichtungs-Hintergrundlaeufe abholen.
    fn take_setup_result(&mut self) {
        let res = self.dep_out.lock().unwrap().take();
        let Some(res) = res else { return };
        match res {
            account::SetupOut::Created { link, web, .. } => {
                self.dep_link = if web.is_empty() { link } else { web };
                // frisch erzeugt = sofort in der Zwischenablage, ohne dass
                // jemand danach noch einen Knopf sucht
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set_text(self.dep_link.clone());
                }
                self.dep_msg = i18n::t("dep.copied").to_string();
                self.dep_next = std::time::Instant::now();
            }
            account::SetupOut::Inbox { pending, claimed } => {
                if self.dep_booted {
                    let mut neue = Vec::new();
                    for cl in &claimed {
                        if !self.dep_seen.contains(&cl.id) {
                            self.dep_seen.insert(cl.id.clone());
                            neue.push(cl.name.clone());
                        }
                    }
                    if !neue.is_empty() {
                        // liegt jetzt im Konto-Adressbuch - einmal abgleichen
                        self.hint = i18n::tf("dep.arrived", &neue.join(", "));
                        self.start_sync(false);
                    }
                } else {
                    // erste Fuellung: nur merken, nichts melden
                    for cl in &claimed {
                        self.dep_seen.insert(cl.id.clone());
                    }
                    self.dep_booted = true;
                }
                self.dep_pending = pending;
                self.dep_claimed = claimed;
            }
            account::SetupOut::Failed(msg) => {
                self.dep_msg = msg;
            }
        }
    }

    /// Sicherheits-Frage vor dem Deinstallieren - direkt in der App, ohne
    /// den Umweg ueber "Apps & Features".
    fn uninstall_modal(&mut self, ctx: &egui::Context) {
        if !self.uninstall_ask {
            return;
        }
        let mut offen = true;
        let mut sicher = false;
        let mut weg = false;
        egui::Window::new(i18n::t("set.uninstall_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut offen)
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(theme::card())
                    .stroke(egui::Stroke::new(1.0, theme::accent()))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.label(
                    egui::RichText::new(i18n::t("set.uninstall_sure"))
                        .size(13.0),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(i18n::t("set.uninstall_keep"))
                        .size(11.5)
                        .color(theme::muted()),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, i18n::t("set.uninstall_yes"), true).clicked() {
                        sicher = true;
                    }
                    if ghost_button(ui, i18n::t("setup.cancel")).clicked() {
                        weg = true;
                    }
                });
            });
        if weg {
            offen = false;
        }
        if sicher {
            self.uninstall_ask = false;
            match service::elevate("--uninstall") {
                Ok(()) => self.hint = i18n::t("set.uninstall_running").to_string(),
                Err(e) => self.hint = format!("{}", e),
            }
        } else if !offen {
            self.uninstall_ask = false;
        }
    }

    /// Lizenz-Fenster (nur X-Remote): Schluessel eingeben, pruefen, ablegen.
    /// Ohne gueltige Lizenz erscheint es bei jedem Start einmal - wegklicken
    /// geht, das Programm laeuft vorerst weiter (weiche Einfuehrung).
    #[cfg(feature = "license")]
    fn license_modal(&mut self, ctx: &egui::Context) {
        if !self.lic_show {
            return;
        }
        let mut offen = true;
        let mut spaeter = false;
        egui::Window::new(i18n::t("lic.title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut offen)
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(theme::card())
                    .stroke(egui::Stroke::new(1.0, theme::accent()))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                ui.label(egui::RichText::new(i18n::t("lic.intro")).size(12.5));
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.lic_key)
                        .hint_text(i18n::t("lic.key_ph"))
                        .desired_width(360.0)
                        .font(egui::TextStyle::Monospace),
                );
                if !self.lic_msg.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(&self.lic_msg)
                            .size(11.5)
                            .color(theme::accent()),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, i18n::t("lic.activate"), true).clicked() {
                        match license::activate(&self.lic_key) {
                            Ok(lic) => {
                                self.lic_msg = String::new();
                                self.lic_show = false;
                                self.hint =
                                    i18n::tf("lic.ok_msg", &lic.name);
                            }
                            Err(e) => {
                                self.lic_msg = i18n::tf("lic.invalid", &e);
                            }
                        }
                    }
                    if ghost_button(ui, i18n::t("lic.later")).clicked() {
                        spaeter = true;
                    }
                });
            });
        if spaeter || !offen {
            self.lic_show = false;
        }
    }

    /// Lade-Fenster waehrend der Dienst ein Update einspielt. Danach werden
    /// alle Teile beendet und der Dienst startet mit dem frischen Stand neu.
    fn update_modal(&mut self, ctx: &egui::Context) {
        if self.upd_flag_at.elapsed() > Duration::from_millis(500) {
            self.upd_flag_at = std::time::Instant::now();
            self.upd_running = update::updating_flag()
                .map(|p| p.exists())
                .unwrap_or(false);
        }
        if !self.upd_running {
            return;
        }
        egui::Window::new(i18n::t("upd.working"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(theme::card())
                    .stroke(egui::Stroke::new(1.0, theme::accent()))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(300.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(i18n::t("upd.restart_soon")).size(13.0),
                    );
                });
            });
    }

    /// Passwort-Nachfrage: "Verbinden" heisst direkt verbinden. Ohne
    /// gespeichertes Passwort fragt dieser Dialog nach - die Anfrage beim
    /// Gegenueber (klopfen) ist hier eine bewusste zweite Option.
    fn pw_ask_ui(&mut self, ctx: &egui::Context) {
        let Some(id) = self.pw_ask.clone() else {
            return;
        };
        let mut offen = true;
        let mut verbinden = false;
        let mut anfragen = false;
        let mut weg = false;
        let name = self
            .book
            .get(&id)
            .map(|x| x.label())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| partners::pretty_id(&id));
        egui::Window::new(i18n::tf("pwask.title", &name))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut offen)
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(theme::card())
                    .stroke(egui::Stroke::new(1.0, theme::accent()))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.label(
                    egui::RichText::new(i18n::t("pwask.note"))
                        .size(12.0)
                        .color(theme::muted()),
                );
                ui.add_space(8.0);
                label_small(ui, i18n::t("pwask.pw"));
                let enter = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.partner_pw)
                            .desired_width(220.0)
                            .margin(egui::Margin::symmetric(8, 4)),
                    )
                    .lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let hat_pw = !self.partner_pw.trim().is_empty();
                    if accent_button(ui, i18n::t("start.connect"), hat_pw).clicked()
                        || (enter && hat_pw)
                    {
                        verbinden = true;
                    }
                    if ghost_button(ui, i18n::t("pwask.ask")).clicked() {
                        anfragen = true;
                    }
                    if ghost_button(ui, i18n::t("setup.cancel")).clicked() {
                        weg = true;
                    }
                });
            });
        if verbinden {
            self.pw_ask = None;
            self.start_session();
        } else if anfragen {
            self.pw_ask = None;
            self.start_ask_session();
        } else if weg || !offen {
            self.pw_ask = None;
        }
    }

    /// Einrichtungs-Dialog: dieses Geraet per Einmal-Code einrichten und
    /// danach installieren (mit Dienst).
    fn setup_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = self.setup_dlg.as_mut() else {
            return;
        };
        // Ergebnis des Einloesens abholen
        let got = dlg.out.lock().unwrap().take();
        if let Some(res) = got {
            match res {
                Ok(r) => {
                    let name = presence::clean(&dlg.name);
                    *self.shared.device_name.lock().unwrap() = name.clone();
                    let _ = presence::save_device_name(&name);
                    if !r.password.is_empty() {
                        *self.shared.password.lock().unwrap() = r.password.clone();
                        let _ = ident::set_fixed_password(Some(&r.password));
                        self.pw_fixed = true;
                    }
                    dlg.owner = if r.owner.is_empty() { r.name_hint } else { r.owner };
                    dlg.done = true;
                    // Installation mit Dienst - fragt einmal nach Admin-Rechten
                    if !setup::running_installed() {
                        match service::elevate("--install --with-service") {
                            Ok(()) if dlg.auto => {
                                // Die heruntergeladene Datei hat ausgespielt:
                                // die installierte Fassung uebernimmt - diese
                                // hier darf sich beenden (und wird geloescht).
                                std::thread::spawn(|| {
                                    std::thread::sleep(std::time::Duration::from_millis(1500));
                                    std::process::exit(0);
                                });
                            }
                            Ok(()) => {}
                            Err(e) => dlg.err = format!("{}", e),
                        }
                    }
                }
                Err(e) => dlg.err = e,
            }
        }
        let mut offen = true;
        let mut los = false;
        let mut weg = false;
        // Aus dem Browser-Download: kein weiterer Klick noetig.
        if dlg.auto && !dlg.done && dlg.err.is_empty() && !dlg.busy.load(Ordering::Relaxed) {
            los = true;
        }
        egui::Window::new(i18n::t("setup.title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut offen)
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(theme::card())
                    .stroke(egui::Stroke::new(1.0, theme::accent()))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                if dlg.auto && !dlg.done {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(i18n::t("setup.auto")).size(13.0),
                        );
                    });
                    if !dlg.err.is_empty() {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(&dlg.err)
                                .size(12.0)
                                .color(theme::muted()),
                        );
                    }
                    return;
                }
                if dlg.done {
                    ui.label(
                        egui::RichText::new(i18n::tf("setup.done", &dlg.owner))
                            .size(13.5),
                    );
                    if !dlg.err.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(&dlg.err)
                                .size(12.0)
                                .color(theme::muted()),
                        );
                    }
                    ui.add_space(10.0);
                    if accent_button(ui, "OK", true).clicked() {
                        weg = true;
                    }
                    return;
                }
                ui.label(
                    egui::RichText::new(i18n::t("setup.lead"))
                        .size(12.0)
                        .color(theme::muted()),
                );
                ui.add_space(8.0);
                label_small(ui, i18n::t("setup.name"));
                let enter = ui
                    .add(
                        egui::TextEdit::singleline(&mut dlg.name)
                            .desired_width(240.0)
                            .margin(egui::Margin::symmetric(8, 4)),
                    )
                    .lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if !dlg.err.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(&dlg.err)
                            .size(12.0)
                            .color(theme::muted()),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let busy = dlg.busy.load(Ordering::Relaxed);
                    let label = if busy {
                        i18n::t("setup.working")
                    } else {
                        i18n::t("setup.ok")
                    };
                    if accent_button(ui, label, !busy).clicked() || (enter && !busy) {
                        if dlg.name.trim().is_empty() {
                            dlg.err = i18n::t("setup.noname").to_string();
                        } else {
                            dlg.err.clear();
                            los = true;
                        }
                    }
                    if ghost_button(ui, i18n::t("setup.cancel")).clicked() {
                        weg = true;
                    }
                });
            });
        if los {
            let dlg = self.setup_dlg.as_ref().unwrap();
            let relay = self.shared.relay_url.clone();
            let code = dlg.code.clone();
            let name = presence::clean(&dlg.name);
            let id = self.shared.my_id.lock().unwrap().clone();
            let busy = dlg.busy.clone();
            let out = dlg.out.clone();
            if !busy.swap(true, Ordering::SeqCst) {
                std::thread::spawn(move || {
                    let res = account::setup_claim(&relay, &code, &id, &name)
                        .map_err(|e| e.to_string());
                    *out.lock().unwrap() = Some(res);
                    busy.store(false, Ordering::SeqCst);
                });
            }
        }
        if weg || !offen {
            self.setup_dlg = None;
        }
    }

    fn set_audio(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        let mut look = self.look.clone();
        // Die Geraeteliste einmal holen - cpal fragt dafuer Windows, das
        // kostet je nach Treiber ein paar Millisekunden.
        if self.snd_in.is_empty() && self.snd_out.is_empty() {
            self.snd_in = audio::input_devices();
            self.snd_out = audio::output_devices();
            self.snd_default = audio::default_names();
        }
        let ins = self.snd_in.clone();
        let outs = self.snd_out.clone();
        let defs = self.snd_default.clone();
        card(ui, |ui| {
            label_small(ui, i18n::t("set.audio_default"));
            if check(ui, &mut look.mic_on, i18n::t("set.mic_default"))
                .on_hover_text(i18n::t("set.mic_default_tip"))
                .changed()
            {}
            if check(ui, &mut look.snd_on, i18n::t("set.snd_default"))
                .changed()
            {}
            ui.add_space(6.0);
            divider(ui);
            ui.add_space(5.0);
            label_small(ui, i18n::t("set.audio_now"));
            ui.horizontal(|ui| {
                self.voice_buttons(ui, 18.0);
                ui.add_space(8.0);
                let v = self.shared.voice.clone();
                level_bar(
                    ui,
                    v.level_out.load(Ordering::Relaxed),
                    v.level_in.load(Ordering::Relaxed),
                );
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(i18n::t("set.audio_note"))
                    .size(11.5)
                    .color(p.muted),
            );
            ui.add_space(6.0);
            divider(ui);
            ui.add_space(5.0);
            label_small(ui, i18n::t("set.dev_pick"));
            if let Some(sel) = device_row(
                ui,
                "fv_mic_dev",
                i18n::t("set.mic_dev"),
                &look.mic_dev,
                &defs.0,
                &ins,
            ) {
                look.mic_dev = sel;
            }
            ui.add_space(4.0);
            if let Some(sel) = device_row(
                ui,
                "fv_spk_dev",
                i18n::t("set.spk_dev"),
                &look.spk_dev,
                &defs.1,
                &outs,
            ) {
                look.spk_dev = sel;
            }
            ui.add_space(6.0);
            if ghost_button(ui, i18n::t("set.dev_refresh")).clicked() {
                self.snd_in.clear();
                self.snd_out.clear();
            }
        });
        if look != self.look {
            self.apply_look(ui.ctx(), look);
        }
    }

    fn set_look(&mut self, ui: &mut egui::Ui) {
        let mut a = self.look.clone();

        label_small(ui, i18n::t("set.template"));
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (key, name, pal) in theme::PRESETS.iter() {
                    let sel = a.preset == *key;
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(140.0, 74.0), egui::Sense::click());
                    let pt = ui.painter();
                    pt.rect_filled(rect, 8.0, pal.bg);
                    pt.rect_filled(
                        egui::Rect::from_min_size(
                            rect.min + egui::vec2(9.0, 9.0),
                            egui::vec2(122.0, 18.0),
                        ),
                        5.0,
                        pal.card,
                    );
                    pt.rect_filled(
                        egui::Rect::from_min_size(
                            rect.min + egui::vec2(9.0, 33.0),
                            egui::vec2(54.0, 10.0),
                        ),
                        4.0,
                        pal.accent,
                    );
                    pt.rect_filled(
                        egui::Rect::from_min_size(
                            rect.min + egui::vec2(68.0, 33.0),
                            egui::vec2(38.0, 10.0),
                        ),
                        4.0,
                        pal.card_hi,
                    );
                    pt.text(
                        rect.min + egui::vec2(10.0, 50.0),
                        egui::Align2::LEFT_TOP,
                        name,
                        egui::FontId::proportional(12.0),
                        pal.text,
                    );
                    pt.rect_stroke(
                        rect,
                        8.0,
                        egui::Stroke::new(
                            if sel { 2.0 } else { 1.0 },
                            if sel { theme::accent() } else { theme::line() },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    if resp.clicked() {
                        a.preset = key.to_string();
                    }
                    ui.add_space(6.0);
                }
            });
        });

        ui.add_space(8.0);
        label_small(ui, i18n::t("set.accent"));
        card(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(a.accent.is_none(), i18n::t("set.accent_preset"))
                    .clicked()
                {
                    a.accent = None;
                }
                ui.add_space(6.0);
                for (name, col) in theme::ACCENTS.iter() {
                    let want = [col.r(), col.g(), col.b()];
                    let sel = a.accent == Some(want);
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                    ui.painter().rect_filled(rect, 11.0, *col);
                    if sel {
                        ui.painter().rect_stroke(
                            rect.expand(2.0),
                            13.0,
                            egui::Stroke::new(2.0, theme::text()),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if resp.on_hover_text(*name).clicked() {
                        a.accent = Some(want);
                    }
                    ui.add_space(4.0);
                }
                ui.add_space(6.0);
                let cur = theme::accent();
                let mut rgb = a.accent.unwrap_or([cur.r(), cur.g(), cur.b()]);
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    a.accent = Some(rgb);
                }
            });
        });

        ui.add_space(8.0);
        card(ui, |ui| {
            let pct = format!("{:.0} %", a.scale * 100.0);
            slider_row(ui, i18n::t("set.size"), &mut a.scale, 0.85..=1.35, &pct);
            ui.add_space(6.0);
            let mut r = a.radius as f32;
            let rtxt = format!("{:.0} px", r);
            if slider_row(ui, i18n::t("set.radius"), &mut r, 0.0..=16.0, &rtxt) {
                a.radius = r.round() as u8;
            }
            ui.add_space(6.0);
            if ghost_button(ui, i18n::t("set.reset")).clicked() {
                let lang = a.lang.clone();
                a = theme::Appearance {
                    lang,
                    ..Default::default()
                };
            }
        });

        if a != self.look {
            self.apply_look(ui.ctx(), a);
        }
    }

    /// Update-Karte: Version, Suchen, Automatik.
    /// Rueckmeldung: Fehler oder Idee, direkt aus dem Programm.
    fn set_feedback(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t("fb.intro"))
                    .size(11.5)
                    .color(p.muted),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if icons::text_button(ui, "shield", i18n::t("fb.bug"), self.fb_bug).clicked() {
                    self.fb_bug = true;
                }
                if icons::text_button(ui, "star", i18n::t("fb.idea"), !self.fb_bug).clicked() {
                    self.fb_bug = false;
                }
            });
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::multiline(&mut self.fb_text)
                    .desired_rows(4)
                    .desired_width(ui.available_width().min(430.0))
                    .hint_text(i18n::t("fb.hint")),
            );
            ui.add_space(4.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.fb_contact)
                    .desired_width(260.0)
                    .margin(egui::Margin::symmetric(8, 4))
                    .hint_text(i18n::t("fb.contact")),
            );
            ui.add_space(6.0);
            let busy = feedback::STATE.load(Ordering::Relaxed) == 1;
            ui.horizontal(|ui| {
                if accent_button(ui, i18n::t("fb.send"), !busy && !self.fb_text.trim().is_empty())
                    .clicked()
                {
                    let id = self.shared.my_id.lock().unwrap().clone();
                    let dev = self.shared.device_name.lock().unwrap().clone();
                    feedback::send(
                        if self.fb_bug { "fehler" } else { "idee" },
                        &self.fb_text,
                        &self.fb_contact,
                        &id,
                        &dev,
                    );
                }
                let st = feedback::STATE.load(Ordering::Relaxed);
                if st == 2 {
                    self.fb_text.clear();
                }
                let msg = feedback::MESSAGE.lock().unwrap().clone();
                if !msg.is_empty() {
                    ui.label(
                        egui::RichText::new(msg)
                            .size(11.5)
                            .color(if st == 3 { p.muted } else { p.green }),
                    );
                }
            });
        });
    }

    fn set_about(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                icons::show(ui, "shield", 16.0, theme::green());
                ui.label(
                    egui::RichText::new(i18n::t("set.e2e"))
                        .size(12.5)
                        .color(theme::text()),
                );
            });
            ui.add_space(6.0);
            divider(ui);
            ui.add_space(5.0);
            info_row(ui, i18n::t("set.version"), update::VERSION);
            info_row(ui, i18n::t("set.relay"), &self.shared.relay_url);
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new(i18n::t("set.e2e_note"))
                    .size(11.0)
                    .color(theme::muted()),
            );
            info_row(
                ui,
                i18n::t("set.config"),
                &ident::config_dir().display().to_string(),
            );
            ui.add_space(6.0);
            self.update_ui(ui);
        });
        #[cfg(feature = "license")]
        self.license_card(ui);
    }

    /// Lizenz-Karte in den Einstellungen: Stand anzeigen, Schluessel
    /// eingeben oder entfernen (nur X-Remote).
    #[cfg(feature = "license")]
    fn license_card(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        card(ui, |ui| {
            ui.label(egui::RichText::new(i18n::t("lic.title")).size(13.5).color(theme::text()));
            ui.add_space(4.0);
            match license::status() {
                license::Status::Active(lic) => {
                    info_row(ui, i18n::t("lic.holder"), &lic.name);
                    info_row(ui, i18n::t("lic.expires"), &lic.expiry_text());
                    ui.add_space(4.0);
                    if ghost_button(ui, i18n::t("lic.remove")).clicked() {
                        license::revoke();
                        self.lic_show = true;
                    }
                }
                license::Status::Missing => {
                    ui.label(
                        egui::RichText::new(i18n::t("lic.none"))
                            .size(11.5)
                            .color(theme::muted()),
                    );
                }
                license::Status::Invalid(e) => {
                    ui.label(
                        egui::RichText::new(i18n::tf("lic.invalid", &e))
                            .size(11.5)
                            .color(theme::muted()),
                    );
                }
            }
            if !matches!(license::status(), license::Status::Active(_)) {
                ui.add_space(6.0);
                if accent_button(ui, i18n::t("lic.activate"), false).clicked() {
                    self.lic_show = true;
                }
            }
        });
    }

    /// Aussehen übernehmen: speichern, Stil neu bauen, Titelleiste nachziehen.
    fn apply_look(&mut self, ctx: &egui::Context, a: theme::Appearance) {
        self.look = a.clone();
        theme::save(&a);
        i18n::set_lang(&a.lang);
        theme::apply(ctx, &a);
        chrome::paint_from_theme();
    }

    fn knock_ui(&mut self, ctx: &egui::Context) {
        let knock = self.shared.knock.lock().unwrap().clone();
        let Some(k) = knock else {
            return;
        };
        let left = 60_u64.saturating_sub(k.at.elapsed().as_secs());
        egui::Window::new("Verbindungsanfrage")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::group(&ctx.style())
                    .fill(theme::card())
                    .stroke(egui::Stroke::new(1.0, theme::accent()))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.label(
                    egui::RichText::new(format!("„{}“ möchte sich verbinden", k.from))
                        .size(16.0)
                        .strong(),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    label_small(ui, "Sitzungscode");
                    ui.label(
                        egui::RichText::new(&k.code)
                            .monospace()
                            .color(theme::accent())
                            .size(22.0)
                            .strong(),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "Lass dir den Code nennen – stimmt er überein, redet ihr wirklich miteinander.",
                    )
                    .color(theme::muted())
                    .size(11.5),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, "Zulassen", true).clicked() {
                        self.shared.knock_answer.store(1, Ordering::Relaxed);
                    }
                    if ui.add(ghost(egui::vec2(110.0, 34.0), "Ablehnen")).clicked() {
                        self.shared.knock_answer.store(2, Ordering::Relaxed);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("noch {} s", left))
                                .color(theme::muted())
                                .size(11.5),
                        );
                    });
                });
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// Was in der Bedienleiste angeklickt wurde. Die Leiste gibt es zweimal
    /// (oben im Fenster und schwebend im Vollbild), den Inhalt aber nur
    /// einmal - deshalb sammelt sie ihre Wuensche hier ein.
    fn session_bar(&mut self, ui: &mut egui::Ui, a: &mut BarActs, kompakt: bool) {
        let stats = *self.shared.stats.lock().unwrap();
        let game = self.shared.game_mode();
        let p = theme::palette();

        if kompakt {
            // Griff zum Verschieben
            let (rect, r) = ui.allocate_exact_size(egui::vec2(16.0, 22.0), egui::Sense::drag());
            ui.put(rect, icons::image("grip", 14.0, p.muted));
            if r.dragged() {
                a.drag += r.drag_delta().x;
            }
            let r = r.on_hover_text(i18n::t("sess.move_bar"));
            if r.hovered() || r.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            ui.add_space(2.0);
        }

        if zeigefinger(ui.button(i18n::t("sess.disconnect"))).clicked() {
            a.disconnect = true;
        }
        ui.separator();

        if !kompakt {
            ui.label(i18n::t("sess.mode"));
        }
        if zeigefinger(
            ui.selectable_label(!game, i18n::t("sess.mode_admin"))
                .on_hover_text(i18n::t("sess.mode_admin_tip")),
        )
        .clicked()
            && game
        {
            a.mode = Some(proto::MODE_ADMIN);
        }
        if zeigefinger(
            ui.selectable_label(game, i18n::t("sess.mode_game"))
                .on_hover_text(i18n::t("sess.mode_game_tip")),
        )
        .clicked()
            && !game
        {
            a.mode = Some(proto::MODE_GAME);
        }

        let mons = self.shared.monitors.lock().unwrap().clone();
        if mons.len() > 1 {
            ui.separator();
            let act = self.shared.active_monitor.load(Ordering::Relaxed) as usize;
            let label = |i: usize, m: &proto::MonitorInfo| {
                if kompakt {
                    format!("{}. {}", i + 1, m.name)
                } else {
                    format!("{}. {} ({}x{})", i + 1, m.name, m.w, m.h)
                }
            };
            let cur = mons
                .get(act)
                .map(|m| label(act, m))
                .unwrap_or_else(|| i18n::t("sess.screen").to_string());
            egui::ComboBox::from_id_salt("monitor_pick")
                .selected_text(cur)
                .show_ui(ui, |ui| {
                    for (i, m) in mons.iter().enumerate() {
                        if ui.selectable_label(i == act, label(i, m)).clicked() {
                            a.monitor = Some(i as u8);
                        }
                    }
                });
        }

        // Aufloesung des fernen Bildschirms - der Host stellt um und setzt
        // am Sitzungsende die vorherige wieder.
        ui.separator();
        {
            let (rw, rh) = *self.shared.remote_size.lock().unwrap();
            // Was der ferne Bildschirm wirklich kann (sagt der Host); solange
            // nichts da ist, die gaengigen Stufen.
            let modi = {
                let l = self.shared.remote_resolutions.lock().unwrap().clone();
                if l.is_empty() {
                    vec![
                        (1280u32, 720u32),
                        (1600, 900),
                        (1920, 1080),
                        (2560, 1440),
                        (3440, 1440),
                        (3840, 2160),
                    ]
                } else {
                    l
                }
            };
            egui::ComboBox::from_id_salt("res_pick")
                .selected_text(format!("{}x{}", rw, rh))
                .show_ui(ui, |ui| {
                    for (w, h) in modi {
                        if ui
                            .selectable_label(w == rw && h == rh, format!("{}x{}", w, h))
                            .clicked()
                        {
                            a.res = Some((w, h));
                        }
                    }
                });
        }

        ui.separator();
        self.voice_buttons(ui, 16.0);
        ui.separator();

        let mut want_pick = false;
        let mut want_open = false;
        ui.menu_button(i18n::t("sess.files"), |ui| {
            if ui.button(i18n::t("sess.send_file")).clicked() {
                want_pick = true;
                ui.close();
            }
            if ui.button(i18n::t("sess.open_dir")).clicked() {
                want_open = true;
                ui.close();
            }
            ui.label(
                egui::RichText::new(i18n::t("sess.drop_tip"))
                    .weak()
                    .size(11.0),
            );
        });
        if want_pick {
            a.pick = true;
        }
        if want_open {
            a.open_dir = true;
        }

        ui.separator();
        ui.menu_button(i18n::t("sess.keys"), |ui| {
            for (text, code) in [
                ("Strg+Alt+Entf", proto::SPECIAL_CAD),
                ("Task-Manager (Strg+Shift+Esc)", proto::SPECIAL_TASKMGR),
                ("Windows-Taste", proto::SPECIAL_WIN),
                ("Alt+Tab", proto::SPECIAL_ALTTAB),
                ("Sperren (Win+L)", proto::SPECIAL_LOCK),
            ] {
                if ui.button(text).clicked() {
                    a.special = Some(code);
                    ui.close();
                }
            }
        });

        ui.separator();
        // Vollbild an/aus - im Vollbild zusaetzlich das Anheften
        let (icon, tip) = if self.full {
            ("shrink", i18n::t("sess.full_off"))
        } else {
            ("expand", i18n::t("sess.full_on"))
        };
        if icon_ghost(ui, icon, tip).clicked() {
            a.toggle_full = true;
        }
        if self.full {
            let pinned = self.bar_pinned;
            if icon_toggle(ui, "pin", pinned, 15.0, i18n::t("sess.pin")).clicked() {
                a.toggle_pin = true;
            }
        }

        ui.separator();
        let (rw, rh) = *self.shared.remote_size.lock().unwrap();
        let direct = self.shared.direct.load(Ordering::Relaxed);
        let text = if kompakt {
            format!("{:.0} fps  {:.0} ms", stats.fps, stats.latency_ms)
        } else {
            format!(
                "{}x{}   {:.0} fps   {:.0} kbit/s   {:.0} ms   {}",
                rw,
                rh,
                stats.fps,
                stats.kbps,
                stats.latency_ms,
                if direct {
                    i18n::t("sess.direct")
                } else {
                    i18n::t("sess.via_relay")
                }
            )
        };
        ui.label(text).on_hover_text(format!(
            "{}x{}, {:.0} kbit/s, {}",
            rw,
            rh,
            stats.kbps,
            if direct {
                i18n::t("sess.direct")
            } else {
                i18n::t("sess.via_relay")
            }
        ));

        if !kompakt {
            ui.separator();
            ui.label(egui::RichText::new(i18n::t("sess.escape")).weak().size(11.0))
                .on_hover_text(i18n::t("sess.escape_tip"));
        }
        if game {
            ui.separator();
            if vinput::is_active() {
                ui.colored_label(theme::green(), i18n::t("sess.grabbed"));
            } else {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 190, 90),
                    i18n::t("sess.not_grabbed"),
                );
            }
        }
    }

    /// Vollbild ein- und ausschalten. Im Vollbild verschwindet der Rahmen von
    /// Windows komplett - genau wie bei der Windows-Fernverbindung.
    fn set_full(&mut self, ctx: &egui::Context, on: bool) {
        if self.full == on {
            return;
        }
        self.full = on;
        self.bar_seen = std::time::Instant::now();
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    fn session_ui(&mut self, ctx: &egui::Context) {
        let mut a = BarActs::default();

        // F11 schaltet das Vollbild - die Taste wird hier verbraucht, damit
        // sie nicht auch noch auf dem anderen Rechner landet.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F11)) {
            a.toggle_full = true;
        }

        if self.full {
            // ---- schwebende Leiste (Windows-Fernverbindung laesst gruessen)
            let screen = ctx.screen_rect();
            // Wie bei der Windows-Fernverbindung: die Leiste kommt zurueck,
            // sobald die Maus den oberen Bildschirmrand beruehrt.
            let pointer_top = ctx
                .input(|i| i.pointer.hover_pos())
                .map(|p| p.y <= screen.top() + 3.0)
                .unwrap_or(false);
            if pointer_top {
                self.bar_seen = std::time::Instant::now();
            }
            let offen = self.bar_pinned
                || self.bar_seen.elapsed() < Duration::from_millis(2500);
            let breite = self.bar_w.max(120.0);
            let links = (screen.width() - breite).max(0.0) * self.bar_x.clamp(0.0, 1.0);
            let pos = egui::pos2(screen.left() + links, screen.top() + 2.0);
            let p = theme::palette();

            // Ausgeblendet heisst ausgeblendet: kein Griff, kein Schatten,
            // nichts. Der obere Bildschirmrand ist der Schalter - genau wie
            // bei der Windows-Fernverbindung.
            if offen {
                let area = egui::Area::new(egui::Id::new("fv_fullbar"))
                    .order(egui::Order::Foreground)
                    .fade_in(false)
                    .fixed_pos(pos)
                    .show(ctx, |ui| {
                        egui::Frame::NONE
                            .fill(p.card.gamma_multiply(0.97))
                            .stroke(egui::Stroke::new(1.0, p.line))
                            .corner_radius(9.0)
                            .shadow(egui::epaint::Shadow {
                                offset: [0, 3],
                                blur: 12,
                                spread: 0,
                                color: egui::Color32::from_black_alpha(90),
                            })
                            .inner_margin(egui::Margin::symmetric(8, 4))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    self.session_bar(ui, &mut a, true);
                                });
                            });
                    });
                let bar_rect = area.response.rect;
                if bar_rect.width() > 20.0 {
                    self.bar_w = bar_rect.width();
                }
                if area.response.hovered() || area.response.contains_pointer() {
                    self.bar_seen = std::time::Instant::now();
                }
            }
            if a.drag != 0.0 {
                let span = (screen.width() - self.bar_w).max(1.0);
                self.bar_x = (self.bar_x + a.drag / span).clamp(0.0, 1.0);
                self.bar_seen = std::time::Instant::now();
            }
        } else {
            egui::TopBottomPanel::top("session_bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    self.session_bar(ui, &mut a, false);
                });
            });
        }

        if let Some(until) = self.hint_until {
            if std::time::Instant::now() < until {
                let text = if self.hint.is_empty() {
                    i18n::t("sess.escape_long").to_string()
                } else {
                    self.hint.clone()
                };
                if !self.full {
                    egui::TopBottomPanel::top("escape_hint").show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(text)
                                    .color(egui::Color32::from_rgb(240, 220, 120)),
                            );
                        });
                    });
                }
            } else {
                self.hint_until = None;
                self.hint.clear();
            }
        }

        let mut image_rect: Option<egui::Rect> = None;
        let mut clicked_image = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(18, 18, 18)))
            .show(ctx, |ui| {
                if let Some(tex) = &self.tex {
                    let size_px = tex.size();
                    let (iw, ih) = (size_px[0] as f32, size_px[1] as f32);
                    let avail = ui.available_size();
                    let scale = (avail.x / iw).min(avail.y / ih).max(0.05);
                    let size = egui::vec2(iw * scale, ih * scale);
                    ui.vertical_centered(|ui| {
                        let resp = ui.add(
                            egui::Image::new(egui::load::SizedTexture::new(tex.id(), size))
                                .sense(egui::Sense::click_and_drag()),
                        );
                        clicked_image = resp.clicked();
                        image_rect = Some(resp.rect);
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(i18n::t("sess.waiting"));
                    });
                }
            });

        // Kein neues Bild seit ein paar Sekunden? Dann sagen wir, woran es
        // fast immer liegt, statt nur "0 fps" anzuzeigen.
        if self.frame_at.elapsed() > Duration::from_secs(4) {
            let p = theme::palette();
            egui::Area::new(egui::Id::new("fv_stall"))
                .order(egui::Order::Foreground)
                .fade_in(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 40.0))
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(p.card.gamma_multiply(0.95))
                        .stroke(egui::Stroke::new(1.0, p.line))
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin::symmetric(16, 12))
                        .show(ui, |ui| {
                            ui.set_max_width(460.0);
                            ui.label(
                                egui::RichText::new(i18n::t("sess.stall_head"))
                                    .size(14.0)
                                    .strong(),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(i18n::t("sess.stall_body"))
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        });
                });
        }

        if let Some(rect) = image_rect {
            self.draw_remote_cursor(ctx, rect, game_mode(&self.shared));
            self.update_grab(ctx, rect, game_mode(&self.shared), clicked_image);
            self.forward_input(ctx, rect, game_mode(&self.shared));
        }

        if a.toggle_full {
            let on = !self.full;
            self.set_full(ctx, on);
        }
        if a.toggle_pin {
            self.bar_pinned = !self.bar_pinned;
            self.bar_seen = std::time::Instant::now();
        }
        if let Some(m) = a.mode {
            self.set_mode(m);
        }
        if let Some(i) = a.monitor {
            self.shared.active_monitor.store(i, Ordering::Relaxed);
            self.shared.send_input(Msg::SetMonitor { index: i });
            self.tex = None;
            self.last_seq = 0;
        }
        if let Some((w, h)) = a.res {
            self.shared.send_input(Msg::SetResolution { width: w, height: h });
        }
        if let Some(code) = a.special {
            self.shared.send_input(Msg::Special { code });
        }
        if a.pick {
            self.pick_and_send();
        }
        if a.open_dir {
            self.open_drop_dir();
        }
        if a.disconnect {
            self.set_full(ctx, false);
            self.stop_session();
        }
    }

    /// The duplication API does not paint the cursor into the frame. In game
    /// mode the local pointer is locked and invisible, so the remote one has
    /// to be drawn - in remote maintenance the local pointer sits exactly
    /// where the remote one is and a second arrow only confuses.
    fn draw_remote_cursor(&self, ctx: &egui::Context, rect: egui::Rect, game: bool) {
        if !game {
            return;
        }
        let (x, y, visible) = *self.shared.remote_cursor.lock().unwrap();
        if !visible {
            return;
        }
        let p = egui::pos2(
            rect.left() + rect.width() * (x as f32 / 10000.0).clamp(0.0, 1.0),
            rect.top() + rect.height() * (y as f32 / 10000.0).clamp(0.0, 1.0),
        );
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("remote_cursor"),
        ));
        let pts = vec![
            p,
            egui::pos2(p.x, p.y + 17.0),
            egui::pos2(p.x + 4.5, p.y + 12.5),
            egui::pos2(p.x + 10.5, p.y + 12.0),
        ];
        painter.add(egui::Shape::convex_polygon(
            pts,
            egui::Color32::WHITE,
            egui::Stroke::new(1.2, egui::Color32::BLACK),
        ));
    }

    /// Keeps the pointer lock in sync with focus and clicks.
    fn update_grab(&mut self, ctx: &egui::Context, rect: egui::Rect, game: bool, clicked: bool) {
        // Die Tastatur wird in BEIDEN Betriebsarten komplett uebernommen -
        // Windows-Taste, Alt+Tab, Alt+F4 gehen dann an den anderen Rechner
        // und nicht mehr an das eigene Windows. Rechte Strg holt sie zurueck.
        vinput::set_relative(game);
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if !focused {
            vinput::set_active(false);
            return;
        }
        // center of the picture in physical screen pixels
        let ppp = ctx.pixels_per_point();
        let origin = ctx
            .input(|i| i.viewport().inner_rect)
            .map(|r| r.min)
            .unwrap_or(egui::Pos2::ZERO);
        let c = rect.center();
        vinput::set_center(
            ((origin.x + c.x) * ppp) as i32,
            ((origin.y + c.y) * ppp) as i32,
        );
        if clicked && !vinput::is_active() {
            vinput::set_active(true);
        }
        if vinput::is_active() && game {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
    }

    fn forward_input(&mut self, ctx: &egui::Context, rect: egui::Rect, game: bool) {
        let grabbed = game && vinput::is_active();

        // modifier keys are not part of egui's Key enum, so we diff the state.
        // In game mode the low level hook already forwards everything.
        if !grabbed {
            let mods = ctx.input(|i| i.modifiers);
            if mods.shift != self.last_mods.shift {
                self.shared.send_input(Msg::Key {
                    code: proto::KEY_SHIFT,
                    named: true,
                    down: mods.shift,
                });
            }
            if mods.ctrl != self.last_mods.ctrl {
                self.shared.send_input(Msg::Key {
                    code: proto::KEY_CTRL,
                    named: true,
                    down: mods.ctrl,
                });
            }
            if mods.alt != self.last_mods.alt {
                self.shared.send_input(Msg::Key {
                    code: proto::KEY_ALT,
                    named: true,
                    down: mods.alt,
                });
            }
            self.last_mods = mods;
        }

        let norm = |pos: egui::Pos2| -> Option<(i32, i32)> {
            if rect.width() < 1.0 || rect.height() < 1.0 || !rect.contains(pos) {
                return None;
            }
            let x = ((pos.x - rect.left()) / rect.width() * 10000.0).clamp(0.0, 10000.0) as i32;
            let y = ((pos.y - rect.top()) / rect.height() * 10000.0).clamp(0.0, 10000.0) as i32;
            Some((x, y))
        };

        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::PointerMoved(pos) => {
                    // in game mode the raw delta path owns the mouse
                    if !grabbed {
                        if let Some((x, y)) = norm(pos) {
                            self.shared.send_input(Msg::MouseMove { x, y });
                        }
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let b = match button {
                        egui::PointerButton::Secondary => 1u8,
                        egui::PointerButton::Middle => 2u8,
                        egui::PointerButton::Extra1 => 3u8,
                        egui::PointerButton::Extra2 => 4u8,
                        _ => 0u8,
                    };
                    if grabbed {
                        self.shared.send_input(Msg::MouseButton {
                            button: b,
                            down: pressed,
                        });
                    } else if let Some((x, y)) = norm(pos) {
                        self.shared.send_input(Msg::MouseMove { x, y });
                        self.shared.send_input(Msg::MouseButton {
                            button: b,
                            down: pressed,
                        });
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. } => {
                    let lines = match unit {
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Point => delta.y / 50.0,
                        egui::MouseWheelUnit::Page => delta.y * 3.0,
                    };
                    let l = lines.round() as i32;
                    if l != 0 {
                        self.shared.send_input(Msg::Wheel { lines: l });
                    }
                }
                egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    ..
                } => {
                    if !repeat && !grabbed {
                        if let Some((code, named)) = map_key(key) {
                            self.shared.send_input(Msg::Key {
                                code,
                                named,
                                down: pressed,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn map_key(k: egui::Key) -> Option<(u32, bool)> {
    use egui::Key as K;
    use proto as P;

    let named = match k {
        K::Backspace => Some(P::KEY_BACKSPACE),
        K::Enter => Some(P::KEY_ENTER),
        K::Tab => Some(P::KEY_TAB),
        K::Escape => Some(P::KEY_ESCAPE),
        K::ArrowLeft => Some(P::KEY_LEFT),
        K::ArrowRight => Some(P::KEY_RIGHT),
        K::ArrowUp => Some(P::KEY_UP),
        K::ArrowDown => Some(P::KEY_DOWN),
        K::Delete => Some(P::KEY_DELETE),
        K::Home => Some(P::KEY_HOME),
        K::End => Some(P::KEY_END),
        K::PageUp => Some(P::KEY_PAGEUP),
        K::PageDown => Some(P::KEY_PAGEDOWN),
        K::Insert => Some(P::KEY_INSERT),
        K::Space => Some(P::KEY_SPACE),
        _ => None,
    };
    if let Some(code) = named {
        return Some((code, true));
    }

    let name = k.name();
    // single letters: "A".."Z"
    if name.len() == 1 {
        let c = name.chars().next().unwrap().to_ascii_lowercase();
        return Some((c as u32, false));
    }
    if let Some(d) = name.strip_prefix("Num") {
        if d.len() == 1 {
            return Some((d.chars().next().unwrap() as u32, false));
        }
    }
    if let Some(f) = name.strip_prefix('F') {
        if let Ok(n) = f.parse::<u32>() {
            if (1..=12).contains(&n) {
                return Some((P::KEY_F1 + n - 1, true));
            }
        }
    }

    let ch = match k {
        K::Minus => '-',
        K::Plus => '+',
        K::Equals => '=',
        K::Comma => ',',
        K::Period => '.',
        K::Slash => '/',
        K::Backslash => '\\',
        K::Semicolon => ';',
        K::Colon => ':',
        K::Quote => '\'',
        K::Backtick => '`',
        K::OpenBracket => '[',
        K::CloseBracket => ']',
        _ => return None,
    };
    Some((ch as u32, false))
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(n) = self.meet_demo {
            self.meet_demo_ui(ctx, n);
            self.shot_ui(ctx);
            return;
        }
        // Ohne staendiges Neuzeichnen im Elternfenster bekommt das
        // Meetingfenster keine Rahmen - und --meetshot wartet ewig.
        // Fuer das Bild wird das Meetingfenster ins HAUPTfenster gezeichnet.
        // Ein eigenes Unterfenster liefert beim Bildbefehl nichts zurueck -
        // gemessen: der Screenshot-Ereignis kam dort nie an.
        if self.meet_shot.is_some() {
            self.meet_win_ui(ctx);
            self.shot_ui(ctx);
            return;
        }
        if self.caption_tick.elapsed() > Duration::from_secs(2) {
            self.caption_tick = std::time::Instant::now();
            chrome::paint_from_theme();
        }
        self.tray_ui(ctx);
        // Einrichtungs-Auftrag aus dem Browser-Download: sobald der Host
        // seine ID vom Relay hat, geht alles von selbst (kein Koppeln).
        if let Some((code, name)) = self.auto_setup.clone() {
            let fertige_id = !self.shared.my_id.lock().unwrap().is_empty();
            if fertige_id {
                self.auto_setup = None;
                self.setup_dlg = Some(SetupDlg {
                    code,
                    name: if name.trim().is_empty() {
                        presence::machine_name()
                    } else {
                        name
                    },
                    auto: true,
                    ..Default::default()
                });
            }
        }
        self.update_modal(ctx);
        self.uninstall_modal(ctx);
        #[cfg(feature = "license")]
        self.license_modal(ctx);
        self.knock_ui(ctx);
        self.pw_ask_ui(ctx);
        self.setup_dialog_ui(ctx);
        self.pull_frame(ctx);
        // eframe setzt beim Start (und wenn Windows zwischen hell und dunkel
        // wechselt) sein eigenes Aussehen durch - dann fehlen zum Beispiel die
        // Raender der Eingabefelder. Also nachsehen und gegebenenfalls unser
        // Aussehen wieder aufsetzen.
        if !theme::style_is_ours(ctx) {
            let look = self.look.clone();
            theme::apply(ctx, &look);
        }
        self.handle_drops(ctx);
        self.handle_link(ctx);
        // Konto: Ergebnisse abholen und alle zwei Minuten leise abgleichen
        self.take_sync_result();
        if self.acc.is_some()
            && !self.acc_busy.load(Ordering::Relaxed)
            && std::time::Instant::now() >= self.acc_next
        {
            self.start_sync(false);
        }
        self.transfer_ui(ctx);

        // right Ctrl: 1x hands the input back, 3x leaves the session
        match self.shared.escape.swap(0, Ordering::Relaxed) {
            1 => {
                self.hint = "Eingabe freigegeben - ins Bild klicken uebernimmt wieder".to_string();
                self.hint_until = Some(std::time::Instant::now() + Duration::from_secs(4));
            }
            2 => {
                self.stop_session();
                self.hint = "Sitzung ueber die Host-Taste beendet".to_string();
            }
            _ => {}
        }

        let connected = self.shared.connected.load(Ordering::Relaxed);
        if !connected {
            if self.session.is_some() && !self.shared.connecting.load(Ordering::Relaxed) {
                // the session died on its own - still book the time
                self.close_session();
            }
            if vinput::is_active() {
                vinput::set_active(false);
            }
            // Das Vollbild gehoerte dem Sitzungsfenster, das gerade zugeht -
            // hier nur noch den Merker zuruecksetzen, das Hauptfenster war nie
            // im Vollbild.
            self.full = false;
        }

        // Das Hauptfenster bleibt immer das Hauptfenster: Geraete, Meet und
        // Einstellungen sind auch waehrend einer laufenden Sitzung bedienbar.
        paint_background(ctx);
        self.rail(ctx);
        self.header(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(egui::Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| {
                // Inhalt nicht ueber die ganze Breite ziehen
                let full = ui.available_width();
                let w = full.min(CONTENT_MAX);
                let pad = ((full - w) / 2.0).max(0.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(w);
                        self.home_ui(ui);
                    });
                });
            });
        self.add_dev_ui(ctx);
        ctx.request_repaint_after(Duration::from_millis(250));

        // Die Sitzung laeuft in einem eigenen Betriebssystem-Fenster. Damit
        // bleibt das Hauptfenster benutzbar (zweite Sitzung aufmachen, Datei
        // suchen), und Vollbild betrifft nur den entfernten Bildschirm.
        if connected {
            let title = match self.shared.host_peer.lock().unwrap().clone() {
                s if s.is_empty() => i18n::t("sess.window").to_string(),
                _ => {
                    let id = self.partner_id.clone();
                    if id.is_empty() {
                        i18n::t("sess.window").to_string()
                    } else {
                        format!("{} - {}", i18n::t("sess.window"), partners::pretty_id(&id))
                    }
                }
            };
            let mut closed = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("fv_session"),
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([1280.0, 800.0])
                    .with_min_inner_size([480.0, 320.0]),
                |vctx, _class| {
                    // Ablegen muss HIER gelesen werden - das Sitzungsfenster
                    // hat einen eigenen Eingang, das Hauptfenster bekam die
                    // Dateien nie zu sehen.
                    self.handle_drops(vctx);
                    self.session_ui(vctx);
                    if vctx.input(|i| i.viewport().close_requested()) {
                        closed = true;
                    }
                    vctx.request_repaint_after(Duration::from_millis(8));
                },
            );
            if closed {
                self.stop_session();
            }
        }

        // Das Meeting bekommt wie die Fernsitzung ein eigenes Fenster:
        // Vorbereitung, Einladung und Teilnehmerliste laufen neben dem
        // Hauptfenster her, nicht als Karteikarte darin.
        if self.meet_win.offen {
            let mut closed = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("fv_meet"),
                egui::ViewportBuilder::default()
                    .with_title(i18n::t("meet.window"))
                    .with_inner_size([920.0, 600.0])
                    .with_min_inner_size([560.0, 430.0]),
                |vctx, _class| {
                    self.meet_win_ui(vctx);
                    if vctx.input(|i| i.viewport().close_requested()) {
                        closed = true;
                    }
                    vctx.request_repaint_after(Duration::from_millis(150));
                },
            );
            if closed {
                self.meet_win_schliessen();
            }
        }

        // --shot: ein paar Frames zeichnen lassen, dann ein Bild anfordern
        self.shot_ui(ctx);
    }
}

// --------------------------------------------------------------- Aussehen --

/// Which page the window shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    /// Konferenzen (FreeViewer Meet)
    Meet,
    Start,
    Devices,
    Settings,
}

/// Zustand des eigenen Meet-Fensters (separater Viewport, Zoom-Ablauf).
struct MeetWin {
    offen: bool,
    meeting: Option<meet::Meeting>,
    /// Der Browser mit Bild und Ton laeuft bereits.
    beigetreten: bool,
    /// Wunsch "beitreten" aus dem grossen Knopf - wird eine Zeile weiter
    /// unten in eine echte native Sitzung umgesetzt.
    nativ_start: bool,
    /// Geraetenamen, wie das System sie meldet (Mikrofone, Kameras).
    mics: Vec<String>,
    cams: Vec<String>,
    /// 0 = Browser-Standard, sonst Index+1 in mics/cams.
    mic_sel: usize,
    cam_sel: usize,
    stumm: bool,
    ohne_video: bool,
    /// Einmalig im Hintergrund eingelesen (die Abfrage dauert einen Moment).
    #[allow(clippy::type_complexity)]
    geraete: Arc<std::sync::Mutex<Option<(Vec<String>, Vec<String>, Vec<String>)>>>,
    geraete_geladen: bool,
    geraete_da: bool,
    /// Lautsprecher - nur fuer die Einstellungen IM Meeting; vor dem
    /// Beitritt zeigt der Browser sie auch nicht.
    spks: Vec<String>,
    spk_sel: usize,
    /// Teilnehmerliste vom Meet-Server (alle 5 s nachgeladen).
    tn: Arc<std::sync::Mutex<Vec<meet::Teilnehmer>>>,
    tn_busy: Arc<std::sync::atomic::AtomicBool>,
    tn_next: std::time::Instant,
    /// Kurzer Hinweis im Fenster ("Link kopiert") und wann er kam.
    toast: Option<(String, std::time::Instant)>,
    /// Anzeigename im Meeting (Feld "Dein Name" auf dem Beitritts-Schirm).
    name: String,
}

impl Default for MeetWin {
    fn default() -> Self {
        Self {
            offen: false,
            meeting: None,
            beigetreten: false,
            nativ_start: false,
            mics: Vec::new(),
            cams: Vec::new(),
            mic_sel: 0,
            cam_sel: 0,
            spks: Vec::new(),
            spk_sel: 0,
            stumm: false,
            ohne_video: false,
            geraete: Arc::new(std::sync::Mutex::new(None)),
            geraete_geladen: false,
            geraete_da: false,
            tn: Arc::new(std::sync::Mutex::new(Vec::new())),
            name: String::new(),
            tn_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            tn_next: std::time::Instant::now(),
            toast: None,
        }
    }
}

/// Was in einer Sitzungsleiste angeklickt wurde.
#[derive(Default)]
struct BarActs {
    disconnect: bool,
    mode: Option<u8>,
    monitor: Option<u8>,
    /// Gewuenschte Aufloesung auf dem Host (Breite, Hoehe).
    res: Option<(u32, u32)>,
    special: Option<u8>,
    pick: bool,
    open_dir: bool,
    toggle_full: bool,
    toggle_pin: bool,
    /// Wie weit der Griff der schwebenden Leiste gezogen wurde.
    drag: f32,
}

fn game_mode(shared: &Arc<Shared>) -> bool {
    shared.game_mode()
}

/// Was im "Geraet hinzufuegen"-Fenster steht, solange es offen ist.
#[derive(Clone, Default)]
struct AddDev {
    id: String,
    name: String,
    folder: String,
    password: String,
    /// Warum das Hinzufuegen gerade nicht geht.
    err: String,
}

/// Was im Bearbeiten-Feld eines Geraetes steht, solange es offen ist.
#[derive(Clone, Default)]
struct DevEdit {
    id: String,
    name: String,
    password: String,
    note: String,
    folder: String,
}

/// Zustand des Einrichtungs-Dialogs (freeviewer://setup/<code>).
#[derive(Default)]
struct SetupDlg {
    code: String,
    name: String,
    /// Aus dem Browser-Download: sofort einrichten, ohne noch einmal zu fragen.
    auto: bool,
    busy: Arc<std::sync::atomic::AtomicBool>,
    out: Arc<std::sync::Mutex<Option<Result<account::SetupClaimReply, String>>>>,
    err: String,
    owner: String,
    done: bool,
}

/// Bereiche innerhalb der Einstellungen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    Access,
    Account,
    Deploy,
    Audio,
    Look,
    Feedback,
    About,
}


/// Breite, ab der der Inhalt nicht weiter auseinandergezogen wird.
const CONTENT_MAX: f32 = 1180.0;

/// Schriften. Auf Windows sieht die mitgelieferte Segoe UI richtig aus, auf
/// dem Mac die System-Schrift, unter Linux DejaVu/Noto. Findet sich nichts,
/// bleibt die in egui eingebaute Schrift.
///
/// WICHTIG: Die Familie "head" MUSS am Ende immer mit mindestens einer
/// Schrift belegt sein. Bis v0.26.1 wurde sie nur angelegt, wenn eine
/// Windows-Datei gefunden wurde - auf Mac und Linux war sie leer und egui
/// brach beim ersten Zeichnen einer Überschrift ab
/// ("FontFamily::Name(\"head\") is not bound to any fonts").
/// Genau das war der Absturz "X-Remote quit unexpectedly" auf dem Mac.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Kandidaten pro Betriebssystem, in der Reihenfolge der Vorliebe.
    #[cfg(windows)]
    let (regular, bold): (&[&str], &[&str]) = (
        &["C:/Windows/Fonts/segoeui.ttf", "C:/Windows/Fonts/SegUIVar.ttf"],
        &["C:/Windows/Fonts/seguisb.ttf", "C:/Windows/Fonts/segoeuib.ttf"],
    );
    #[cfg(target_os = "macos")]
    let (regular, bold): (&[&str], &[&str]) = (
        &[
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/SFNSText.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
            "/Library/Fonts/Arial.ttf",
        ],
        &[
            "/System/Library/Fonts/SFNSRounded.ttf",
            "/System/Library/Fonts/SFNSDisplay.ttf",
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ],
    );
    #[cfg(all(unix, not(target_os = "macos")))]
    let (regular, bold): (&[&str], &[&str]) = (
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        ],
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        ],
    );

    let mut body: Option<&str> = None;
    let mut head: Option<&str> = None;
    for (key, files) in [("ui", regular), ("ui_bold", bold)] {
        for f in files {
            if let Ok(bytes) = std::fs::read(f) {
                fonts.font_data.insert(
                    key.to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(bytes)),
                );
                if key == "ui" {
                    body = Some(key);
                } else {
                    head = Some(key);
                }
                break;
            }
        }
    }
    if let Some(b) = body {
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, b.to_owned());
    }

    // Die Überschriften-Familie bekommt IMMER einen Inhalt: erst die fette
    // Schrift (falls gefunden), danach die komplette Liste der normalen
    // Schriften als Rückfallebene. Damit kann sie nie leer sein.
    let fallback = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let liste = fonts
        .families
        .entry(egui::FontFamily::Name("head".into()))
        .or_insert_with(|| fallback.clone());
    if let Some(h) = head {
        liste.insert(0, h.to_owned());
    }
    if liste.is_empty() {
        // Letzte Rettung - darf eigentlich nie passieren.
        liste.extend(fallback.iter().cloned());
        if liste.is_empty() {
            liste.push("Ubuntu-Light".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

/// Kräftige Schrift für Überschriften, sonst die normale.
fn head_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("head".into()))
}

fn install_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    // SVG-Symbole: der Bildlader von egui_extras rastert sie in der Groesse,
    // in der sie gebraucht werden.
    egui_extras::install_image_loaders(ctx);
    let look = theme::load();
    i18n::set_lang(&look.lang);
    theme::apply(ctx, &look);
}
/// Farbnebel und feines Raster hinter allem - derselbe Trick wie auf
/// fleitec-Seiten, nur eben gemalt statt per CSS.
fn paint_background(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::background());
    let r = ctx.screen_rect();
    painter.rect_filled(r, 0.0, theme::bg());

    if !theme::palette().orbs {
        return;
    }
    // zwei weiche Farborbs (viele Kreise mit wenig Deckkraft = Verlauf)
    for (rel_x, rel_y, radius, color) in [
        (0.18, 0.05, 0.55, theme::accent()),
        (0.92, 0.75, 0.50, theme::violet()),
    ] {
        let center = egui::pos2(r.left() + r.width() * rel_x, r.top() + r.height() * rel_y);
        let max = r.width().max(r.height()) * radius;
        let steps = 14;
        for i in 0..steps {
            let f = 1.0 - i as f32 / steps as f32;
            painter.circle_filled(center, max * f, color.gamma_multiply(0.012));
        }
    }

    // 64px-Raster, das nach unten ausblendet
    let step = 64.0;
    let mut y = r.top();
    while y < r.bottom() {
        let fade = (1.0 - (y - r.top()) / r.height()).clamp(0.0, 1.0);
        painter.hline(
            r.left()..=r.right(),
            y,
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha((10.0 * fade) as u8)),
        );
        y += step;
    }
    let mut x = r.left();
    while x < r.right() {
        painter.vline(
            x,
            r.top()..=r.bottom(),
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(5)),
        );
        x += step;
    }
}

/// Marke oben links: abgerundetes Quadrat mit Auge.
fn logo(ui: &mut egui::Ui) {
    // Das echte FreeViewer-Logo - dasselbe Bild wie auf der Webseite und als
    // Fenstersymbol. Einmal als Textur laden, danach nur noch malen.
    let id = egui::Id::new("fv_logo_tex");
    let tex = ui
        .ctx()
        .data_mut(|d| d.get_temp::<egui::TextureHandle>(id))
        .unwrap_or_else(|| {
            let bild = image::load_from_memory(include_bytes!("../assets/icon.png"))
                .map(|i| i.to_rgba8())
                .unwrap_or_else(|_| {
                    image::RgbaImage::from_pixel(1, 1, image::Rgba([0x38, 0xbd, 0xf8, 0xff]))
                });
            let (w, h) = bild.dimensions();
            let ci =
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &bild.into_raw());
            let t = ui
                .ctx()
                .load_texture("fv_logo", ci, egui::TextureOptions::LINEAR);
            ui.ctx().data_mut(|d| d.insert_temp(id, t.clone()));
            t
        });
    ui.image((tex.id(), egui::vec2(34.0, 34.0)));
}

/// Reiter in der Kopfzeile.
fn tab(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    let label = egui::RichText::new(text)
        .size(13.5)
        .color(if selected { theme::bg() } else { theme::muted() });
    let btn = egui::Button::new(label)
        .fill(if selected {
            theme::accent()
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::NONE)
        .corner_radius(9)
        .min_size(egui::vec2(0.0, 24.0));
    ui.add(btn)
}

/// Überschrift über einer Karte.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(
        egui::RichText::new(title)
            .font(head_font(15.5))
            .color(theme::text()),
    );
    ui.add_space(3.0);
}

/// Kleine graue Beschriftung über einem Feld.
fn label_small(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(11.5).color(theme::muted()));
    ui.add_space(2.0);
}

/// Dünne Trennlinie innerhalb einer Karte.
fn divider(ui: &mut egui::Ui) {
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(
        r.left()..=r.right(),
        r.top(),
        egui::Stroke::new(1.0, theme::line()),
    );
    ui.add_space(1.0);
}

/// Eine Karte im Hausstil.
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .fill(theme::card())
        .stroke(egui::Stroke::new(1.0, theme::line()))
        .corner_radius(13)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Der eine blaue Knopf.
fn accent_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(
        egui::RichText::new(text)
            .size(14.5)
            .color(if enabled { theme::bg() } else { theme::muted() }),
    )
    .fill(if enabled { theme::accent() } else { theme::card_hi() })
    .stroke(egui::Stroke::NONE)
    .corner_radius(10)
    .min_size(egui::vec2(200.0, 31.0));
    ui.add_enabled(enabled, btn)
}

/// Zweitrangiger Knopf mit Rand.
fn ghost(size: egui::Vec2, text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text).size(13.5).color(theme::text()))
        .fill(theme::card_hi())
        .stroke(egui::Stroke::new(1.0, theme::line()))
        .corner_radius(10)
        .min_size(size)
}

/// Kleiner zweitrangiger Knopf.
fn ghost_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    zeigefinger(ui.add(
        egui::Button::new(egui::RichText::new(text).size(12.5).color(theme::muted()))
            .fill(theme::card_hi())
            .stroke(egui::Stroke::new(1.0, theme::line()))
            .corner_radius(8),
    ))
}

/// Grüner oder grauer Punkt vor einem Gerät.
fn dot(ui: &mut egui::Ui, online: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    let c = if online { theme::green() } else { theme::muted().gamma_multiply(0.55) };
    if online {
        ui.painter().circle_filled(rect.center(), 6.5, c.gamma_multiply(0.25));
    }
    ui.painter().circle_filled(rect.center(), 4.0, c);
}

/// Zustand des eigenen Rechners, oben rechts.
fn status_pill(ui: &mut egui::Ui, ok: bool, text: &str) {
    egui::Frame::group(ui.style())
        .fill(if ok {
            theme::green().gamma_multiply(0.14)
        } else {
            theme::card_hi()
        })
        .stroke(egui::Stroke::new(
            1.0,
            if ok { theme::green().gamma_multiply(0.6) } else { theme::line() },
        ))
        .corner_radius(12)
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(11.5)
                    .color(if ok { theme::green() } else { theme::muted() }),
            );
        });
}

/// „Schlüssel   Wert“-Zeile in der Info-Karte.
fn info_row(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(key).color(theme::muted()).size(11.5));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(value).size(12.5))
                    .selectable(true)
                    .truncate(),
            );
        });
    });
    ui.add_space(2.0);
}

/// Fenstersymbol: dasselbe Auge wie im Infobereich, 32x32 RGBA.
fn app_icon() -> egui::IconData {
    // Dieselbe Zeichnung, die auch in der EXE steckt (siehe build.rs), damit
    // Fenster, Taskleiste, Startmenue und Explorer dasselbe zeigen.
    const RAW: &[u8] = include_bytes!("../assets/icon.png");
    match image::load_from_memory(RAW) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            egui::IconData {
                rgba: rgba.into_raw(),
                width: w,
                height: h,
            }
        }
        Err(_) => egui::IconData {
            rgba: vec![0x38, 0xbd, 0xf8, 0xff],
            width: 1,
            height: 1,
        },
    }
}

/// Speichert ein Bild der Oberflaeche als JPEG (fuer --shot).
fn save_shot(img: &egui::ColorImage, path: &std::path::Path) {
    use image::ImageEncoder;
    let [w, h] = img.size;
    let mut rgb = Vec::with_capacity(w * h * 3);
    for p in &img.pixels {
        rgb.extend_from_slice(&[p.r(), p.g(), p.b()]);
    }
    match std::fs::File::create(path) {
        Ok(mut f) => {
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 92);
            match enc.write_image(&rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8) {
                Ok(()) => println!("SHOT {}x{} -> {}", w, h, path.display()),
                Err(e) => eprintln!("SHOT FAIL: {}", e),
            }
        }
        Err(e) => eprintln!("SHOT FAIL: {}", e),
    }
}

// ------------------------------------------------------------ kleine Helfer

/// Symbolknopf ohne Rahmen - Hover und Druck kommen aus der Palette.
fn zeigefinger(r: egui::Response) -> egui::Response {
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn icon_ghost(ui: &mut egui::Ui, icon: &str, tip: &str) -> egui::Response {
    let r = ui
        .scope(|ui| {
            let v = ui.visuals_mut();
            v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
            v.widgets.active.bg_stroke = egui::Stroke::NONE;
            ui.add(
                egui::Button::image(icons::image(icon, 15.0, theme::muted()))
                    .corner_radius(7)
                    .min_size(egui::vec2(28.0, 24.0)),
            )
        })
        .inner;
    zeigefinger(r).on_hover_text(tip)
}

/// Ein- und ausschaltbarer Symbolknopf (Mikrofon, Ton).
fn icon_toggle(ui: &mut egui::Ui, icon: &str, on: bool, size: f32, tip: &str) -> egui::Response {
    let col = if on { theme::accent() } else { theme::muted() };
    let r = ui
        .scope(|ui| {
            let v = ui.visuals_mut();
            v.widgets.inactive.weak_bg_fill = if on {
                theme::accent().gamma_multiply(0.16)
            } else {
                egui::Color32::TRANSPARENT
            };
            v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
            v.widgets.active.bg_stroke = egui::Stroke::NONE;
            ui.add(
                egui::Button::image(icons::image(icon, size, col))
                    .corner_radius(8)
                    .min_size(egui::vec2(size + 14.0, size + 11.0)),
            )
        })
        .inner;
    zeigefinger(r).on_hover_text(tip)
}

/// Zwei kleine Pegelbalken: raus und rein.
fn level_bar(ui: &mut egui::Ui, out: u32, inn: u32) {
    let p = theme::palette();
    for (val, col) in [(out, p.accent), (inn, p.green)] {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 6.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 3.0, p.card_hi);
        let w = rect.width() * (val.min(100) as f32 / 100.0);
        if w > 0.5 {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(w, rect.height())),
                3.0,
                col,
            );
        }
        ui.add_space(4.0);
    }
}

/// Monitor oder Laptop - je nachdem, wie das Geraet heisst.
fn device_symbol(label: &str) -> &'static str {
    let l = label.to_lowercase();
    if l.contains("laptop") || l.contains("book") || l.contains("note") {
        "laptop"
    } else {
        "monitor"
    }
}

/// Offline steht grau da, online gruen bzw. in normaler Schrift.
fn row_color(online: bool, is_text: bool) -> egui::Color32 {
    let p = theme::palette();
    if online {
        if is_text {
            p.text
        } else {
            p.green
        }
    } else if is_text {
        p.muted.gamma_multiply(0.75)
    } else {
        p.muted.gamma_multiply(0.55)
    }
}


// ============================================================ Bedienelemente
//
// egui bringt eigene Kaestchen und Schieber mit, die sich aber mit der
// eingestellten Rundung zu Punkten verformen und im hellen Modus kaum zu
// sehen sind. Darum zeichnen wir beides selbst - eckig, mit sichtbarem Rand
// und einer Reaktion auf die Maus.

/// Eckiges Kaestchen mit Haken.
fn check(ui: &mut egui::Ui, on: &mut bool, text: &str) -> egui::Response {
    let p = theme::palette();
    let side = 17.0;
    let gap = 9.0;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, p.text);
    let size = egui::vec2(side + gap + galley.size().x, side.max(galley.size().y));
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let hov = resp.hovered();
    let br = egui::Rect::from_min_size(
        egui::pos2(rect.min.x, rect.center().y - side / 2.0),
        egui::vec2(side, side),
    );
    let pt = ui.painter();
    let idle_fill = if p.dark { p.field } else { p.card_hi };
    let idle_edge = if p.dark {
        p.line
    } else {
        // im Hellen reicht die Haarlinie nicht - der Rand muss tragen
        p.muted.gamma_multiply(0.85)
    };
    let fill = if *on {
        p.accent
    } else if hov {
        p.accent.gamma_multiply(0.12)
    } else {
        idle_fill
    };
    let edge = if *on || hov { p.accent } else { idle_edge };
    pt.rect_filled(br, 3.0, fill);
    pt.rect_stroke(
        br,
        3.0,
        egui::Stroke::new(if hov { 2.0 } else { 1.4 }, edge),
        egui::StrokeKind::Inside,
    );
    if *on {
        let c = br.center();
        let s = side * 0.30;
        pt.add(egui::Shape::line(
            vec![
                egui::pos2(c.x - s, c.y),
                egui::pos2(c.x - s * 0.15, c.y + s * 0.8),
                egui::pos2(c.x + s, c.y - s * 0.75),
            ],
            egui::Stroke::new(2.2, p.on_accent),
        ));
    }
    let ty = rect.center().y - galley.size().y / 2.0;
    pt.galley(egui::pos2(br.max.x + gap, ty), galley, p.text);
    zeigefinger(resp)
}

/// Schieber mit Beschriftung links und Wert rechts - nichts rutscht mehr
/// ineinander wie bei "Schriftgroesse 100 %" und "Rundung".
fn slider_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    shown: &str,
) -> bool {
    let p = theme::palette();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(108.0, 22.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(egui::RichText::new(label).size(12.5).color(p.muted));
            },
        );
        let w = (ui.available_width() - 66.0).clamp(90.0, 460.0);
        changed = ui
            .add_sized(
                egui::vec2(w, 22.0),
                egui::Slider::new(value, range).show_value(false),
            )
            .changed();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(shown).size(12.5).color(p.text));
        });
    });
    changed
}

/// Auswahlliste mit Beschriftung links. Gibt den neuen Wert zurueck, wenn
/// sich etwas geaendert hat. `None` in der Liste = Systemstandard.
fn device_row(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    current: &Option<String>,
    default_name: &str,
    list: &[String],
) -> Option<Option<String>> {
    let p = theme::palette();
    let mut picked: Option<Option<String>> = None;
    let shown = match current {
        Some(n) => n.clone(),
        None => format!("{} ({})", i18n::t("set.dev_default"), default_name),
    };
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(108.0, 22.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(egui::RichText::new(label).size(12.5).color(p.muted));
            },
        );
        egui::ComboBox::from_id_salt(id)
            .selected_text(egui::RichText::new(shown).size(12.5))
            .width((ui.available_width() - 8.0).clamp(160.0, 420.0))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(
                        current.is_none(),
                        format!("{} ({})", i18n::t("set.dev_default"), default_name),
                    )
                    .on_hover_text(i18n::t("set.dev_default_tip"))
                    .clicked()
                {
                    picked = Some(None);
                }
                for name in list.iter() {
                    if ui
                        .selectable_label(current.as_deref() == Some(name.as_str()), name)
                        .clicked()
                    {
                        picked = Some(Some(name.clone()));
                    }
                }
            });
    });
    // Ein gewaehltes Geraet, das nicht mehr da ist, muss man sehen.
    if let Some(n) = current {
        if !list.iter().any(|x| x == n) && !list.is_empty() {
            ui.label(
                egui::RichText::new(format!("{} - {}", n, i18n::t("set.dev_gone")))
                    .size(11.0)
                    .color(theme::palette().violet),
            );
        }
    }
    picked
}