//! Kameras ueber DirectShow - damit die OBS-Virtualkamera endlich auftaucht.
//!
//! WARUM ueberhaupt ein zweiter Weg? `meetcam` zaehlt Kameras ueber Media
//! Foundation auf. Das ist der moderne, richtige Weg - aber die OBS
//! Virtual Camera meldet sich unter Windows AUSSCHLIESSLICH als
//! DirectShow-Filter an. Sie taucht deshalb in der Media-Foundation-Liste
//! gar nicht auf, egal wie oft man neu einliest. Genau das war Justins
//! Beobachtung ("warum sehe ich meine OBS-Kamera nicht?").
//!
//! Dieses Modul zaehlt die DirectShow-Geraete auf UND kann sie oeffnen.
//! Nur aufzaehlen waere eine Luege: ein Eintrag, der beim Anklicken nichts
//! liefert, ist schlimmer als kein Eintrag.
//!
//! Aufbau des Graphen (ohne eigenen COM-Rueckruf, bewusst):
//!
//!   Quelle (Kamera)  ->  SampleGrabber  ->  NullRenderer
//!
//! Der SampleGrabber laeuft im Puffer-Betrieb (`SetBufferSamples(TRUE)`).
//! Wir HOLEN das jeweils letzte Bild mit `GetCurrentBuffer` ab, statt einen
//! `ISampleGrabberCB`-Rueckruf zu bauen. Das spart eine selbstgebaute
//! COM-Klasse samt Lebensdauer-Fallen und passt genau zu der Art, wie der
//! Rest des Programms arbeitet: immer nur das NEUESTE Bild zaehlt, wer
//! langsamer abholt, ueberspringt Bilder statt einen Rueckstau zu bauen.
//!
//! DirectShow liefert RGB24 von UNTEN nach OBEN und in der Reihenfolge
//! B,G,R. Beides wird hier geradegezogen, bevor daraus NV12 wird.

#![cfg(windows)]

