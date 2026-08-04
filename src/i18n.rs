//! Sprachen. Jeder sichtbare Text steht genau einmal hier - Deutsch und
//! Englisch. Weitere Sprachen sind eine weitere Spalte, sonst nichts.
//!
//! Aufruf: `t("start.connect")`, mit Platzhalter `tf("dev.count", "7")`.
//!
//! WICHTIG: Diese Datei ist UTF-8 ohne BOM. Wer sie mit einem Werkzeug
//! anfasst, das die Bytes noch einmal als UTF-8 kodiert, macht aus "ä" ein
//! "Ã¤". Der Test `no_mojibake` unten faengt genau das ab.

use std::sync::atomic::{AtomicU8, Ordering};

/// 0 = Deutsch, 1 = English
static LANG: AtomicU8 = AtomicU8::new(0);

pub const LANGS: [(&str, &str); 2] = [("de", "Deutsch"), ("en", "English")];

pub fn set_lang(code: &str) {
    LANG.store(if code.starts_with("en") { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn lang() -> &'static str {
    if LANG.load(Ordering::Relaxed) == 1 {
        "en"
    } else {
        "de"
    }
}

/// key, Deutsch, English
const TABLE: &[(&str, &str, &str)] = &[
    // Navigation und Kopfzeile
    ("nav.start", "Start", "Home"),
    ("nav.devices", "Geräte", "Devices"),
    ("nav.settings", "Einstellungen", "Settings"),
    ("hdr.search", "Suchen und verbinden", "Search and connect"),
    ("st.ready", "Bereit", "Ready"),
    ("st.connecting", "Verbinde …", "Connecting …"),
    ("st.online", "Online", "Online"),
    ("st.offline", "Offline", "Offline"),
    // Startseite
    ("start.share", "Diesen PC teilen", "Share this PC"),
    ("start.your_id", "Ihre ID", "Your ID"),
    ("start.password", "Passwort", "Password"),
    ("start.keep_pw", "Passwort behalten", "Keep password"),
    (
        "start.keep_pw_tip",
        "An: gleiches Passwort nach jedem Neustart – nötig für unbeaufsichtigten Zugriff.",
        "On: same password after every restart – needed for unattended access.",
    ),
    (
        "start.pw_random",
        "Wird bei jedem Start neu gewürfelt. Feste Passwörter stehen in den Einstellungen unter Zugriff.",
        "Rolled fresh at every start. Permanent passwords live in Settings under Access.",
    ),
    ("start.copy", "Kopieren", "Copy"),
    ("start.new_pw", "Neues Passwort erzeugen", "Create a new password"),
    ("start.control", "Anderen PC steuern", "Control another PC"),
    ("start.partner", "Partner-ID", "Partner ID"),
    ("start.recent_short", "zuletzt", "recent"),
    (
        "start.pw_hint",
        "leer lassen für Anfrage",
        "leave empty to ask",
    ),
    ("start.remember", "Passwort merken", "Remember password"),
    ("start.connect", "Verbinden", "Connect"),
    ("start.ask", "Anfragen", "Ask"),
    (
        "start.ask_tip",
        "Ohne Passwort: die Person am anderen Rechner muss zulassen.",
        "Without a password: the person at the other end has to allow it.",
    ),
    ("start.game", "Spielmodus", "Game mode"),
    (
        "start.game_tip",
        "Rohe Maus und ganze Tastatur. Rechte Strg gibt die Eingabe wieder frei.",
        "Raw mouse and full keyboard. Right Ctrl hands input back.",
    ),
    ("start.voice", "Sprache", "Voice"),
    ("start.mic", "Mikrofon", "Microphone"),
    ("start.sound", "Ton", "Sound"),
    (
        "start.mic_tip",
        "Mikrofon in der laufenden Sitzung senden",
        "Send your microphone during the session",
    ),
    (
        "start.sound_tip",
        "Sprache der anderen Seite abspielen",
        "Play the other side's voice",
    ),
    ("start.recent", "Letzte Verbindungen", "Recent connections"),
    ("start.nosession", "Keine aktive Sitzung", "No active session"),
    // Geräte
    ("dev.count", "{} Geräte", "{} devices"),
    ("dev.add", "Hinzufügen", "Add"),
    (
        "dev.add_tip",
        "Die ID aus dem Suchfeld in die Liste legen",
        "Put the ID from the search field into the list",
    ),
    ("dev.refresh_tip", "Zustand neu abfragen", "Ask again who is online"),
    ("dev.name", "NAME", "NAME"),
    ("dev.id", "ID", "ID"),
    ("dev.status", "STATUS", "STATUS"),
    ("dev.group_recent", "Letzte Verbindungen", "Recent"),
    ("dev.group_fav", "Favoriten", "Favourites"),
    ("dev.group_online", "Online", "Online"),
    ("dev.group_offline", "Offline", "Offline"),
    ("dev.connect", "Verbinden", "Connect"),
    ("dev.rename", "Umbenennen", "Rename"),
    ("dev.fav_add", "Als Favorit", "Add to favourites"),
    ("dev.fav_del", "Favorit entfernen", "Remove favourite"),
    ("dev.remove", "Aus der Liste löschen", "Remove from the list"),
    (
        "dev.empty",
        "Noch keine Geräte – oben eine ID eingeben und verbinden.",
        "No devices yet – type an ID above and connect.",
    ),
    ("dev.nohit", "Kein Gerät passt zur Suche.", "No device matches."),
    ("dev.me", "(dieses Gerät)", "(this device)"),
    (
        "dev.secure",
        "Bereit – jede Sitzung ist Ende zu Ende verschlüsselt",
        "Ready – every session is end to end encrypted",
    ),
    ("dev.newname", "Neuer Name", "New name"),
    ("dev.save", "Speichern", "Save"),
    ("dev.cancel", "Abbrechen", "Cancel"),
    ("dev.id9", "Erst eine 9-stellige ID eingeben", "Enter a 9 digit ID first"),
    ("dev.add_title", "Gerät hinzufügen", "Add device"),
    ("dev.add_id", "FreeViewer-ID", "FreeViewer ID"),
    ("dev.add_id_hint", "9-stellige ID, z. B. 497 628 420", "9 digit ID, e.g. 497 628 420"),
    ("dev.add_name", "Name", "Name"),
    ("dev.add_pw", "Passwort", "Password"),
    ("dev.add_name_hint", "Wie das Gerät in der Liste heißen soll", "How the device is listed"),
    (
        "dev.add_pw_hint",
        "Passwort merken - dann verbindet ein Doppelklick sofort",
        "Remember the password - a double click connects right away",
    ),
    ("dev.add_ok", "Hinzufügen", "Add"),
    ("dev.add_bad_id", "Die ID besteht aus 9 Ziffern", "The ID is 9 digits long"),
    ("dev.add_known", "Dieses Gerät steht schon in der Liste", "That device is already listed"),
    ("dev.add_self", "Das ist diese Maschine", "That is this machine"),
    ("sess.window", "FreeViewer - Sitzung", "FreeViewer - session"),
    // Einstellungen
    ("set.general", "Allgemein", "General"),
    ("set.deploy", "Einrichten", "Rollout"),
    ("dep.title", "Neues Gerät per Link einrichten", "Set up a new device by link"),
    ("dep.lead", "Einmal-Link erzeugen, auf dem anderen Gerät öffnen, Namen vergeben – FreeViewer installiert sich dort (mit Dienst) und das Gerät taucht hier mit ID und Passwort auf.", "Create a one-time link, open it on the other device, pick a name - FreeViewer installs itself there (with service) and the device shows up here with ID and password."),
    ("dep.need_login", "Dafür musst du mit deinem Konto angemeldet sein.", "You need to be signed in for this."),
    ("dep.to_account", "Zum Konto", "Go to account"),
    ("dep.pw", "Passwort für das neue Gerät", "Password for the new device"),
    ("dep.pw_new", "Neu würfeln", "Roll again"),
    ("dep.name_hint", "Namens-Vorschlag (kann dort geändert werden)", "Name suggestion (can be changed there)"),
    ("dep.name_hint_ph", "z. B. Wohnzimmer-PC", "e.g. living-room PC"),
    ("dep.make", "Link erzeugen", "Create link"),
    ("dep.making", "Erzeuge …", "Creating …"),
    ("dep.link", "Diesen Link auf dem neuen Gerät öffnen:", "Open this link on the new device:"),
    ("dep.copy", "In die Zwischenablage", "Copy to clipboard"),
    ("dep.link_note", "Hat das Gerät schon FreeViewer, öffnet sich das Einrichtungs-Fenster. Sonst lädt der Link das kleine Setup herunter, das nach dem Code fragt.", "With FreeViewer present the setup window opens. Without, the link downloads the small setup which asks for the code."),
    ("dep.status", "Laufende und fertige Einrichtungen", "Pending and finished setups"),
    ("dep.none", "Noch keine Einrichtungen.", "No setups yet."),
    ("dep.group", "Neu eingerichtet", "Newly set up"),
    ("dep.arrived", "{} ist eingerichtet und liegt im Adressbuch.", "{} was set up and added to the address book."),
    ("dep.waiting", "wartet auf das andere Gerät", "waiting for the other device"),
    ("dep.count", "Geräte pro Link", "Devices per link"),
    ("dep.left", "noch {} frei", "{} left"),
    ("dep.revoke_tip", "Link widerrufen - er funktioniert sofort nicht mehr", "Revoke link - it stops working right away"),
    ("dep.unlimited", "ohne Limit", "no limit"),
    ("dep.unlimited_tip", "Der Link lässt sich beliebig oft einlösen, bis du ihn widerrufst.", "The link can be redeemed as often as you like until you revoke it."),
    ("dep.used_unlimited", "ohne Limit, {} eingerichtet", "no limit, {} set up"),
    ("dep.copied", "Link in der Zwischenablage.", "Link copied to clipboard."),
    ("upd.auto", "Automatisch aktualisieren", "Update automatically"),
    ("upd.by_service", "Der Dienst spielt das Update ein - gleich startet alles neu.", "The service applies the update - everything restarts in a moment."),
    ("upd.working", "Update läuft", "Updating"),
    ("upd.restart_soon", "FreeViewer wird aktualisiert und startet gleich neu.", "FreeViewer is updating and will restart in a moment."),
    ("setup.auto", "FreeViewer wird auf diesem Gerät eingerichtet …", "Setting up FreeViewer on this device …"),
    ("set.repair", "Reparieren", "Repair"),
    ("set.repair_running", "Reparatur läuft - Dateien werden frisch geschrieben.", "Repair running - files are written fresh."),
    ("set.uninstall_title", "Deinstallieren", "Uninstall"),
    ("set.uninstall_sure", "Bist du dir sicher? FreeViewer wird von diesem Gerät entfernt.", "Are you sure? FreeViewer will be removed from this device."),
    ("set.uninstall_keep", "Deine ID und dein Adressbuch bleiben erhalten - bei einer Neuinstallation ist alles wieder da.", "Your ID and address book are kept - a reinstall brings everything back."),
    ("set.uninstall_yes", "Ja, deinstallieren", "Yes, uninstall"),
    ("dep.done_row", "eingerichtet - im Adressbuch", "set up - in the address book"),
    ("setup.title", "Dieses Gerät einrichten", "Set up this device"),
    ("setup.lead", "Über diesen Einmal-Link wird FreeViewer hier richtig installiert (nach Programme, mit Dienst – also auch vor der Anmeldung erreichbar). Das Gerät taucht danach in der Liste des Absenders auf, fertig zum Verbinden.", "This one-time link installs FreeViewer properly (Program Files, with service - reachable before sign-in). The device then shows up in the sender's list, ready to connect."),
    ("setup.name", "Wie soll dieses Gerät heißen?", "What should this device be called?"),
    ("setup.ok", "Einrichten und installieren", "Set up and install"),
    ("setup.cancel", "Abbrechen", "Cancel"),
    ("setup.working", "Richte ein …", "Setting up …"),
    ("setup.done", "Fertig – die Installation läuft. {} sieht dieses Gerät jetzt in seiner Liste.", "Done - installation is running. {} now sees this device in the list."),
    ("setup.noname", "Bitte einen Namen eintragen.", "Please enter a name."),
    ("pwask.title", "Mit {} verbinden", "Connect to {}"),
    ("pwask.note", "Für dieses Gerät ist kein Passwort gespeichert. Passwort eintragen und direkt verbinden - oder eine Anfrage schicken, dann bestätigt es jemand am anderen Gerät.", "No password saved for this device. Enter the password to connect directly - or send a request for someone to confirm on the other device."),
    ("pwask.pw", "Passwort", "Password"),
    ("pwask.ask", "Anfrage senden", "Send request"),
    ("set.access", "Zugriff", "Access"),
    ("set.audio", "Ton", "Sound"),
    ("set.look", "Darstellung", "Appearance"),
    ("set.about", "Info", "About"),
    ("set.name", "Name in fremden Listen", "Name others see"),
    ("set.name_hint", "z. B. Büro-PC", "e.g. office PC"),
    ("set.lang", "Sprache", "Language"),
    ("set.autostart", "Mit Windows starten", "Start with Windows"),
    (
        "set.autostart_tip",
        "Startet unsichtbar in den Infobereich.",
        "Starts hidden in the notification area.",
    ),
    ("set.service", "Dienst einrichten", "Install service"),
    (
        "set.service_tip",
        "Auch am Sperr- und Anmeldebildschirm erreichbar, noch bevor sich jemand anmeldet. Fragt nach Administrator-Rechten.",
        "Reachable at the lock and logon screen, before anybody signs in. Asks for administrator rights.",
    ),
    ("set.template", "Vorlage", "Preset"),
    ("set.accent", "Akzentfarbe", "Accent colour"),
    ("set.accent_preset", "wie die Vorlage", "same as preset"),
    ("set.size", "Schriftgröße", "Text size"),
    ("set.radius", "Rundung", "Corners"),
    ("set.reset", "Zurücksetzen", "Reset"),
    ("set.preview", "Vorschau", "Preview"),
    ("set.config", "Ordner", "Folder"),
    ("set.relay", "Relay", "Relay"),
    ("set.version", "Version", "Version"),
    ("set.e2e", "Ende zu Ende verschlüsselt", "End to end encrypted"),
    // Ton
    ("set.audio_default", "Beim Verbinden", "When connecting"),
    ("set.mic_default", "Mikrofon gleich an", "Microphone on right away"),
    (
        "set.mic_default_tip",
        "Aus: jede Sitzung startet stumm, das Mikrofon lässt sich in der Sitzung anschalten.",
        "Off: every session starts muted, the microphone can be switched on during the session.",
    ),
    ("set.snd_default", "Ton der Gegenseite gleich an", "Sound of the other side on right away"),
    ("set.audio_now", "In dieser Sitzung", "In this session"),
    (
        "set.audio_note",
        "Sprache läuft im selben verschlüsselten Kanal wie das Bild: 24 kHz, 20-ms-Pakete, rund 97 kbit/s.",
        "Voice runs inside the same encrypted channel as the picture: 24 kHz, 20 ms packets, about 97 kbit/s.",
    ),
    ("set.mic_dev", "Mikrofon", "Microphone"),
    ("set.spk_dev", "Wiedergabe", "Playback"),
    ("set.dev_pick", "Gerät wählen", "Pick a device"),
    ("set.dev_default", "Standardgerät", "System default"),
    (
        "set.dev_default_tip",
        "Folgt dem, was Windows gerade als Standard führt.",
        "Follows whatever Windows currently uses as default.",
    ),
    ("set.dev_gone", "nicht mehr vorhanden", "no longer present"),
    ("set.dev_refresh", "Geräte neu einlesen", "Read devices again"),
    ("set.clip", "Zwischenablage teilen", "Share clipboard"),
    (
        "set.clip_tip",
        "Kopierter Text gilt auf beiden Rechnern - in beide Richtungen, nur Text.",
        "Copied text works on both machines - both ways, text only.",
    ),
    (
        "set.e2e_note",
        "Bild, Ton, Tastatur und Dateien laufen verschlüsselt (AES-256-GCM) direkt zwischen den beiden Rechnern. Der Relay leitet nur weiter und kann nichts mitlesen; nichts wird dort gespeichert.",
        "Picture, sound, keyboard and files run encrypted (AES-256-GCM) straight between the two machines. The relay only forwards and cannot read anything; nothing is stored there.",
    ),
    (
        "start.self",
        "Das ist dieser PC - eine Verbindung zu sich selbst bringt nichts.",
        "That is this PC - connecting to yourself gets you nowhere.",
    ),
    // Meet
    ("nav.meet", "Meet", "Meet"),
    ("meet.new", "Neues Meeting", "New meeting"),
    ("meet.title_hint", "Titel (freiwillig)", "Title (optional)"),
    ("meet.start", "Meeting starten", "Start meeting"),
    ("meet.join_head", "Einem Meeting beitreten", "Join a meeting"),
    ("meet.join", "Beitreten", "Join"),
    ("meet.id", "Meeting-ID", "Meeting ID"),
    ("meet.pw", "Passwort", "Password"),
    ("meet.running", "Laufende Meetings", "Meetings running"),
    ("meet.none", "Gerade läuft kein Meeting.", "No meeting running right now."),
    ("meet.refresh", "Neu laden", "Reload"),
    ("meet.copy_invite", "Einladung kopieren", "Copy invitation"),
    ("meet.copied", "Einladung liegt in der Zwischenablage.", "Invitation copied."),
    ("meet.created", "Meeting steht - ID und Passwort weitergeben.", "Meeting is up - pass on ID and password."),
    ("meet.opening", "Meeting wird geöffnet …", "Opening the meeting …"),
    ("meet.need_id", "Erst eine Meeting-ID eingeben.", "Enter a meeting ID first."),
    ("meet.err", "Meet-Server nicht erreichbar", "Meet server not reachable"),
    // Das eigene Meeting-Fenster (Zoom-Ablauf)
    ("meet.window", "FreeViewer Meet", "FreeViewer Meet"),
    ("meet.prejoin", "Kamera und Mikrofon", "Camera and microphone"),
    ("meet.cam", "Kamera", "Camera"),
    ("meet.mic", "Mikrofon", "Microphone"),
    (
        "meet.dev_default",
        "Standard (Browser wählt)",
        "Default (browser picks)",
    ),
    ("meet.mute_join", "Mikrofon stumm beitreten", "Join with microphone muted"),
    ("meet.camoff_join", "Kamera aus beitreten", "Join with camera off"),
    ("meet.join_now", "Jetzt beitreten", "Join now"),
    (
        "meet.reopen",
        "Meeting-Fenster erneut öffnen",
        "Reopen meeting window",
    ),
    (
        "meet.in_meeting",
        "Du bist im Meeting - Bild und Ton laufen im eigenen Fenster.",
        "You are in the meeting - picture and sound run in their own window.",
    ),
    ("meet.invite_head", "Einladung", "Invitation"),
    ("meet.people", "Teilnehmer", "Participants"),
    (
        "meet.people_none",
        "Noch niemand im Raum - Einladung weitergeben.",
        "Nobody in the room yet - pass on the invitation.",
    ),
    ("meet.toast_copied", "Link kopiert", "Link copied"),
    ("meet.host", "Gastgeber", "host"),
    ("meet.m_off", "Mikro aus", "mic off"),
    ("meet.c_off", "Kamera aus", "camera off"),
    ("meet.hand_up", "Hand oben", "hand up"),
    ("meet.devices_load", "Geräte werden eingelesen …", "Reading devices …"),
    (
        "meet.note",
        "Im Fenster laufen Bild, Ton und Bildschirmteilen wie im Browser. Wer die Maus des anderen übernehmen will, geht über Geräte - dafür braucht die Gegenseite FreeViewer.",
        "The window carries video, sound and screen sharing just like the browser. To take over someone's mouse use Devices - that side needs FreeViewer.",
    ),
    ("meet.offer", "Fernsteuerung anbieten", "Offer remote control"),
    (
        "meet.offer_tip",
        "Im Meeting erscheint dann ein Knopf, mit dem du die Steuerung dieses PCs freigibst. Jede Verbindung musst du hier trotzdem einzeln zulassen.",
        "A button appears in the meeting that hands over control of this PC. You still have to allow every single connection here.",
    ),
    (
        "link.control",
        "Verbindung zu {} - bitte am anderen Rechner zulassen",
        "Connecting to {} - please allow it on the other machine",
    ),
    // Installation
    ("set.install", "Installation", "Installation"),
    (
        "set.install_portable",
        "Portabel gestartet - nichts installiert",
        "Running portable - nothing installed",
    ),
    ("set.install_here", "Installiert in {}", "Installed in {}"),
    (
        "set.install_other",
        "Installiert in {} - diese Datei liegt woanders",
        "Installed in {} - this file lives somewhere else",
    ),
    (
        "set.install_note",
        "Installieren legt FreeViewer nach Programme, macht einen Startmenü-Eintrag (dann findet ihn die Windows-Suche) und trägt ihn in Apps & Features ein. Fragt einmal nach Administrator-Rechten.",
        "Installing puts FreeViewer into Program Files, adds a start menu entry (so Windows search finds it) and registers it in Apps & Features. Asks once for administrator rights.",
    ),
    ("set.install_do", "FreeViewer installieren", "Install FreeViewer"),
    ("set.install_with_service", "Dienst gleich mit einrichten", "Set up the service as well"),
    ("set.install_running", "Installation läuft - bitte die Rückfrage bestätigen.", "Installing - please confirm the prompt."),
    ("set.uninstall_do", "Deinstallieren", "Uninstall"),
    (
        "set.uninstall_running",
        "Wird entfernt - ID, Geräteliste und Passwörter bleiben erhalten.",
        "Removing - your ID, device list and passwords stay.",
    ),
    // Dauerpasswoerter
    ("set.pw_perm", "Feste Passwörter", "Permanent passwords"),
    (
        "set.pw_perm_tip",
        "Mit jedem dieser Passwörter kommt man auf diesen PC – zusätzlich zum zufälligen Sitzungspasswort. Sie überleben jeden Neustart.",
        "Every one of these gets you into this PC – on top of the random session password. They survive a restart.",
    ),
    ("set.pw_add", "Passwort hinzufügen", "Add password"),
    ("set.pw_label", "Bezeichnung", "Label"),
    ("set.pw_label_hint", "z. B. Handy", "e.g. phone"),
    ("set.pw_value", "Passwort", "Password"),
    ("set.pw_show", "Anzeigen", "Show"),
    ("set.pw_hide", "Verbergen", "Hide"),
    ("set.pw_del", "Löschen", "Delete"),
    ("set.pw_empty", "Noch keine festen Passwörter.", "No permanent passwords yet."),
    ("set.pw_min", "Mindestens 6 Zeichen.", "At least 6 characters."),
    ("set.pw_dup", "Das Passwort steht schon in der Liste.", "That password is already in the list."),
    ("set.pw_max", "Mehr als 10 feste Passwörter sind nicht sinnvoll.", "More than 10 permanent passwords make no sense."),
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
    ("sess.mic_on", "Mikrofon an", "Microphone on"),
    ("sess.mic_off", "Mikrofon aus", "Microphone off"),
    ("sess.snd_on", "Ton an", "Sound on"),
    ("sess.snd_off", "Ton aus", "Sound off"),
    ("sess.drop", "Dateien hier ablegen und senden", "Drop files here to send them"),
    ("sess.drop_none", "Keine aktive Sitzung – Datei nicht gesendet", "No active session – file not sent"),
    ("sess.drop_sent", "{} Datei(en) werden übertragen", "Sending {} file(s)"),
    // Sitzungsleiste
    ("sess.disconnect", "Trennen", "Disconnect"),
    ("sess.mode", "Modus:", "Mode:"),
    ("sess.mode_admin", "Fernwartung", "Remote support"),
    (
        "sess.mode_admin_tip",
        "Scharfes Bild, absolute Maus, Tastatur nur im Fenster",
        "Sharp picture, absolute mouse, keyboard only inside the window",
    ),
    ("sess.mode_game", "Spiel", "Game"),
    (
        "sess.mode_game_tip",
        "Relative Maus für Ingame-Kameras, komplette Tastatur (Win, Alt+Tab), mehr fps",
        "Relative mouse for in-game cameras, whole keyboard (Win, Alt+Tab), more fps",
    ),
    ("sess.screen", "Bildschirm", "Screen"),
    ("sess.files", "Dateien", "Files"),
    ("sess.send_file", "Datei senden …", "Send a file …"),
    ("sess.open_dir", "Empfangsordner öffnen", "Open the download folder"),
    (
        "sess.drop_tip",
        "Tipp: Dateien einfach ins Fenster ziehen",
        "Tip: just drag files onto the window",
    ),
    ("sess.keys", "Tasten senden", "Send keys"),
    ("sess.direct", "direkt", "direct"),
    ("sess.via_relay", "über Relay", "via relay"),
    ("sess.escape", "rechte Strg = raus", "right Ctrl = out"),
    (
        "sess.escape_tip",
        "Einmal drücken gibt Maus und Tastatur wieder an diesen PC zurück.\nDreimal schnell hintereinander beendet die Sitzung.",
        "Press once to hand mouse and keyboard back to this PC.\nThree times quickly ends the session.",
    ),
    (
        "sess.escape_long",
        "Rechte Strg = Eingabe freigeben  |  3x rechte Strg = Sitzung beenden",
        "Right Ctrl = release input  |  3x right Ctrl = end session",
    ),
    ("sess.grabbed", "Eingabe gegriffen – rechte Strg löst", "Input grabbed – right Ctrl releases"),
    ("sess.not_grabbed", "frei – ins Bild klicken zum Greifen", "free – click the picture to grab"),
    ("sess.waiting", "Warte auf Bild …", "Waiting for a picture …"),
    ("sess.full_on", "Vollbild (F11)", "Full screen (F11)"),
    ("sess.full_off", "Vollbild verlassen (F11)", "Leave full screen (F11)"),
    ("sess.pin", "Leiste festhalten", "Keep the bar visible"),
    ("sess.move_bar", "Ziehen verschiebt die Leiste", "Drag to move the bar"),
    ("sess.stall_head", "Gerade kommt kein Bild", "No picture coming in"),
    (
        "sess.stall_body",
        "Meist zeigt der andere PC gerade den Sperr- oder Anmeldebildschirm. Diesen Bereich darf nur der FreeViewer-Dienst sehen: dort in den Einstellungen unter Allgemein „Dienst einrichten“ anhaken. Sonst hilft ein Blick auf die Verbindung.",
        "Usually the other PC is showing the lock or sign-in screen. Only the FreeViewer service may see that: on that PC tick “Install service” in Settings under General. Otherwise check the connection.",
    ),
    // Konto
    ("set.account", "Konto", "Account"),
    ("acc.title", "FleiTec-Konto", "FleiTec account"),
    (
        "acc.why",
        "Freiwillig. Ohne Anmeldung funktioniert alles wie bisher – angemeldet steht deine Geräteliste auf jedem Rechner zur Verfügung.",
        "Optional. Everything works without signing in – signed in, your device list is the same on every PC.",
    ),
    ("acc.user", "Benutzername", "User name"),
    ("acc.pass", "Passwort", "Password"),
    ("acc.login", "Anmelden", "Sign in"),
    ("acc.logout", "Abmelden", "Sign out"),
    ("acc.logging_in", "Melde an …", "Signing in …"),
    ("acc.need_both", "Benutzername und Passwort eingeben.", "Please enter user name and password."),
    ("acc.hello", "Angemeldet als {}", "Signed in as {}"),
    ("acc.logged_out", "Abgemeldet. Die Geräteliste bleibt auf diesem PC.", "Signed out. The device list stays on this PC."),
    ("acc.expired", "Anmeldung abgelaufen – bitte neu anmelden.", "Sign-in expired – please sign in again."),
    ("acc.sync_now", "Jetzt abgleichen", "Sync now"),
    ("acc.syncing", "Gleiche ab …", "Syncing …"),
    ("acc.updated", "Geräteliste aktualisiert.", "Device list updated."),
    ("acc.uptodate", "Alles auf dem neuesten Stand.", "Everything up to date."),
    ("acc.last_sync", "Zuletzt abgeglichen", "Last sync"),
    ("acc.never", "noch nie", "never"),
    ("acc.count", "{} Geräte in der Liste", "{} devices in the list"),
    ("acc.privacy_head", "Was hochgeht", "What is uploaded"),
    (
        "acc.privacy",
        "Nur die Geräteliste: ID, Name, Ordner, Notiz und die Nutzungszahlen. Gespeicherte Passwörter bleiben auf diesem PC – sie sind mit der Kennung dieses Rechners verschlüsselt und wären woanders ohnehin wertlos. Bildschirm, Eingaben und Dateien laufen weiterhin Ende zu Ende verschlüsselt und niemals über das Konto.",
        "Only the device list: ID, name, folder, note and the usage counters. Saved passwords stay on this PC – they are encrypted with this machine's identity and would be worthless anywhere else. Screen, input and files stay end to end encrypted and never touch the account.",
    ),
    (
        "host.mac_permissions",
        "Damit dieser Mac freigegeben werden kann: Systemeinstellungen - Datenschutz & Sicherheit - Bildschirmaufnahme UND Bedienungshilfen fuer FreeViewer einschalten.",
        "To share this Mac: System Settings - Privacy & Security - turn on Screen Recording AND Accessibility for FreeViewer.",
    ),
];

pub fn t(key: &str) -> &'static str {
    let en = LANG.load(Ordering::Relaxed) == 1;
    for (k, de, e) in TABLE.iter() {
        if *k == key {
            return if en { e } else { de };
        }
    }
    // Kein Eintrag: den Schlüssel zeigen, das fällt sofort auf
    Box::leak(key.to_string().into_boxed_str())
}

/// Text mit einem Platzhalter `{}`.
pub fn tf(key: &str, arg: &str) -> String {
    t(key).replacen("{}", arg, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_languages_are_filled() {
        for (k, de, en) in TABLE.iter() {
            assert!(!de.is_empty(), "{} ohne Deutsch", k);
            assert!(!en.is_empty(), "{} ohne Englisch", k);
        }
    }

    #[test]
    fn keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (k, _, _) in TABLE.iter() {
            assert!(seen.insert(*k), "Schlüssel {} doppelt", k);
        }
    }

    #[test]
    fn switching_language_works() {
        set_lang("de");
        assert_eq!(t("nav.devices"), "Geräte");
        set_lang("en");
        assert_eq!(t("nav.devices"), "Devices");
        set_lang("de");
    }

    #[test]
    fn placeholder_is_filled() {
        set_lang("de");
        assert_eq!(tf("dev.count", "7"), "7 Geräte");
    }

    #[test]
    fn german_texts_really_use_umlauts() {
        // genau der Fehler, den Justin gemeldet hat: "Geraete" statt "Geräte"
        let ascii_sins = [
            "Geraet", "hoeren", "Groesse", "waehrend", "zuruecksetzen", "traegt", "faellt",
            "haelt", "laeuft", "waehlen", "koennen", "muessen", "moechte", "loeschen",
            "Passwoert", "Rueck", "Buero", "zusaetzlich", "ueberleben", "Menue", "Kaestchen",
            "Verknuepf", "Pruef", "naechste", "schliess",
        ];
        for (k, de, _) in TABLE.iter() {
            for bad in ascii_sins.iter() {
                assert!(!de.contains(bad), "{} enthält {}", k, bad);
            }
        }
    }

    /// Doppelt kodiertes UTF-8 ("Ã¤" statt "ä") darf nie wieder durchrutschen.
    #[test]
    fn no_mojibake() {
        let sins = ["Ã", "Â", "â€", "ï»¿"];
        for (k, de, en) in TABLE.iter() {
            for bad in sins.iter() {
                assert!(!de.contains(bad), "{} deutsch ist kaputt kodiert: {}", k, de);
                assert!(!en.contains(bad), "{} englisch ist kaputt kodiert: {}", k, en);
            }
        }
    }
}
