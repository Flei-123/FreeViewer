#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! FreeViewer - open source remote desktop.
//!
//! One binary does both jobs, like TeamViewer: it registers this machine at the
//! relay (so others can connect to your ID) and it can connect to another ID.
//!
//! Relay can be overridden with the FV_RELAY environment variable,
//! the session password with FV_PASSWORD.

mod audio;
mod authguard;
mod brand;
mod autostart;
mod capture;
mod chrome;
mod clip;
mod crypto;
mod encoder;
mod feedback;
mod freunde;
mod h264;
mod hostside;
mod i18n;
mod account;
mod icons;
mod ident;
#[cfg(feature = "license")]
mod license;
mod link;
mod machineacl;
mod meet;
mod meetsig;
mod meete2e;
mod meetrtc;
mod meetaudio;
mod meetui;
mod meetfenster;
mod meetvideo;
#[cfg(windows)]
mod camdshow;
mod meetcam;
mod avatar;
mod fenster;
mod meetschirm;
mod input;
mod net;
mod p2p;
mod res;
mod partners;
mod presence;
mod pwlist;
mod secstore;
mod proto;
mod selftest;
mod service;
mod setup;
mod shared;
mod theme;
mod tray;
mod testhost;
mod update;
mod viewer;
mod vinput;
mod xfer;
#[cfg(test)]
mod vectors;
mod cli_meet;
mod cli_session;
mod cli_tools;
mod startup;
mod widgets;
mod app_frame;
mod app_core;
mod app_start;
mod app_meet;
mod app_meetrun;
mod app_friends;
mod app_devices;
mod app_settings;
mod app_account;
mod app_modals;
mod app_settings_tabs;
mod app_session;

