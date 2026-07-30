$ErrorActionPreference = 'Stop'
$root = 'C:\Users\Admin\Projects\FreeViewer'
$enc = New-Object System.Text.UTF8Encoding($false)
$script:fail = 0
function RepOne([string]$text, [string]$old, [string]$new, [string]$what) {
  $pat = [regex]::Escape($old) -replace '\\r\\n', '\r?\n' -replace '\\n', '\r?\n'
  $ms = [regex]::Matches($text, $pat)
  if ($ms.Count -ne 1) { Write-Host ("FEHLER {0}: {1} Treffer" -f $what, $ms.Count); $script:fail++; return $text }
  Write-Host ("ok " + $what)
  return $text.Remove($ms[0].Index, $ms[0].Length).Insert($ms[0].Index, $new)
}

# ============================================ audio.rs: Standard beim Verbinden
$p = Join-Path $root 'src\audio.rs'
$a = [System.IO.File]::ReadAllText($p, $enc)
$a = RepOne $a '/// Sample rate on the wire.' @'
/// Wie eine neue Sitzung startet: beides aus, bis der Nutzer es anders will.
pub static DEFAULT_MIC: AtomicBool = AtomicBool::new(false);
pub static DEFAULT_SND: AtomicBool = AtomicBool::new(false);

/// Aus den Einstellungen gesetzt.
pub fn set_defaults(mic: bool, snd: bool) {
    DEFAULT_MIC.store(mic, Ordering::Relaxed);
    DEFAULT_SND.store(snd, Ordering::Relaxed);
}

