$ErrorActionPreference = 'Stop'
$root = 'C:\Users\Admin\Projects\FreeViewer'
$enc = New-Object System.Text.UTF8Encoding($false)
$fail = 0

function Patch($rel, $anchor, $new, $mode) {
  $p = Join-Path $root $rel
  $s = [System.IO.File]::ReadAllText($p, $enc)
  $n = ([regex]::Matches($s, [regex]::Escape($anchor))).Count
  if ($n -ne 1) { Write-Output ("FEHLER {0}: Anker {1}x -> {2}" -f $rel, $n, $anchor.Trim()); $script:fail++; return }
  if ($mode -eq 'before') { $s = $s.Replace($anchor, ($new + $anchor)) }
  elseif ($mode -eq 'after') { $s = $s.Replace($anchor, ($anchor + $new)) }
  else { $s = $s.Replace($anchor, $new) }
  [System.IO.File]::WriteAllText($p, $s, $enc)
  Write-Output ("ok {0}: {1}" -f $rel, $anchor.Trim().Substring(0, [Math]::Min(48, $anchor.Trim().Length)))
}

# ---------------------------------------------------------------- Cargo.toml
Patch 'Cargo.toml' 'ureq = "3"' "`ncpal = `"0.15`"" 'after'

# ------------------------------------------------------------------- proto.rs
Patch 'src/proto.rs' '    /// The video path lost data, please send a full frame.' @'
    /// One 20 ms packet of speech: mono, 24 kHz, IMA-ADPCM. Travels in both
    /// directions inside the same encrypted channel as everything else.
    Audio { seq: u32, data: Vec<u8> },
'@ 'before'

Patch 'src/proto.rs' 'const T_NEEDKEY: u8 = 0x62;' "`nconst T_AUDIO: u8 = 0x70;" 'after'

Patch 'src/proto.rs' 'const MAX_ADDRS: usize = 8;' @'

/// One speech packet is 243 bytes; anything much larger is not ours.
pub const MAX_AUDIO: usize = 4096;
'@ 'after'

Patch 'src/proto.rs' '        Msg::NeedKeyframe => {' @'
        Msg::Audio { seq, data } => {
            let n = data.len().min(MAX_AUDIO);
            v.reserve(n + 12);
            v.push(T_AUDIO);
            pu32(&mut v, *seq);
            pu32(&mut v, n as u32);
            v.extend_from_slice(&data[..n]);
        }
'@ 'before'

Patch 'src/proto.rs' '        T_PING => Some(Msg::Ping { ts: r.u64()? }),' @'
        T_AUDIO => {
            let seq = r.u32()?;
            let n = r.u32()? as usize;
            if n > MAX_AUDIO {
                return None;
            }
            let data = r.take(n)?.to_vec();
            Some(Msg::Audio { seq, data })
        }
'@ 'before'

# ------------------------------------------------------------------ shared.rs
Patch 'src/shared.rs' '    pub stats: Mutex<Stats>,' @'
    /// Microphone/speaker of the running session (voice link).
    pub voice: std::sync::Arc<crate::audio::VoiceState>,
'@ 'before'

Patch 'src/shared.rs' '            stats: Mutex::new(Stats::default()),' @'
            voice: std::sync::Arc::new(crate::audio::VoiceState::default()),
'@ 'before'

# ---------------------------------------------------------------- hostside.rs
Patch 'src/hostside.rs' '    p2p: Option<Arc<crate::p2p::P2p>>,' @'

    /// Speech both ways while the session runs.
    voice: Option<crate::audio::Voice>,
'@ 'after'

Patch 'src/hostside.rs' '            p2p: None,' @'

            voice: None,
'@ 'after'

Patch 'src/hostside.rs' '                self.p2p = p2p;' @'

                // voice link: speech in both directions, same encrypted channel
                {
                    let vtx = out_tx.clone();
                    let vsend: std::sync::Arc<dyn Fn(Msg) + Send + Sync> =
                        std::sync::Arc::new(move |m: Msg| {
                            let _ = vtx.send(encode(&m));
                        });
                    self.voice = Some(crate::audio::Voice::start(shared.voice.clone(), vsend));
                }
'@ 'after'