use app_frame::*;
use cli_meet::*;
use cli_session::*;
use cli_tools::*;
use startup::*;
use widgets::*;

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

    if let Some(i) = std::env::args().position(|a| a == "--apply-update") {
        return cli_apply_update(i);
    }

    if std::env::args().any(|a| a == "--install") {
        return cli_install();
    }
    if std::env::args().any(|a| a == "--uninstall") {
        return cli_uninstall();
    }

    if std::env::args().any(|a| a == "--version" || a == "-V") {
        return cli_version();
    }

    if std::env::args().any(|a| a == "--audiodev") {
        return cli_audiodev();
    }
    if let Some(i) = std::env::args().position(|a| a == "--audioloop") {
        return cli_audioloop(i);
    }
    if std::env::args().any(|a| a == "--e2evektor") {
        return cli_e2evektor();
    }

    if let Some(i) = std::env::args().position(|a| a == "--schirmshot") {
        return cli_schirmshot(i);
    }

    if let Some(i) = std::env::args().position(|a| a == "--camtest") {
        return cli_camtest(i);
    }

    if std::env::args().any(|a| a == "--kameraliste") {
        return cli_kameraliste();
    }

    if let Some(i) = std::env::args().position(|a| a == "--meetalles") {
        return cli_meetalles(i);
    }

    if let Some(i) = std::env::args().position(|a| a == "--meetsteuer") {
        return cli_meetsteuer(i);
    }

    if let Some(i) = std::env::args().position(|a| a == "--meetempfang") {
        return cli_meetempfang(i);
    }

    if let Some(i) = std::env::args().position(|a| a == "--meetschirm") {
        return cli_meetschirm(i);
    }

    if let Some(i) = std::env::args().position(|a| a == "--meetkamera") {
        return cli_meetkamera(i);
    }

    if let Some(i) = std::env::args().position(|a| a == "--meetbild") {
        return cli_meetbild(i);
    }

    if let Some(i) = std::env::args().position(|a| a == "--meetmik") {
        return cli_meetmik(i);
    }

    if let Some(i) = std::env::args().position(|a| a == "--meetton") {
        return cli_meetton(i);
    }

    if let Some(i) = std::env::args().position(|a| a == "--meettest") {
        return cli_meettest(i);
    }

    if std::env::args().any(|a| a == "--uitest") {
        return cli_uitest(shared);
    }

    if std::env::args().any(|a| a == "--synctest") {
        return cli_synctest(shared);
    }

    if std::env::args().any(|a| a == "--service") {
        return cli_service();
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
    if std::env::args().any(|a| a == "--status") {
        return cli_status();
    }
    if let Some(pos) = std::env::args().position(|a| a == "--autostart") {
        return cli_autostart(pos);
    }

    if std::env::args().any(|a| a == "--update") {
        return cli_update();
    }

    if std::env::args().any(|a| a == "--monitors") {
        return cli_monitors();
    }

    if std::env::args().any(|a| a == "--captest") {
        return cli_captest();
    }

    if std::env::args().any(|a| a == "--gputest") {
        return cli_gputest();
    }

    if std::env::args().any(|a| a == "--h264test") {
        return cli_h264test();
    }

    if std::env::args().any(|a| a == "--videotest") {
        return cli_videotest();
    }

    if std::env::args().any(|a| a == "--p2ptest") {
        return cli_p2ptest();
    }

    if std::env::args().any(|a| a == "--deltatest") {
        return cli_deltatest();
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
    // The Mac may be host once the application is signed with a Developer ID
    // - without it macOS 15 does not grant the necessary permissions
    // permanently (they would be gone again with every self-update). We start
    // the host anyway and tell the user which two checkmarks they need.
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
        // Take over ID and password from the service agent as soon as it
        // publishes them - regardless of whether the service was already running at start or
        // the UI was replaced only later. Only fresh values
        // (< 90 s) count, so that an old state.json does not overwrite anything.
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
        // The password only goes to the console when explicitly asked for
        // (logs and process lists tend to end up in places they should not).
        if std::env::args().any(|a| a == "--show-password") {
            println!(
                "password = {}",
                shared.password.lock().unwrap_or_else(|p| p.into_inner())
            );
        } else {
            println!("password hidden (start with --show-password to print it)");
        }
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
        return cli_connect(pos, argv, shared);
    }

    if let Some(pos) = argv.iter().position(|a| a == "--ask") {
        return cli_ask(pos, argv, shared);
    }

    // scripted input self test:  freeviewer --inputtest <id> <password>
    if let Some(pos) = argv.iter().position(|a| a == "--inputtest") {
        selftest::input_selftest(shared.clone(), &argv, pos);
    }

    vinput::init(shared.clone());
    // If the service is installed, IT does the update (as SYSTEM, without
    // a prompt). UI and agent then only check and report it -
    // otherwise both would constantly ask for administrator rights.
    update::watcher(shared.clone(), !service::installed());

    if let Some(i) = std::env::args().position(|a| a == "--shot") {
        return cli_shot(i, shared);
    }

    // Started by the autostart entry (or by the service): no window in the
    // user's face, only the tray icon. The host runs either way.
    let start_hidden = std::env::args().any(|a| a == "--tray" || a == "--background");
    // Did a freeviewer:// address come along? Then put it in the mailbox -
    // regardless of whether this version picks it up itself or the already running one.
    if let Some(url) = std::env::args().find(|a| link::is_ours(a)) {
        if link::parse(&url).is_some() {
            link::drop_in(&url);
        }
    }

    // Freshly downloaded from the browser? Then the setup job
    // (code + desired name) sits at the end of the file - the device sets itself
    // up as soon as the UI knows its ID.
    let embedded = if setup::running_installed() {
        None
    } else {
        link::embedded_setup()
    };

    // A second window of the same user is pointless - bring the first one to
    // the front instead. The service agent does not count as a second window,
    // it deliberately runs next to the UI.
    if !tray::claim_single_instance(is_agent) {
        if !is_agent {
            println!("{} laeuft bereits - Fenster nach vorne geholt.", crate::brand::NAME);
        }
        return Ok(());
    }
    // Register the scheme for this user (portable, without admin rights).
    // In a proper installation the machine-wide entry
    // is added as well.
    if !link::points_here() {
        let _ = link::register(false);
    }
    autostart::refresh();
    if is_agent {
        // After an update: if a window was open before, it is reopened
        // here. The service cannot do that itself - it sits in
        // session 0 and would end up on an invisible desktop.
        update::gui_wieder_oeffnen();
    }
    if is_agent {
        // Two identical tray icons (agent + UI) are useless:
        // the agent shows its own only while no UI is running.
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
        // Tell the service (session 0) that a window is open here.
        // Without this marker it does not know after the update whether it should
        // reopen the UI.
        update::gui_marke_pflegen();
    }

    // FV_RENDERER=dx forces DirectX from the start (for the hotline:
    // anyone who knows their machine has no OpenGL saves themselves the two
    // failed attempts). Anything else takes the normal path.
    let dx_erzwungen = std::env::var("FV_RENDERER")
        .map(|v| {
            let v = v.to_lowercase();
            v == "dx" || v == "directx" || v == "wgpu"
        })
        .unwrap_or(false);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([700.0, 470.0])
            .with_visible(!start_hidden)
            .with_icon(app_icon())
            .with_title(crate::brand::NAME),
        renderer: if dx_erzwungen {
            eframe::Renderer::Wgpu
        } else {
            eframe::Renderer::Glow
        },
        ..Default::default()
    };

    // Second attempt without hardware acceleration: some servers do have
    // OpenGL, but no hardware context. Better to draw slowly than not
    // start at all.
    let shared2 = shared.clone();
    let shared3 = shared.clone();
    let shared4 = shared.clone();
    let embedded2 = embedded.clone();
    let embedded3 = embedded.clone();
    let bauen = move |cc: &eframe::CreationContext<'_>| {
        install_theme(&cc.egui_ctx);
        Ok(Box::new(App::new(shared, start_hidden, embedded.clone())) as Box<dyn eframe::App>)
    };
    // The watchdog: `run_native` hangs on machines without usable
    // graphics output instead of returning an error - a return value
    // that never comes cannot be evaluated.
    {
        let wach = shared3.clone();
        std::thread::spawn(move || {
            let frist = Duration::from_secs(
                std::env::var("FV_FENSTER_FRIST_SEK")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(25),
            );
            std::thread::sleep(frist);
            if BILD_GEZEICHNET.load(Ordering::Relaxed) {
                return;
            }
            // The attempts (OpenGL, OpenGL without acceleration, DirectX)
            // report themselves when they are all through - then the main path
            // takes care of the message. The watchdog is ONLY there for
            // the case that `run_native` hangs and never returns.
            if ANLAEUFE_FERTIG.load(Ordering::Relaxed) {
                return;
            }
            start_fehler_melden(&format!(
                "Das Fenster kam auch nach {} Sekunden nicht hoch (kein Bild gezeichnet)",
                frist.as_secs()
            ));
            ohne_fenster_weiterlaufen(&wach);
        });
    }
    let gl_kaputt_simulieren = std::env::var("FV_TEST_GL_KAPUTT").as_deref() == Ok("1");
    let erst = if gl_kaputt_simulieren {
        Err(eframe::Error::AppCreation(
            "TEST: OpenGL kuenstlich abgeschaltet".into(),
        ))
    } else {
        eframe::run_native(crate::brand::NAME, options, Box::new(bauen))
    };
    let Err(e1) = erst else {
        return Ok(());
    };
    capture::log_line(&format!("Oberflaeche startet nicht ({}) - zweiter Anlauf ohne Hardwarebeschleunigung", e1));
    let optionen2 = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([700.0, 470.0])
            .with_visible(!start_hidden)
            .with_icon(app_icon())
            .with_title(crate::brand::NAME),
        hardware_acceleration: eframe::HardwareAcceleration::Off,
        ..Default::default()
    };
    let zweit = if gl_kaputt_simulieren {
        Err(eframe::Error::AppCreation(
            "TEST: OpenGL kuenstlich abgeschaltet".into(),
        ))
    } else {
        eframe::run_native(
            crate::brand::NAME,
            optionen2,
            Box::new(move |cc| {
                install_theme(&cc.egui_ctx);
                Ok(Box::new(App::new(shared2, start_hidden, embedded2.clone()))
                    as Box<dyn eframe::App>)
            }),
        )
    };
    let e2 = match zweit {
        Ok(()) => return Ok(()),
        Err(e2) => e2,
    };

    // THIRD ATTEMPT: DIRECTX INSTEAD OF OPENGL.
    //
    // This is exactly where the customer case of 20.08.2026 lay (start.log:
    // "egui_glow requires opengl 2.0+"). Windows ships ONLY
    // OpenGL 1.1 by itself (the "GDI Generic" renderer) - on servers, in
    // remote desktop sessions and in virtual machines without a
    // graphics driver that is all there is. Both OpenGL attempts
    // must fail there, also the one without hardware acceleration:
    // "no acceleration" still means OpenGL.
    //
    // Direct3D is the Windows-native interface and is ALWAYS there: if
    // there is no real graphics card, Microsoft's own software
    // rasterizer WARP steps in ("Microsoft Basic Render Driver"). That is why
    // this attempt draws a window even on a bare VM.
    capture::log_line(&format!(
        "OpenGL geht nicht ({}) - dritter Anlauf mit DirectX (wgpu)",
        e2
    ));
    let optionen3 = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([700.0, 470.0])
            .with_visible(!start_hidden)
            .with_icon(app_icon())
            .with_title(crate::brand::NAME),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let dritt = eframe::run_native(
        crate::brand::NAME,
        optionen3,
        Box::new(move |cc| {
            install_theme(&cc.egui_ctx);
            Ok(Box::new(App::new(shared4, start_hidden, embedded3.clone())) as Box<dyn eframe::App>)
        }),
    );
    match dritt {
        Ok(()) => Ok(()),
        Err(e3) => {
            // No window - but the machine should still be reachable.
            // On a server that is exactly the actual purpose.
            ANLAEUFE_FERTIG.store(true, Ordering::Relaxed);
            start_fehler_melden(&format!(
                "{} | ohne Hardwarebeschleunigung: {} | DirectX: {}",
                e1, e2, e3
            ));
            ohne_fenster_weiterlaufen(&shared3);
        }
    }
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
    /// Friends list including sync with the relay. Only started on the first
    /// look at the page - whoever never opens it should not
    /// cause background traffic either.
    freunde: Option<std::sync::Arc<freunde::Dienst>>,
    /// Eingabefeld "FreeViewer-Nummer des Freundes".
    freunde_id: String,
    /// Last feedback on the friends system (error or success).
    freunde_meldung: String,
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
    /// --shot: where the image of the own UI is written to.
    shot: Option<std::path::PathBuf>,
    /// --meetshot: the same for the own meeting window.
    meet_shot: Option<std::path::PathBuf>,
    /// Frame counter and arming of the meetshot.
    meet_shot_n: u32,
    meet_shot_arm: u32,
    /// --meetshot mit Raum/Passwort: automatisch beitreten.
    meet_shot_join: bool,
    /// After how many frames the image is taken (joining takes time).
    shot_warte: u32,
    /// Frames drawn since the start in --shot mode.
    /// Chosen look (template, accent, size, rounding).
    look: theme::Appearance,
    /// Gewaehlter Bereich in den Einstellungen.
    stab: SettingsTab,
    /// Feedback: Art, Text, Kontakt.
    fb_bug: bool,
    fb_text: String,
    fb_contact: String,
    /// Device list: selected folder and open edit field.
    folder: String,
    /// Offenes "Geraet hinzufuegen"-Fenster.
    add_dev: Option<AddDev>,
    edit_dev: Option<DevEdit>,
    /// When the title bar was last colored.
    caption_tick: std::time::Instant,
    shot_n: u32,
    /// Feste Passwoerter dieses Rechners (Einstellungen -> Zugriff).
    pw_list: Vec<pwlist::Entry>,
    /// Input fields for a new entry.
    pw_new: String,
    pw_new_label: String,
    /// Which entry is currently visible in plain text.
    pw_show: Option<usize>,
    /// Session password on the start page: shown in clear text?
    pw_visible: bool,
    /// when the start page password was revealed (auto-hides)
    pw_visible_at: Option<std::time::Instant>,
    /// text typed into the "current password" field of the replace dialog
    pw_old_input: String,
    pw_old_wrong: bool,
    /// trash button armed for this list row (second click deletes)
    pw_del_arm: Option<usize>,
    /// The last viewer try knocked ("ask") instead of using a password.
    last_ask: bool,
    /// A change that would replace the permanent password waits for a yes.
    pw_overwrite: Option<PwOverwrite>,
    /// Should the service be set up right away during installation?
    install_service: bool,
    /// When the mailbox was last checked.
    link_check: std::time::Instant,
    /// Meet: inputs and what the server said last.
    meet_title: String,
    /// Checkbox "end-to-end encrypt" when creating. Off by default:
    /// with the checkbox only someone with a current version can join.
    meet_e2e: bool,
    meet_id: String,
    meet_pw: String,
    meet_last: Option<meet::Meeting>,
    meet_list: Arc<std::sync::Mutex<Vec<meet::Meeting>>>,
    /// How often the title bar of the meeting window has been recolored.
    meet_chrome: u32,
    meet_busy: Arc<std::sync::atomic::AtomicBool>,
    meet_err: Arc<std::sync::Mutex<String>>,
    meet_loaded: bool,
    /// Offer at joining that others may control this PC.
    meet_offer_control: bool,
    /// The own Meet window (Zoom flow in a separate viewport).
    meet_win: MeetWin,
    /// Natives Meeting (Stufe 1: Ton, Chat, Warteraum) - laeuft ohne Browser.
    nativ_meet: Option<meetui::NativMeet>,
    /// Bildkacheln des nativen Meetings: Teilnehmer -> (Stand, Textur).
    nativ_bilder: std::collections::HashMap<u64, (u64, egui::TextureHandle)>,
    /// Shared screens: participant -> (state, texture). Own storage,
    /// because a participant sends camera AND screen at the same time.
    nativ_schirme: std::collections::HashMap<u64, (u64, egui::TextureHandle)>,
    /// Whether sharing was already active in the last frame - the
    /// automatic minimizing and the small window depend on it.
    meet_teilt_war: bool,
    /// Eigenes Kamerabild im nativen Meeting: (Stand, Textur).
    nativ_eigen: Option<(u64, egui::TextureHandle)>,
    /// The own SHARED image - so that you see yourself what you share.
    nativ_eigen_schirm: Option<(u64, egui::TextureHandle)>,
    /// Camera and microphone BEFORE joining (self preview, level).
    vorprobe: meetui::Vorprobe,
    /// What only concerns the UI: sidebar, tabs, camera slot.
    meet_z: meetfenster::Fensterzustand,
    /// How many chat lines the user has already seen (counter on the button).
    meet_gelesen: usize,
    /// When the camera may start AFTER joining. The preview holds
    /// the device until joining; on Windows the camera needs
    /// a moment until it is free again - reopening immediately fails.
    meet_kam_start: Option<std::time::Instant>,
    /// No real window system (--uitest): then do not open extra windows.
    headless: bool,
    /// --meetdemo N: draw an example state of the meeting instead of the
    /// UI. Only there to be able to make real images -
    /// the build server has neither a camera nor a second participant.
    meet_demo: Option<usize>,
    /// Fullscreen like in the Windows remote connection: no window frame, the
    /// control bar floats over the image and can be moved.
    full: bool,
    /// Where the floating bar sits (0 = far left, 1 = far right).
    bar_x: f32,
    /// How wide it was last (for the position calculation).
    bar_w: f32,
    /// Pinned = always stays visible, otherwise it retracts.
    bar_pinned: bool,
    /// When the bar was last noticed (for retracting).
    bar_seen: std::time::Instant,
    /// When the last image arrived - for the "no image" hint.
    frame_at: std::time::Instant,
    /// Konto: angemeldete Sitzung, Eingabefelder, Zustand des Abgleichs.
    acc: Option<account::Session>,
    acc_user: String,
    acc_pass: String,
    acc_msg: String,
    acc_busy: Arc<std::sync::atomic::AtomicBool>,
    acc_out: Arc<std::sync::Mutex<Option<account::SyncOut>>>,
    acc_next: std::time::Instant,
    /// Setup tab: one-time links for new devices.
    dep_pw: String,
    dep_hint_name: String,
    /// How many devices may be set up with one link (1-25).
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
    /// Password prompt before connecting (partner ID).
    pw_ask: Option<String>,
    /// Safety question before uninstalling.
    uninstall_ask: bool,
    /// License: input field, message and whether the window is open (X-Remote only).
    #[cfg(feature = "license")]
    lic_key: String,
    #[cfg(feature = "license")]
    lic_msg: String,
    #[cfg(feature = "license")]
    lic_show: bool,
    /// Setup job from the browser download (code, desired name) -
    /// waits for the own ID, then everything happens by itself.
    auto_setup: Option<(String, String)>,
    /// When the update-is-running marker was last checked.
    upd_flag_at: std::time::Instant,
    /// Is the marker currently set? (for the loading window)
    upd_running: bool,
    /// Audio devices, read once (the query takes time).
    snd_in: Vec<String>,
    snd_out: Vec<String>,
    snd_default: (String, String),
}



