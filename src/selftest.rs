//! `freeviewer --inputtest <id> <password>`
//!
//! Connects as a viewer and drives a scripted input sequence over the real
//! encrypted session, so that the *host* side can be measured from outside
//! (cursor position, key state, clipboard). Every step is printed with a
//! timestamp so the two logs can be lined up afterwards.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::proto::{self, Msg};
use crate::shared::Shared;

fn nap(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

pub fn input_selftest(shared: Arc<Shared>, argv: &[String], pos: usize) -> ! {
    let id = argv.get(pos + 1).cloned().unwrap_or_default();
    let pw = argv.get(pos + 2).cloned().unwrap_or_default();

    let sh = shared.clone();
    let idc = id.clone();
    crate::rt().spawn(async move { crate::viewer::run_viewer(sh, idc, pw).await });

    let start = Instant::now();
    while !shared.connected.load(Ordering::Relaxed) {
        if start.elapsed() > Duration::from_secs(20) {
            println!("FAIL: {}", shared.viewer_status.lock().unwrap());
            std::process::exit(1);
        }
        nap(100);
    }
    println!("verbunden nach {} ms", start.elapsed().as_millis());
    nap(900);

    macro_rules! step {
        ($($a:tt)*) => {{
            println!("[{:>6} ms] {}", start.elapsed().as_millis(), format!($($a)*));
        }};
    }

    // ---- absolute pointer (Fernwartung) -------------------------------------
    step!("mouse_abs 5000/5000 = Bildmitte");
    shared.send_input(Msg::MouseMove { x: 5000, y: 5000 });
    nap(1600);

    step!("mouse_abs 2500/2500 = oben links");
    shared.send_input(Msg::MouseMove { x: 2500, y: 2500 });
    nap(1600);

    // ---- game mode: relative pointer ---------------------------------------
    step!("SetMode -> Spiel");
    shared.send_input(Msg::SetMode {
        mode: proto::MODE_GAME,
    });
    shared.mode.store(proto::MODE_GAME, Ordering::Relaxed);
    nap(900);

    step!("mouse_delta 30x (+10/0) = +300 px relativ");
    for _ in 0..30 {
        shared.send_input(Msg::MouseDelta { dx: 10, dy: 0 });
        nap(10);
    }
    nap(1400);

    step!("mouse_delta 30x (0/+8) = +240 px relativ");
    for _ in 0..30 {
        shared.send_input(Msg::MouseDelta { dx: 0, dy: 8 });
        nap(10);
    }
    nap(1400);

    // ---- keyboard ----------------------------------------------------------
    for n in 1..=2 {
        step!("NumLock tippen #{}", n);
        shared.send_input(Msg::KeyVk {
            vk: 0x90,
            ext: false,
            down: true,
        });
        shared.send_input(Msg::KeyVk {
            vk: 0x90,
            ext: false,
            down: false,
        });
        nap(1500);
    }

    step!("Strg DOWN (bleibt absichtlich haengen)");
    shared.send_input(Msg::KeyVk {
        vk: 0x11,
        ext: false,
        down: true,
    });
    nap(1600);

    step!("Special RELEASE (haengende Tasten loesen)");
    shared.send_input(Msg::Special {
        code: proto::SPECIAL_RELEASE,
    });
    nap(1600);

    // ---- clipboard ---------------------------------------------------------
    let text = format!("FV-CLIP-{}", start.elapsed().as_millis());
    step!("Zwischenablage -> {}", text);
    shared.send_input(Msg::Clipboard { text });
    nap(2500);

    // ---- clipboard the other way round: host -> viewer ----------------------
    // Note: on ONE machine both processes share the same Windows clipboard,
    // so the viewer usually picks a change up itself before the host can send
    // it. What matters is that host -> viewer messages arrive at all.
    step!("warte auf Zwischenablage-Nachrichten vom Host (max 10 s)");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && shared.clip_from_host.load(Ordering::Relaxed) == 0 {
        nap(200);
    }
    let n = shared.clip_from_host.load(Ordering::Relaxed);
    if n > 0 {
        step!("{} Zwischenablage-Update(s) vom Host empfangen", n);
    } else {
        step!("FEHLER: keine Zwischenablage vom Host bekommen");
    }

    step!("SetMode -> Fernwartung");
    shared.send_input(Msg::SetMode {
        mode: proto::MODE_ADMIN,
    });
    nap(600);

    let st = *shared.stats.lock().unwrap();
    let (rw, rh) = *shared.remote_size.lock().unwrap();
    println!(
        "OK: inputtest durch, Remote {}x{}, {:.0} fps, {:.0} kbit/s, {:.0} ms rtt",
        rw, rh, st.fps, st.kbps, st.latency_ms
    );
    std::process::exit(0);
}