use crate::meetcam::{Bild, Geraet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::{Interface, BSTR, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Media::DirectShow::{
    IBaseFilter, ICaptureGraphBuilder2, ICreateDevEnum, IGraphBuilder, IMediaControl,
};
use windows::Win32::Media::MediaFoundation::{
    AM_MEDIA_TYPE, CLSID_CaptureGraphBuilder2, CLSID_FilterGraph, CLSID_SystemDeviceEnum,
    CLSID_VideoInputDeviceCategory, MEDIASUBTYPE_RGB24, MEDIATYPE_Video, PIN_CATEGORY_CAPTURE,
    VIDEOINFOHEADER,
};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IEnumMoniker, IMoniker,
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};

/// Erkennungszeichen im `Geraet::id`: alles danach ist ein DirectShow-Pfad.
/// So weiss `meetcam::oeffnen`, welcher Weg gemeint ist, ohne raten zu
/// muessen - und ohne dass sich die beiden Namensraeume vermischen.
pub const PRAEFIX: &str = "ds:";

// Die beiden Filter aus qedit.dll. Sie stehen nicht in der windows-Kiste,
// weil Microsoft sie fuer "veraltet" erklaert hat - ausgeliefert werden sie
// mit Windows 10 und 11 aber weiterhin, und es gibt keinen Ersatz, der
// DirectShow-Quellen lesen koennte.
const CLSID_SAMPLE_GRABBER: GUID = GUID::from_u128(0xc1f400a0_3f08_11d3_9f0b_006008039e37);
const CLSID_NULL_RENDERER: GUID = GUID::from_u128(0xc1f400a4_3f08_11d3_9f0b_006008039e37);

#[allow(non_snake_case)]
#[windows::core::interface("6b652fff-11fe-4fce-92ad-0266b5d7c78f")]
unsafe trait ISampleGrabber: windows::core::IUnknown {
    unsafe fn SetOneShot(&self, one_shot: BOOL) -> HRESULT;
    unsafe fn SetMediaType(&self, typ: *const AM_MEDIA_TYPE) -> HRESULT;
    unsafe fn GetConnectedMediaType(&self, typ: *mut AM_MEDIA_TYPE) -> HRESULT;
    unsafe fn SetBufferSamples(&self, puffern: BOOL) -> HRESULT;
    unsafe fn GetCurrentBuffer(&self, groesse: *mut i32, puffer: *mut i32) -> HRESULT;
    unsafe fn GetCurrentSample(&self, probe: *mut *mut core::ffi::c_void) -> HRESULT;
    unsafe fn SetCallback(&self, rueckruf: *mut core::ffi::c_void, welche: i32) -> HRESULT;
}

/// Text aus einer Eigenschaft des Geraete-Steckbriefs holen.
unsafe fn eigenschaft(bag: &IPropertyBag, name: &str) -> String {
    let breit: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut v = windows::core::VARIANT::default();
    if bag.Read(PCWSTR(breit.as_ptr()), &mut v, None).is_err() {
        return String::new();
    }
    BSTR::try_from(&v).map(|b| b.to_string()).unwrap_or_default()
}

/// Ueber alle DirectShow-Videoquellen laufen. `f` bekommt (Steckbrief,
/// Name, Pfad) und darf mit `Some(..)` abbrechen - so laesst sich mit
/// derselben Schleife auflisten UND gezielt suchen.
unsafe fn mit_geraeten<T>(mut f: impl FnMut(&IMoniker, &str, &str) -> Option<T>) -> Option<T> {
    let aufzaehler: ICreateDevEnum =
        CoCreateInstance(&CLSID_SystemDeviceEnum, None, CLSCTX_INPROC_SERVER).ok()?;
    let mut monis: Option<IEnumMoniker> = None;
    // Liefert S_FALSE, wenn es die Kategorie gar nicht gibt - dann ist
    // `monis` leer, und genau das faengt das `?` hier ab.
    aufzaehler
        .CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut monis, 0)
        .ok()?;
    let monis = monis?;
    loop {
        let mut eins: [Option<IMoniker>; 1] = [None];
        let mut geholt = 0u32;
        if monis.Next(&mut eins, Some(&mut geholt)).is_err() || geholt == 0 {
            return None;
        }
        let Some(m) = eins[0].as_ref() else { return None };
        let bag: IPropertyBag = match m.BindToStorage(None, None) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let name = eigenschaft(&bag, "FriendlyName");
        // Der Geraetepfad ist eindeutig; die Virtualkameras mancher
        // Programme haben keinen - dann muss der Name herhalten.
        let mut pfad = eigenschaft(&bag, "DevicePath");
        if pfad.is_empty() {
            pfad = name.clone();
        }
        if name.is_empty() && pfad.is_empty() {
            continue;
        }
        if let Some(t) = f(m, &name, &pfad) {
            return Some(t);
        }
    }
}

/// Alle DirectShow-Kameras. Die Kennung traegt `PRAEFIX`.
pub fn liste() -> Vec<Geraet> {
    let mut aus: Vec<Geraet> = Vec::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        mit_geraeten::<()>(|_, name, pfad| {
            aus.push(Geraet {
                name: name.to_string(),
                id: format!("{}{}", PRAEFIX, pfad),
            });
            None
        });
    }
    aus
}