Patch 'src/hostside.rs' '                        other => {' @'
                        Msg::Audio { seq, data } => {
                            if let Some(v) = self.voice.as_ref() {
                                v.feed(seq, &data);
                            }
                        }
'@ 'before'

# ------------------------------------------------------------------ viewer.rs
Patch 'src/viewer.rs' '    let mut ping_task: Option<tokio::task::JoinHandle<()>> = None;' @'

    // voice link of this session (dropped when the session ends)
    let mut voice: Option<crate::audio::Voice> = None;
'@ 'after'

Patch 'src/viewer.rs' '                        // clipboard sync (own thread, clipboard handles are not Send)' @'
                        // voice link: microphone out, speaker in
                        {
                            let sh = shared.clone();
                            let vsend: Arc<dyn Fn(Msg) + Send + Sync> =
                                Arc::new(move |m: Msg| sh.send_input(m));
                            voice = Some(crate::audio::Voice::start(
                                shared.voice.clone(),
                                vsend,
                            ));
                        }

'@ 'before'

Patch 'src/viewer.rs' '                            Some(Msg::Cursor { x, y, visible }) => {' @'
                            Some(Msg::Audio { seq, data }) => {
                                if let Some(v) = voice.as_ref() {
                                    v.feed(seq, &data);
                                }
                            }
'@ 'before'

# -------------------------------------------------------------------- main.rs
Patch 'src/main.rs' 'mod autostart;' "mod audio;`nmod autostart;" 'replace'

Patch 'src/main.rs' '    // Draws every page once without a window:  freeviewer --uitest' @'
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

'@ 'before'

# Bedienleiste der Sitzung: Mikro/Ton umschalten
Patch 'src/main.rs' '                ui.separator();
                let mut want_pick = false;' @'
                ui.separator();
                {
                    let v = self.shared.voice.clone();
                    let mic = v.mic.load(Ordering::Relaxed);
                    if ui
                        .selectable_label(mic, if mic { "Mikro an" } else { "Mikro aus" })
                        .on_hover_text("Sprache zum anderen Rechner senden")
                        .clicked()
                    {
                        v.mic.store(!mic, Ordering::Relaxed);
                    }
                    let spk = v.speaker.load(Ordering::Relaxed);
                    if ui
                        .selectable_label(spk, if spk { "Ton an" } else { "Ton aus" })
                        .on_hover_text("Sprache der anderen Seite abspielen")
                        .clicked()
                    {
                        v.speaker.store(!spk, Ordering::Relaxed);
                    }
                    if mic || v.got.load(Ordering::Relaxed) > 0 {
                        ui.label(
                            egui::RichText::new(format!(
                                "Pegel {} / {}",
                                v.level_out.load(Ordering::Relaxed),
                                v.level_in.load(Ordering::Relaxed)
                            ))
                            .weak()
                            .size(11.0),
                        );
                    }
                }

'@ 'replace'

# Startseite (Host): Sprache erlauben / hoeren
Patch 'src/main.rs' '                ui.label(egui::RichText::new(host_peer).size(12.5).color(MUTED));' @'

                ui.add_space(4.0);
                label_small(ui, "Sprache in der Sitzung");
                ui.horizontal(|ui| {
                    let v = self.shared.voice.clone();
                    let mut mic = v.mic.load(Ordering::Relaxed);
                    if ui
                        .checkbox(&mut mic, "Mikrofon senden")
                        .on_hover_text("Nur waehrend einer laufenden Sitzung.")
                        .changed()
                    {
                        v.mic.store(mic, Ordering::Relaxed);
                    }
                    let mut spk = v.speaker.load(Ordering::Relaxed);
                    if ui.checkbox(&mut spk, "Ton hoeren").changed() {
                        v.speaker.store(spk, Ordering::Relaxed);
                    }
                });
                let prob = self.shared.voice.problem.lock().unwrap().clone();
                if !prob.is_empty() {
                    ui.label(egui::RichText::new(prob).size(11.5).color(MUTED));
                }
'@ 'after'

if ($fail -gt 0) { Write-Output "ABBRUCH: $fail Anker nicht eindeutig" } else { Write-Output 'alle Patches gesetzt' }
