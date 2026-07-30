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

# ============================================================ vinput.rs
$p = Join-Path $root 'src\vinput.rs'
$v = [System.IO.File]::ReadAllText($p, $enc)
$v = RepOne $v 'static CX: AtomicI32 = AtomicI32::new(0);' @'
/// Relative Maus (Spielmodus). Die Tastatur wird auch in der Fernwartung
/// komplett uebernommen, die Maus aber nur im Spielmodus umgestellt.
static REL: AtomicBool = AtomicBool::new(false);
static CX: AtomicI32 = AtomicI32::new(0);
'@ 'vinput: REL'
$v = RepOne $v 'pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}' @'
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Maus relativ messen (nur Spielmodus).
pub fn set_relative(on: bool) {
    REL.store(on, Ordering::Relaxed);
}

pub fn is_relative() -> bool {
    REL.load(Ordering::Relaxed)
}
'@ 'vinput: set_relative'
$v = RepOne $v '                let active = ACTIVE.load(Ordering::Relaxed);' '                let active = ACTIVE.load(Ordering::Relaxed) && REL.load(Ordering::Relaxed);' 'vinput: Zeiger nur im Spielmodus'
# aus (schon gepatcht): WriteAllText $v

# ============================================================ main.rs: Grab
$p = Join-Path $root 'src\main.rs'
$m = [System.IO.File]::ReadAllText($p, $enc)
$m = RepOne $m '    fn update_grab(&mut self, ctx: &egui::Context, rect: egui::Rect, game: bool, clicked: bool) {
        if !game {
            return;
        }
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));' @'
    fn update_grab(&mut self, ctx: &egui::Context, rect: egui::Rect, game: bool, clicked: bool) {
        // Die Tastatur wird in BEIDEN Betriebsarten komplett uebernommen -
        // Windows-Taste, Alt+Tab, Alt+F4 gehen dann an den anderen Rechner
        // und nicht mehr an das eigene Windows. Rechte Strg holt sie zurueck.
        vinput::set_relative(game);
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
'@ 'main: Grab in beiden Betriebsarten'
$m = RepOne $m '        if vinput::is_active() {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }' '        if vinput::is_active() && game {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }' 'main: Zeiger nur im Spielmodus verstecken'

# Module
$m = RepOne $m 'mod h264;' "mod feedback;`r`nmod h264;" 'main: mod feedback'
# DEFAULT_RELAY muss aus feedback.rs erreichbar sein
$m = RepOne $m 'const DEFAULT_RELAY: &str' 'pub const DEFAULT_RELAY: &str' 'main: DEFAULT_RELAY oeffentlich'

# App-Felder fuer Feedback und Geraeteliste
$m = RepOne $m '    stab: SettingsTab,' @'
    stab: SettingsTab,
    /// Feedback: Art, Text, Kontakt.
    fb_bug: bool,
    fb_text: String,
    fb_contact: String,
    /// Geraeteliste: gewaehlter Ordner und offenes Bearbeiten-Feld.
    folder: String,
    edit_dev: Option<DevEdit>,
'@ 'main: Felder Feedback/Ordner'
$m = RepOne $m '            stab: SettingsTab::General,' @'
            stab: SettingsTab::General,
            fb_bug: true,
            fb_text: String::new(),
            fb_contact: String::new(),
            folder: String::new(),
            edit_dev: None,
'@ 'main: Felder init'

# DevEdit-Struktur neben SettingsTab
$m = RepOne $m '/// Bereiche innerhalb der Einstellungen.' @'
/// Was im Bearbeiten-Feld eines Geraetes steht, solange es offen ist.
#[derive(Clone, Default)]
struct DevEdit {
    id: String,
    name: String,
    password: String,
    note: String,
    folder: String,
}

/// Bereiche innerhalb der Einstellungen.
'@ 'main: DevEdit'

# Info-Seite: Update + Feedback
$m = RepOne $m '    fn set_about(&mut self, ui: &mut egui::Ui) {' @'
    /// Update-Karte: Version, Suchen, Automatik.
    fn set_update(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                info_row(ui, i18n::t("set.version"), update::VERSION);
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if icons::text_button(ui, "refresh", i18n::t("upd.check"), false).clicked() {
                    let sh = self.shared.clone();
                    sh.set_update_status(i18n::t("upd.checking"));
                    std::thread::spawn(move || match update::check() {
                        Ok(rel) => {
                            if update::newer(&rel.version, update::VERSION) {
                                sh.set_update_status(format!("{} {}", i18n::t("upd.found"), rel.version));
                                *sh.update.lock().unwrap() = Some(rel);
                            } else {
                                sh.set_update_status(format!("{} (v{})", i18n::t("upd.current"), update::VERSION));
                            }
                        }
                        Err(e) => sh.set_update_status(format!("{}: {}", i18n::t("upd.failed"), e)),
                    });
                }
                self.update_ui(ui);
            });
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(i18n::t("upd.note"))
                    .size(11.0)
                    .color(p.muted),
            );
        });
    }

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
'@ 'main: Update- und Feedback-Karte'

# Reiter Update/Feedback in die Einstellungen
$m = RepOne $m '                    (SettingsTab::About, "eye", i18n::t("set.about")),' @'
                    (SettingsTab::Update, "refresh", i18n::t("set.update")),
                    (SettingsTab::Feedback, "chat", i18n::t("set.feedback")),
                    (SettingsTab::About, "eye", i18n::t("set.about")),