/// Sample rate on the wire.
'@ 'audio: Standardwerte'
$a = RepOne $a '            mic: AtomicBool::new(std::env::var("FV_AUDIO_MIC").as_deref() == Ok("1")),' '            mic: AtomicBool::new(false),' 'audio: mic startet aus'
$a = RepOne $a '            speaker: AtomicBool::new(true),' '            speaker: AtomicBool::new(false),' 'audio: Ton startet aus'
$a = RepOne $a '    pub fn start(state: Arc<VoiceState>, send: Arc<dyn Fn(Msg) + Send + Sync>) -> Voice {' @'
    pub fn start(state: Arc<VoiceState>, send: Arc<dyn Fn(Msg) + Send + Sync>) -> Voice {
        // Jede Sitzung beginnt mit dem, was in den Einstellungen steht -
        // standardmaessig ist beides stumm.
        state.mic.store(
            DEFAULT_MIC.load(Ordering::Relaxed)
                || std::env::var("FV_AUDIO_MIC").as_deref() == Ok("1"),
            Ordering::Relaxed,
        );
        state
            .speaker
            .store(DEFAULT_SND.load(Ordering::Relaxed), Ordering::Relaxed);
'@ 'audio: Sitzungsstart setzt Standard'
# aus (schon gepatcht): WriteAllText $a

# ============================================ theme.rs: zwei Schalter merken
$p = Join-Path $root 'src\theme.rs'
$t = [System.IO.File]::ReadAllText($p, $enc)
$t = RepOne $t '    /// Sprachkuerzel: "de" oder "en"
    pub lang: String,' @'
    /// Sprachkuerzel: "de" oder "en"
    pub lang: String,
    /// Mikrofon beim Verbinden gleich an?
    pub mic_on: bool,
    /// Ton der anderen Seite beim Verbinden gleich an?
    pub snd_on: bool,
'@ 'theme: Tonstandards'
$t = RepOne $t '            lang: "de".to_string(),' @'
            lang: "de".to_string(),
            mic_on: false,
            snd_on: false,
'@ 'theme: Tonstandards Default'
$t = RepOne $t '            if let Some(l) = v.get("lang").and_then(|x| x.as_str()) {' @'
            if let Some(b) = v.get("mic_on").and_then(|x| x.as_bool()) {
                a.mic_on = b;
            }
            if let Some(b) = v.get("snd_on").and_then(|x| x.as_bool()) {
                a.snd_on = b;
            }
            if let Some(l) = v.get("lang").and_then(|x| x.as_str()) {
'@ 'theme: Tonstandards laden'
$t = RepOne $t '    v.insert("lang".into(), serde_json::Value::from(a.lang.clone()));' @'
    v.insert("lang".into(), serde_json::Value::from(a.lang.clone()));
    v.insert("mic_on".into(), serde_json::Value::from(a.mic_on));
    v.insert("snd_on".into(), serde_json::Value::from(a.snd_on));
'@ 'theme: Tonstandards speichern'
$t = RepOne $t 'pub fn apply(ctx: &egui::Context, a: &Appearance) {' @'
pub fn apply(ctx: &egui::Context, a: &Appearance) {
    crate::audio::set_defaults(a.mic_on, a.snd_on);
'@ 'theme: Standards an audio geben'
# aus (schon gepatcht): WriteAllText $t

# ============================================ main.rs
$p = Join-Path $root 'src\main.rs'
$m = [System.IO.File]::ReadAllText($p, $enc)

# Sprach-Karte vom Dashboard entfernen
$blk = [regex]::Match($m, '(?s)            // Sprache in der Sitzung - klein, direkt unter dem eigenen Kasten\r?\n.*?\r?\n            \}\);\r?\n')
if ($blk.Success) {
  $m = $m.Remove($blk.Index, $blk.Length)
  Write-Host 'ok main: Sprach-Karte vom Dashboard entfernt'
} else { Write-Host 'FEHLER main: Sprach-Karte'; $script:fail++ }

# Tonseite: Standardschalter statt Dauerschalter
$m = RepOne $m '        card(ui, |ui| {
            ui.horizontal(|ui| {
                self.voice_buttons(ui, 18.0);
                ui.add_space(8.0);
                let v = self.shared.voice.clone();
                level_bar(
                    ui,
                    v.level_out.load(Ordering::Relaxed),
                    v.level_in.load(Ordering::Relaxed),
                );
            });' @'
        let mut look = self.look.clone();
        card(ui, |ui| {
            label_small(ui, i18n::t("set.audio_default"));
            if ui
                .checkbox(&mut look.mic_on, i18n::t("set.mic_default"))
                .on_hover_text(i18n::t("set.mic_default_tip"))
                .changed()
            {}
            if ui
                .checkbox(&mut look.snd_on, i18n::t("set.snd_default"))
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
'@ 'main: Tonseite mit Standardwerten'
$m = RepOne $m '            let (mic, spk) = audio::device_names();
            info_row(ui, i18n::t("set.mic_dev"), &mic);
            info_row(ui, i18n::t("set.spk_dev"), &spk);
        });
    }' @'
            let (mic, spk) = audio::device_names();
            info_row(ui, i18n::t("set.mic_dev"), &mic);
            info_row(ui, i18n::t("set.spk_dev"), &spk);
        });
        if look != self.look {
            self.apply_look(ui.ctx(), look);
        }
    }
'@ 'main: Tonstandards speichern'
[System.IO.File]::WriteAllText($p, $m, $enc)

# ============================================ i18n
$p = Join-Path $root 'src\i18n.rs'
$s = [System.IO.File]::ReadAllText($p, $enc)
$s = RepOne $s '    ("set.clip", "Zwischenablage teilen", "Share clipboard"),' @'
    ("set.audio_default", "Standard beim Verbinden", "Default when connecting"),
    ("set.audio_now", "Jetzt in der Sitzung", "Right now in the session"),
    ("set.mic_default", "Mikrofon gleich an", "Microphone on right away"),
    (
        "set.mic_default_tip",
        "Aus: jede Sitzung startet stumm und man schaltet das Mikrofon in der Sitzungsleiste dazu.",
        "Off: every session starts muted; switch the microphone on in the session bar.",
    ),
    ("set.snd_default", "Ton gleich an", "Sound on right away"),
    ("set.clip", "Zwischenablage teilen", "Share clipboard"),
'@ 'i18n: Tonstandards'
# aus (schon gepatcht): WriteAllText $s

if ($script:fail -gt 0) { Write-Host ("ABBRUCH: " + $script:fail); exit 1 }
Write-Host 'r5 fertig'
