//! Sprachen. Jeder sichtbare Text steht genau einmal hier - Deutsch und
//! Englisch. Weitere Sprachen sind eine weitere Spalte, sonst nichts.
//!
//! Aufruf: `t("start.connect")`, mit Platzhalter `tf("dev.count", "7")`.

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
    (
        "dev.secure",
        "Bereit – jede Sitzung ist Ende zu Ende verschlüsselt",
        "Ready – every session is end to end encrypted",
    ),
    ("dev.newname", "Neuer Name", "New name"),
    ("dev.save", "Speichern", "Save"),
    ("dev.cancel", "Abbrechen", "Cancel"),
    ("dev.id9", "Erst eine 9-stellige ID eingeben", "Enter a 9 digit ID first"),
    // Einstellungen
    ("set.general", "Allgemein", "General"),
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
    (
        "set.audio_note",
        "Sprache läuft im selben verschlüsselten Kanal wie das Bild: 24 kHz, 20-ms-Pakete, rund 97 kbit/s.",
        "Voice runs inside the same encrypted channel as the picture: 24 kHz, 20 ms packets, about 97 kbit/s.",
    ),
    ("set.mic_dev", "Mikrofon", "Microphone"),
    ("set.spk_dev", "Wiedergabe", "Playback"),
    // Sitzung
    ("sess.mic_on", "Mikrofon an", "Microphone on"),
    ("sess.mic_off", "Mikrofon aus", "Microphone off"),
    ("sess.snd_on", "Ton an", "Sound on"),
    ("sess.snd_off", "Ton aus", "Sound off"),
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
        let ascii_sins = ["Geraete", "hoeren", "Groesse", "waehrend", "zuruecksetzen"];
        for (k, de, _) in TABLE.iter() {
            for bad in ascii_sins.iter() {
                assert!(!de.contains(bad), "{} enthält {}", k, bad);
            }
        }
    }
}