// --------------------------------------------------------------- Aussehen --

/// Which page the window shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    /// Konferenzen (FreeViewer Meet)
    Meet,
    Start,
    Devices,
    /// Friends: confirmed by both sides, then one click to meeting or
    /// remote maintenance.
    Freunde,
    Settings,
}

/// Zustand des eigenen Meet-Fensters (separater Viewport, Zoom-Ablauf).
struct MeetWin {
    offen: bool,
    meeting: Option<meet::Meeting>,
    /// The browser with image and sound is already running.
    beigetreten: bool,
    /// Request "join" from the big button - converted one line further
    /// down into a real native session.
    nativ_start: bool,
    /// Device names as the system reports them (microphones, cameras).
    mics: Vec<String>,
    cams: Vec<String>,
    /// 0 = Browser-Standard, sonst Index+1 in mics/cams.
    mic_sel: usize,
    cam_sel: usize,
    stumm: bool,
    ohne_video: bool,
    /// Read once in the background (the query takes a moment).
    #[allow(clippy::type_complexity)]
    geraete: Arc<std::sync::Mutex<Option<(Vec<String>, Vec<String>, Vec<String>)>>>,
    geraete_geladen: bool,
    geraete_da: bool,
    /// Speakers - only for the settings IN the meeting; before
    /// joining the browser does not show them either.
    spks: Vec<String>,
    spk_sel: usize,
    /// Bildschirme dieses Rechners (Name, Breite, Hoehe) - einmal eingelesen.
    monitore: Vec<(String, u32, u32)>,
    /// Open program windows (id, title, program). Only re-read every
    /// one and a half seconds: EnumWindows asks the window manager and the process
    /// for EVERY window - doing that at every frame rate
    /// would be a waste.
    fenster: Vec<(isize, String, String)>,
    fenster_naechste: std::time::Instant,
    /// Teilnehmerliste vom Meet-Server (alle 5 s nachgeladen).
    tn: Arc<std::sync::Mutex<Vec<meet::Teilnehmer>>>,
    tn_busy: Arc<std::sync::atomic::AtomicBool>,
    tn_next: std::time::Instant,
    /// Short hint in the window ("Link copied") and when it came.
    toast: Option<(String, std::time::Instant)>,
    /// Display name in the meeting (field "Dein Name" on the join screen).
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
            monitore: Vec::new(),
            fenster: Vec::new(),
            fenster_naechste: std::time::Instant::now(),
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
    /// Desired resolution on the host (width, height).
    res: Option<(u32, u32)>,
    special: Option<u8>,
    pick: bool,
    open_dir: bool,
    toggle_full: bool,
    toggle_pin: bool,
    /// How far the handle of the floating bar was dragged.
    drag: f32,
}