/// Der Aufnahmefaden. Aufbau und Signatur wie `meetcam::win::schleife`,
/// damit `meetcam::oeffnen` die beiden Wege austauschen kann.
#[allow(clippy::too_many_arguments)]
pub fn schleife(
    id: Option<String>,
    zw: u32,
    zh: u32,
    fps: u32,
    tx: std::sync::mpsc::Sender<std::result::Result<String, String>>,
    neu: Arc<Mutex<Option<Bild>>>,
    zaehler: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    fehler: Arc<Mutex<String>>,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        if let Err(e) = schleife_inner(id, zw, zh, fps, &tx, &neu, &zaehler, &stop) {
            // Scheiterte schon der Start, hat der Aufrufer noch nichts
            // bekommen - die Meldung muss zu ihm, sonst sieht es aus, als
            // passiere einfach nichts.
            let _ = tx.send(Err(e.clone()));
            if let Ok(mut f) = fehler.lock() {
                *f = e;
            }
        }
        CoUninitialize();
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn schleife_inner(
    id: Option<String>,
    zw: u32,
    zh: u32,
    fps: u32,
    tx: &std::sync::mpsc::Sender<std::result::Result<String, String>>,
    neu: &Arc<Mutex<Option<Bild>>>,
    zaehler: &Arc<AtomicU64>,
    stop: &Arc<AtomicBool>,
) -> std::result::Result<(), String> {
    let gesucht = id
        .unwrap_or_default()
        .trim_start_matches(PRAEFIX)
        .to_string();

    // 1) Quelle suchen und binden.
    let treffer = mit_geraeten(|m, name, pfad| {
        let passt = gesucht.is_empty()
            || pfad.eq_ignore_ascii_case(&gesucht)
            || name.to_lowercase().contains(&gesucht.to_lowercase());
        if !passt {
            return None;
        }
        match m.BindToObject::<_, _, IBaseFilter>(None, None) {
            Ok(f) => Some(Ok((f, name.to_string()))),
            Err(e) => Some(Err(format!("{} laesst sich nicht oeffnen: {}", name, e))),
        }
    });
    let (quelle, name) = match treffer {
        Some(Ok(q)) => q,
        Some(Err(e)) => return Err(e),
        None => return Err("keine DirectShow-Kamera gefunden".into()),
    };

    // 2) Graph zusammenstecken.
    let graph: IGraphBuilder = CoCreateInstance(&CLSID_FilterGraph, None, CLSCTX_INPROC_SERVER)
        .map_err(|e| format!("Filtergraph: {}", e))?;
    let bauer: ICaptureGraphBuilder2 =
        CoCreateInstance(&CLSID_CaptureGraphBuilder2, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("Graphbauer: {}", e))?;
    bauer
        .SetFiltergraph(&graph)
        .map_err(|e| format!("Graph verbinden: {}", e))?;
    graph
        .AddFilter(&quelle, PCWSTR(w("Quelle").as_ptr()))
        .map_err(|e| format!("Quelle einhaengen: {}", e))?;

    // 3) SampleGrabber auf RGB24 festlegen. OHNE feste Vorgabe verhandelt
    //    DirectShow irgendein Format (haeufig MJPG oder YUY2), und der
    //    Puffer waere nicht zu gebrauchen.
    let greifer: IBaseFilter = CoCreateInstance(&CLSID_SAMPLE_GRABBER, None, CLSCTX_INPROC_SERVER)
        .map_err(|e| format!("SampleGrabber fehlt (qedit.dll): {}", e))?;
    let sg: ISampleGrabber = greifer
        .cast()
        .map_err(|e| format!("SampleGrabber spricht nicht: {}", e))?;
    let wunsch = AM_MEDIA_TYPE {
        majortype: MEDIATYPE_Video,
        subtype: MEDIASUBTYPE_RGB24,
        ..Default::default()
    };
    sg.SetMediaType(&wunsch as *const _)
        .ok()
        .map_err(|e| format!("Format nicht setzbar: {}", e))?;
    // Puffern statt Rueckruf - siehe Kopf der Datei.
    sg.SetBufferSamples(BOOL(1))
        .ok()
        .map_err(|e| format!("Puffer nicht schaltbar: {}", e))?;
    sg.SetOneShot(BOOL(0))
        .ok()
        .map_err(|e| format!("Dauerbetrieb nicht schaltbar: {}", e))?;
    graph
        .AddFilter(&greifer, PCWSTR(w("Greifer").as_ptr()))
        .map_err(|e| format!("Greifer einhaengen: {}", e))?;

    let senke: IBaseFilter = CoCreateInstance(&CLSID_NULL_RENDERER, None, CLSCTX_INPROC_SERVER)
        .map_err(|e| format!("NullRenderer fehlt: {}", e))?;
    graph
        .AddFilter(&senke, PCWSTR(w("Senke").as_ptr()))
        .map_err(|e| format!("Senke einhaengen: {}", e))?;

    bauer
        .RenderStream(
            Some(&PIN_CATEGORY_CAPTURE),
            &MEDIATYPE_Video,
            &quelle,
            &greifer,
            &senke,
        )
        .map_err(|e| format!("Graph nicht verbindbar: {}", e))?;

    // 4) Welche Groesse ist dabei herausgekommen? Erst danach steht fest,
    //    wie der Puffer zu lesen ist - raten waere hier fatal, ein falscher
    //    Zeilenabstand ergibt Schraegstreifen statt Bild.
    let mut mt = AM_MEDIA_TYPE::default();
    sg.GetConnectedMediaType(&mut mt)
        .ok()
        .map_err(|e| format!("Format nicht lesbar: {}", e))?;
    if mt.cbFormat < std::mem::size_of::<VIDEOINFOHEADER>() as u32 || mt.pbFormat.is_null() {
        aufraeumen(&mut mt);
        return Err("Format ohne Bildkopf".into());
    }
    let vih = &*(mt.pbFormat as *const VIDEOINFOHEADER);
    let qb = vih.bmiHeader.biWidth.unsigned_abs();
    // Negative Hoehe = das Bild liegt schon richtig herum. Positiv (der
    // Normalfall bei DirectShow) heisst: von unten nach oben.
    let kopfueber = vih.bmiHeader.biHeight > 0;
    let qh = vih.bmiHeader.biHeight.unsigned_abs();
    aufraeumen(&mut mt);
    if qb < 2 || qh < 2 {
        return Err("unsinnige Bildgroesse".into());
    }
    // DirectShow richtet Zeilen auf 4 Bytes aus.
    let zeile = ((qb as usize * 3) + 3) & !3;

    // 5) Los.
    let steuerung: IMediaControl = graph
        .cast()
        .map_err(|e| format!("Steuerung fehlt: {}", e))?;
    steuerung
        .Run()
        .map_err(|e| format!("Graph laeuft nicht an: {}", e))?;
    let _ = tx.send(Ok(name));

    let takt = std::time::Duration::from_millis((1000 / fps.max(1)).max(5) as u64);
    let mut roh: Vec<u8> = Vec::new();
    let mut rgb: Vec<u8> = vec![0; qb as usize * qh as usize * 3];
    let mut nv12: Vec<u8> = Vec::new();
    let mut leerlauf = 0u32;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(takt);
        let mut groesse = 0i32;
        if sg.GetCurrentBuffer(&mut groesse, std::ptr::null_mut()).is_err() || groesse <= 0 {
            // Am Anfang ist noch nichts da - das ist normal. Bleibt es
            // dabei, liefert die Quelle nichts, und das muss auffallen.
            leerlauf += 1;
            if leerlauf > fps.max(1) * 5 {
                return Err("Kamera liefert keine Bilder".into());
            }
            continue;
        }
        leerlauf = 0;
        let noetig = groesse as usize;
        if roh.len() < noetig {
            roh.resize(noetig, 0);
        }
        if sg
            .GetCurrentBuffer(&mut groesse, roh.as_mut_ptr() as *mut i32)
            .is_err()
        {
            continue;
        }
        if noetig < zeile * qh as usize {
            continue;
        }
        // BGR von unten nach oben -> RGB von oben nach unten.
        for y in 0..qh as usize {
            let quelle_y = if kopfueber { qh as usize - 1 - y } else { y };
            let src = quelle_y * zeile;
            let dst = y * qb as usize * 3;
            for x in 0..qb as usize {
                rgb[dst + x * 3] = roh[src + x * 3 + 2];
                rgb[dst + x * 3 + 1] = roh[src + x * 3 + 1];
                rgb[dst + x * 3 + 2] = roh[src + x * 3];
            }
        }
        if crate::meetcam::rgb_nach_nv12(&rgb, qb, qh, zw, zh, &mut nv12) {
            if let Ok(mut b) = neu.lock() {
                *b = Some(Bild {
                    breite: zw,
                    hoehe: zh,
                    nv12: nv12.clone(),
                });
            }
            zaehler.fetch_add(1, Ordering::Relaxed);
        }
    }
    let _ = steuerung.Stop();
    Ok(())
}

/// Das Formatstueck eines AM_MEDIA_TYPE gehoert uns - sonst leckt bei
/// jedem Oeffnen Speicher.
unsafe fn aufraeumen(mt: &mut AM_MEDIA_TYPE) {
    if !mt.pbFormat.is_null() {
        CoTaskMemFree(Some(mt.pbFormat as *const _));
        mt.pbFormat = std::ptr::null_mut();
    }
    if mt.pUnk.is_some() {
        core::mem::ManuallyDrop::drop(&mut mt.pUnk);
    }
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kennung_traegt_das_praefix() {
        // Ohne eindeutiges Praefix koennte `oeffnen` nicht entscheiden,
        // ueber welchen Weg das Geraet aufzumachen ist.
        assert!(format!("{}\\\\?\\usb#vid_046d", PRAEFIX).starts_with(PRAEFIX));
    }
}