'@ 'main: Reiter Update/Feedback'
$m = RepOne $m '                    SettingsTab::About => self.set_about(ui),' @'
                    SettingsTab::Update => self.set_update(ui),
                    SettingsTab::Feedback => self.set_feedback(ui),
                    SettingsTab::About => self.set_about(ui),
'@ 'main: Reiter-Inhalte'
$m = RepOne $m 'enum SettingsTab {
    General,
    Access,
    Audio,
    Look,
    About,
}' 'enum SettingsTab {
    General,
    Access,
    Audio,
    Look,
    Update,
    Feedback,
    About,
}' 'main: SettingsTab erweitert'
$m = RepOne $m '                        "look" => SettingsTab::Look,' @'
                        "look" => SettingsTab::Look,
                        "update" => SettingsTab::Update,
                        "feedback" => SettingsTab::Feedback,
'@ 'main: shot --tab update/feedback'

[System.IO.File]::WriteAllText($p, $m, $enc)

# ============================================================ partners.rs
$p = Join-Path $root 'src\partners.rs'
$s = [System.IO.File]::ReadAllText($p, $enc)
$s = RepOne $s '    /// Encrypted password (hex), only present if the user asked for it.
    #[serde(default)]
    pub secret: Option<String>,' @'
    /// Encrypted password (hex), only present if the user asked for it.
    #[serde(default)]
    pub secret: Option<String>,
    /// Ordner/Gruppe, in der das Geraet steht. Leer = "Alle".
    #[serde(default)]
    pub group: String,
    /// Freie Notiz zum Geraet.
    #[serde(default)]
    pub note: String,
'@ 'partners: group + note'
$s = RepOne $s '    /// Decrypted password, if one was stored.' @'
    /// Ordner setzen (leer = kein Ordner).
    pub fn set_group(&mut self, id: &str, group: &str) {
        self.entry(id).group = group.trim().to_string();
        self.save();
    }

    /// Notiz setzen.
    pub fn set_note(&mut self, id: &str, note: &str) {
        self.entry(id).note = note.trim().to_string();
        self.save();
    }

    /// Passwort hinterlegen (None loescht es).
    pub fn set_password(&mut self, id: &str, password: Option<&str>) {
        let sealed = match password {
            Some(pw) if !pw.is_empty() => protect(pw),
            _ => None,
        };
        self.entry(id).secret = sealed;
        self.save();
    }

    /// Alle vorhandenen Ordner, alphabetisch.
    pub fn groups(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .entries
            .iter()
            .map(|p| p.group.trim().to_string())
            .filter(|g| !g.is_empty())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Decrypted password, if one was stored.
'@ 'partners: set_group/set_note/set_password/groups'
# aus (schon gepatcht): WriteAllText $s

# ============================================================ i18n.rs
$p = Join-Path $root 'src\i18n.rs'
$s = [System.IO.File]::ReadAllText($p, $enc)
$s = RepOne $s '    // Sitzung' @'
    // Update und Rueckmeldung
    ("set.update", "Update", "Update"),
    ("set.feedback", "Rückmeldung", "Feedback"),
    ("upd.check", "Nach Updates suchen", "Check for updates"),
    ("upd.checking", "Suche …", "Checking …"),
    ("upd.found", "Neue Version:", "New version:"),
    ("upd.current", "Aktuell", "Up to date"),
    ("upd.failed", "Suche fehlgeschlagen", "Check failed"),
    (
        "upd.note",
        "FreeViewer sieht beim Start und alle 30 Minuten nach. Nie mitten in einer Sitzung.",
        "FreeViewer looks at start and every 30 minutes. Never during a session.",
    ),
    ("fb.bug", "Fehler melden", "Report a bug"),
    ("fb.idea", "Idee vorschlagen", "Suggest an idea"),
    (
        "fb.intro",
        "Etwas kaputt oder eine Idee? Geht direkt an den Entwickler.",
        "Something broken or an idea? Goes straight to the developer.",
    ),
    ("fb.hint", "Beschreibe den Fehler oder die Idee …", "Describe the bug or your idea …"),
    ("fb.contact", "Kontakt (freiwillig, z. B. E-Mail)", "Contact (optional, e.g. email)"),
    ("fb.send", "Senden", "Send"),
    // Geraete bearbeiten
    ("dev.edit", "Bearbeiten", "Edit"),
    ("dev.details", "Gerät", "Device"),
    ("dev.folder", "Ordner", "Folder"),
    ("dev.folder_all", "Alle Geräte", "All devices"),
    ("dev.folder_new", "Neuer Ordner", "New folder"),
    ("dev.note", "Notiz", "Note"),
    ("dev.pw_stored", "Passwort hinterlegt", "Password saved"),
    ("dev.pw_none", "kein Passwort", "no password"),
    ("dev.pw_set", "Passwort hinterlegen", "Save a password"),
    ("dev.pw_clear", "Passwort löschen", "Delete password"),
    ("dev.ask_connect", "Bestätigung anfordern", "Ask for confirmation"),
    ("dev.stats", "Verbindungen", "Connections"),
    ("dev.last", "Zuletzt", "Last"),
    ("dev.nosel", "Links ein Gerät anklicken.", "Pick a device on the left."),
    // Sitzung
'@ 'i18n: neue Texte'
# aus (schon gepatcht): WriteAllText $s

if ($script:fail -gt 0) { Write-Host ("ABBRUCH: " + $script:fail + " Fehler") ; exit 1 }
Write-Host 'alle Patches gesetzt'