fn game_mode(shared: &Arc<Shared>) -> bool {
    shared.game_mode()
}

/// What is in the "Add device" window while it is open.
#[derive(Clone, Default)]
struct AddDev {
    id: String,
    name: String,
    folder: String,
    password: String,
    /// Why adding is currently not possible.
    err: String,
}

/// What is in the edit field of a device while it is open.
#[derive(Clone, Default)]
struct DevEdit {
    id: String,
    name: String,
    password: String,
    note: String,
    folder: String,
}

/// A password change that needs a confirmation because it would replace the
/// permanent password of this machine.
#[derive(Clone, PartialEq)]
enum PwOverwrite {
    /// "Create a new password" while a permanent password is stored.
    Regenerate,
    /// A setup link wants to set this password.
    Setup(String),
}

/// Zustand des Einrichtungs-Dialogs (freeviewer://setup/<code>).
#[derive(Default)]
struct SetupDlg {
    code: String,
    name: String,
    /// From the browser download: set up immediately, without asking again.
    auto: bool,
    busy: Arc<std::sync::atomic::AtomicBool>,
    out: Arc<std::sync::Mutex<Option<Result<account::SetupClaimReply, String>>>>,
    err: String,
    owner: String,
    done: bool,
}

/// Areas within the settings.
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


/// Width beyond which the content is not stretched further.
const CONTENT_MAX: f32 = 1180.0;
/// How long the start page password stays readable after clicking the eye.
const PW_REVEAL_SECS: Duration = Duration::from_secs(10);
