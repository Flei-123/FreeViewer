//! Split out of main.rs (r35): mechanical move, behaviour unchanged.
use crate::lockutil::LockExt;
use super::*;

pub(crate) fn cli_connect(pos: usize, argv: Vec<String>, shared: Arc<Shared>) -> eframe::Result<()> {
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
            let mons = shared.monitors.lock_ok().clone();
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
                match shared.xfer.lock_ok().as_mut() {
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
                let (rw, rh) = *shared.remote_size.lock_ok();
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
            let st = *shared.stats.lock_ok();
            let cur = *shared.remote_cursor.lock_ok();
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
                "Transport: {} ({} frames direct via UDP, rest via the relay)",
                if shared.direct.load(Ordering::Relaxed) {
                    "direkt (P2P)"
                } else {
                    "Relay"
                },
                udp
            );
            std::process::exit(0);
        }
        let status = shared.viewer_status.lock_ok().clone();
        if shared.viewer_failed.load(Ordering::Relaxed) {
            println!("FAIL: {}", status);
            std::process::exit(1);
        }
        if start.elapsed() > Duration::from_secs(30) {
            println!("FAIL: {}", shared.viewer_status.lock_ok());
            std::process::exit(1);
        }
    }
}

// scripted "please confirm" test:  freeviewer --ask <id> [frames]
pub(crate) fn cli_ask(pos: usize, argv: Vec<String>, shared: Arc<Shared>) -> eframe::Result<()> {
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
                shared.session_code.lock_ok()
            );
            std::process::exit(0);
        }
        let status = shared.viewer_status.lock_ok().clone();
        if shared.viewer_failed.load(Ordering::Relaxed) {
            println!("FAIL: {}", status);
            std::process::exit(1);
        }
        if start.elapsed() > Duration::from_secs(45) {
            println!("FAIL: Zeitueberschreitung - {}", status);
            std::process::exit(1);
        }
    }
}

// Image of the own UI:
//   freeviewer --shot C:\bild.jpg [--view start|devices|settings] [--demo]
// The app draws itself and saves the result - this also works
// when the screen is locked and a window photo would be just white.
pub(crate) fn cli_shot(i: usize, shared: Arc<Shared>) -> eframe::Result<()> {
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
                *shot_shared.my_id.lock_ok() = "497628420".to_string();
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
                *shot_shared.knock.lock_ok() = Some(shared::Knock {
                    from: "Laptop von Justin".to_string(),
                    code: "1234".to_string(),
                    at: std::time::Instant::now(),
                    unverified: true,
                    friend_like: true,
                });
            }
            app.view = match want.as_deref() {
                Some("devices") => View::Devices,
                Some("meet") => View::Meet,
                Some("freunde") | Some("friends") => View::Friends,
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
                // Open the own meeting window and take the image OF THAT.
                // With "--meetshot bild.jpg RAUM PASS" it really
                // joins - then the image shows the real meeting and
                // not just the join screen.
                // Usage:  --shot bild.jpg --meetshot [RAUM PASS]
                let a2 = std::env::args().nth(mi + 1).unwrap_or_default();
                let a3 = std::env::args().nth(mi + 2).unwrap_or_default();
                let echt = a2.contains('-') && !a3.is_empty();
                app.meet_win_open(meet::Meeting {
                    id: if echt { a2.clone() } else { "482-913-770".to_string() },
                    titel: "Fenster-Test".to_string(),
                    passphrase: if echt { a3.clone() } else { "test1234".to_string() },
                    termin_text: String::new(),
                    e2e: String::new(),
                });
                app.meet_shot_join = echt;
                // With a real join the connection has to be up first.
                app.shot_warte = if echt { 900 } else { 60 };
                // shot MUST be set as well - the image maker depends on it.
                app.shot = Some(std::path::PathBuf::from(path.clone()));
                app.meet_shot = Some(std::path::PathBuf::from(path));
            } else {
                app.shot = Some(std::path::PathBuf::from(path));
            }
            // --shotwait N: take the image only after N frames - this lets
            // a state be photographed that needs a few seconds
            // (e.g. "relay not reachable").
            if let Some(n) = std::env::args()
                .skip_while(|a| a != "--shotwait")
                .nth(1)
                .and_then(|x| x.parse::<u32>().ok())
            {
                app.shot_warte = n;
            }
            Ok(Box::new(app))
        }),
    );
}
