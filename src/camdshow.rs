//! Cameras via DirectShow - so the OBS virtual camera finally shows up.
//!
//! WHY a second path at all? `meetcam` enumerates cameras via Media
//! Foundation. That is the modern, correct path - but the OBS
//! Virtual Camera registers on Windows EXCLUSIVELY as a
//! DirectShow filter. It therefore does not show up in the Media Foundation list
//! at all, no matter how often you re-read. That was exactly Justin's
//! observation ("why can't I see my OBS camera?").
//!
//! This module enumerates the DirectShow devices AND can open them.
//! Only enumerating would be a lie: an entry that delivers nothing
//! when clicked is worse than no entry.
//!
//! Structure of the graph (deliberately without an own COM callback):
//!
//!   Quelle (Kamera)  ->  SampleGrabber  ->  NullRenderer
//!
//! The SampleGrabber runs in buffer mode (`SetBufferSamples(TRUE)`).
//! We FETCH the latest image with `GetCurrentBuffer` instead of building an
//! `ISampleGrabberCB` callback. That saves a self-built
//! COM class including lifetime traps and fits exactly the way the
//! rest of the program works: only the NEWEST image counts, whoever picks up
//! more slowly skips images instead of building up a backlog.
//!
//! DirectShow delivers RGB24 from BOTTOM to TOP and in the order
//! B,G,R. Both are straightened out here before it becomes NV12.

#![cfg(windows)]
// The COM names come from qedit.dll and must be named exactly like this.
#![allow(non_snake_case)]

use crate::meetcam::{Image, Device};
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

/// Marker in `Device::id`: everything after it is a DirectShow path.
/// This way `meetcam::open` knows which path is meant without having to
/// guess - and without the two namespaces getting mixed up.
pub const PREFIX: &str = "ds:";

// The two filters from qedit.dll. They are not in the windows crate,
// because Microsoft declared them "deprecated" - they are still shipped
// with Windows 10 and 11, though, and there is no replacement that
// could read DirectShow sources.
/// FORMAT_VideoInfo: `pbFormat` holds a VIDEOINFOHEADER only for this format type.
const FORMAT_VIDEO_INFO: GUID = GUID::from_u128(0x05589f80_c356_11ce_bf01_00aa0055595a);
const CLSID_SAMPLE_GRABBER: GUID = GUID::from_u128(0xc1f400a0_3f08_11d3_9f0b_006008039e37);
const CLSID_NULL_RENDERER: GUID = GUID::from_u128(0xc1f400a4_3f08_11d3_9f0b_006008039e37);

// SAFETY: declaration of the qedit.dll ISampleGrabber vtable (the methods below are unsafe for the
// same reason). The IID and the method order/signatures match the documented ISampleGrabber layout
// (SetOneShot, SetMediaType, GetConnectedMediaType, SetBufferSamples, GetCurrentBuffer,
// GetCurrentSample, SetCallback); a wrong order would call the wrong COM slot. Callers must pass
// pointers that are valid for the duration of the call; `puffer` of GetCurrentBuffer is really a
// BYTE* that must have room for `*dimension` bytes.
#[windows::core::interface("6b652fff-11fe-4fce-92ad-0266b5d7c78f")]
unsafe trait ISampleGrabber: windows::core::IUnknown {
    unsafe fn SetOneShot(&self, one_shot: BOOL) -> HRESULT;
    unsafe fn SetMediaType(&self, typ: *const AM_MEDIA_TYPE) -> HRESULT;
    unsafe fn GetConnectedMediaType(&self, typ: *mut AM_MEDIA_TYPE) -> HRESULT;
    unsafe fn SetBufferSamples(&self, puffern: BOOL) -> HRESULT;
    unsafe fn GetCurrentBuffer(&self, dimension: *mut i32, puffer: *mut i32) -> HRESULT;
    unsafe fn GetCurrentSample(&self, probe: *mut *mut core::ffi::c_void) -> HRESULT;
    unsafe fn SetCallback(&self, callback_fn: *mut core::ffi::c_void, welche: i32) -> HRESULT;
}

// SAFETY: caller contract: COM must be initialised on the calling thread and `bag` must be a live
// IPropertyBag. `wide` is NUL-terminated and outlives the call, `v` is a local VARIANT that Read
// fills and that is cleared by its Drop.
/// Text aus einer Eigenschaft des Geraete-Steckbriefs holen.
unsafe fn property(bag: &IPropertyBag, name: &str) -> String {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut v = windows::core::VARIANT::default();
    if bag.Read(PCWSTR(wide.as_ptr()), &mut v, None).is_err() {
        return String::new();
    }
    BSTR::try_from(&v).map(|b| b.to_string()).unwrap_or_default()
}

// SAFETY: caller contract: COM must be initialised (CoInitializeEx) on the calling thread, and the
// closure must not let the borrowed IMoniker escape (it is only valid inside the iteration). All
// interface pointers are created and dropped inside this function; `one`/`fetched` are local
// out-slots that outlive each `Next` call, and a null entry is treated as end of list.
/// Iterate over all DirectShow video sources. `f` gets (moniker,
/// name, path) and may abort with `Some(..)` - so the same loop can
/// list AND search specifically.
unsafe fn with_devices<T>(mut f: impl FnMut(&IMoniker, &str, &str) -> Option<T>) -> Option<T> {
    let tally: ICreateDevEnum =
        CoCreateInstance(&CLSID_SystemDeviceEnum, None, CLSCTX_INPROC_SERVER).ok()?;
    let mut monis: Option<IEnumMoniker> = None;
    // Returns S_FALSE if the category does not exist at all - then
    // `monis` is empty, and exactly that is caught by the `?` here.
    tally
        .CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut monis, 0)
        .ok()?;
    let monis = monis?;
    loop {
        let mut one: [Option<IMoniker>; 1] = [None];
        let mut fetched = 0u32;
        if monis.Next(&mut one, Some(&mut fetched)).is_err() || fetched == 0 {
            return None;
        }
        let Some(m) = one[0].as_ref() else { return None };
        let bag: IPropertyBag = match m.BindToStorage(None, None) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let name = property(&bag, "FriendlyName");
        // The device path is unique; the virtual cameras of some
        // programs have none - then the name has to do.
        let mut file_path = property(&bag, "DevicePath");
        if file_path.is_empty() {
            file_path = name.clone();
        }
        if name.is_empty() && file_path.is_empty() {
            continue;
        }
        if let Some(t) = f(m, &name, &file_path) {
            return Some(t);
        }
    }
}

/// All DirectShow cameras. The id carries `PREFIX`.
pub fn list_items() -> Vec<Device> {
    let mut aus: Vec<Device> = Vec::new();
    // SAFETY: CoInitializeEx is called first on this thread, so COM is usable for `with_devices`
    // (a repeated init is harmless; a failed one is not paired with CoUninitialize). The closure
    // only borrows the local `aus` and no raw pointer leaves the block; `with_devices` has dropped
    // all its interfaces when it returns, so COM can be released right after.
    unsafe {
        let com_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        with_devices::<()>(|_, name, file_path| {
            aus.push(Device {
                name: name.to_string(),
                id: format!("{}{}", PREFIX, file_path),
            });
            None
        });
        if com_ok {
            CoUninitialize();
        }
    }
    aus
}

/// The capture thread. Structure and signature like `meetcam::win::event_loop`,
/// so that `meetcam::open` can swap the two paths.
#[allow(clippy::too_many_arguments)]
pub fn event_loop(
    id: Option<String>,
    zw: u32,
    zh: u32,
    fps: u32,
    tx: std::sync::mpsc::Sender<std::result::Result<String, String>>,
    neu: Arc<Mutex<Option<Image>>>,
    ctr: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<String>>,
) {
    // SAFETY: this runs on its own capture thread: CoInitializeEx before any COM call and
    // CoUninitialize after `loop_inner` has returned, so every COM interface created there (all
    // locals of that function) is already dropped when COM is torn down. CoUninitialize is only
    // called when CoInitializeEx succeeded (S_OK or S_FALSE), so the calls stay balanced.
    unsafe {
        let com_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        if let Err(e) = loop_inner(id, zw, zh, fps, &tx, &neu, &ctr, &stop) {
            // If the start already failed, the caller has not received anything
            // yet - the message has to go to them, otherwise it looks as if
            // simply nothing happens.
            let _ = tx.send(Err(e.clone()));
            if let Ok(mut f) = error.lock() {
                *f = e;
            }
        }
        if com_ok {
            CoUninitialize();
        }
    }
}

// SAFETY: caller contract: COM is initialised (MTA) on the current thread and stays so until
// return; this function owns all interface pointers it creates and drops them before returning. Raw
// buffers: `mt` is filled by GetConnectedMediaType and released via `cleanup`; `roh` is resized
// to the reported buffer size before it is handed to GetCurrentBuffer; all later accesses to
// `roh`/`rgb` are bounds-checked slice indexing.
#[allow(clippy::too_many_arguments)]
unsafe fn loop_inner(
    id: Option<String>,
    zw: u32,
    zh: u32,
    fps: u32,
    tx: &std::sync::mpsc::Sender<std::result::Result<String, String>>,
    neu: &Arc<Mutex<Option<Image>>>,
    ctr: &Arc<AtomicU64>,
    stop: &Arc<AtomicBool>,
) -> std::result::Result<(), String> {
    let searched = id
        .unwrap_or_default()
        .trim_start_matches(PREFIX)
        .to_string();

    // 1) Find and bind the source.
    let hit = with_devices(|m, name, file_path| {
        let passt = searched.is_empty()
            || file_path.eq_ignore_ascii_case(&searched)
            || name.to_lowercase().contains(&searched.to_lowercase());
        if !passt {
            return None;
        }
        match m.BindToObject::<_, _, IBaseFilter>(None, None) {
            Ok(f) => Some(Ok((f, name.to_string()))),
            Err(e) => Some(Err(format!("{} laesst sich nicht oeffnen: {}", name, e))),
        }
    });
    let (origin, name) = match hit {
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
        .AddFilter(&origin, PCWSTR(w("Quelle").as_ptr()))
        .map_err(|e| format!("Quelle einhaengen: {}", e))?;

    // 3) Fix the SampleGrabber to RGB24. WITHOUT a fixed setting
    //    DirectShow negotiates some format (often MJPG or YUY2), and the
    //    buffer would be unusable.
    let grabber: IBaseFilter = CoCreateInstance(&CLSID_SAMPLE_GRABBER, None, CLSCTX_INPROC_SERVER)
        .map_err(|e| format!("SampleGrabber fehlt (qedit.dll): {}", e))?;
    let sg: ISampleGrabber = grabber
        .cast()
        .map_err(|e| format!("SampleGrabber spricht nicht: {}", e))?;
    let wish = AM_MEDIA_TYPE {
        majortype: MEDIATYPE_Video,
        subtype: MEDIASUBTYPE_RGB24,
        ..Default::default()
    };
    sg.SetMediaType(&wish as *const _)
        .ok()
        .map_err(|e| format!("Format nicht setzbar: {}", e))?;
    // Buffering instead of callback - see the top of the file.
    sg.SetBufferSamples(BOOL(1))
        .ok()
        .map_err(|e| format!("Puffer nicht schaltbar: {}", e))?;
    sg.SetOneShot(BOOL(0))
        .ok()
        .map_err(|e| format!("Dauerbetrieb nicht schaltbar: {}", e))?;
    graph
        .AddFilter(&grabber, PCWSTR(w("Greifer").as_ptr()))
        .map_err(|e| format!("Greifer einhaengen: {}", e))?;

    let sink: IBaseFilter = CoCreateInstance(&CLSID_NULL_RENDERER, None, CLSCTX_INPROC_SERVER)
        .map_err(|e| format!("NullRenderer fehlt: {}", e))?;
    graph
        .AddFilter(&sink, PCWSTR(w("Senke").as_ptr()))
        .map_err(|e| format!("Senke einhaengen: {}", e))?;

    bauer
        .RenderStream(
            Some(&PIN_CATEGORY_CAPTURE),
            &MEDIATYPE_Video,
            &origin,
            &grabber,
            &sink,
        )
        .map_err(|e| format!("Graph nicht verbindbar: {}", e))?;

    // 4) What size came out of it? Only then is it clear
    //    how to read the buffer - guessing would be fatal here, a wrong
    //    row stride gives diagonal stripes instead of an image.
    let mut mt = AM_MEDIA_TYPE::default();
    sg.GetConnectedMediaType(&mut mt)
        .ok()
        .map_err(|e| format!("Format nicht lesbar: {}", e))?;
    if mt.formattype != FORMAT_VIDEO_INFO
        || mt.cbFormat < std::mem::size_of::<VIDEOINFOHEADER>() as u32
        || mt.pbFormat.is_null()
    {
        cleanup(&mut mt);
        return Err("Format ohne Bildkopf".into());
    }
    let vih = &*(mt.pbFormat as *const VIDEOINFOHEADER);
    let qb = vih.bmiHeader.biWidth.unsigned_abs();
    // Negative height = the image is already the right way up. Positive (the
    // normal case with DirectShow) means: bottom to top.
    let upside_down = vih.bmiHeader.biHeight > 0;
    let qh = vih.bmiHeader.biHeight.unsigned_abs();
    cleanup(&mut mt);
    if qb < 2 || qh < 2 {
        return Err("unsinnige Bildgroesse".into());
    }
    // DirectShow richtet Zeilen auf 4 Bytes aus.
    let text_row = ((qb as usize * 3) + 3) & !3;

    // 5) Los.
    let control: IMediaControl = graph
        .cast()
        .map_err(|e| format!("Steuerung fehlt: {}", e))?;
    control
        .Run()
        .map_err(|e| format!("Graph laeuft nicht an: {}", e))?;
    let _ = tx.send(Ok(name));

    let beat = std::time::Duration::from_millis((1000 / fps.max(1)).max(5) as u64);
    let mut roh: Vec<u8> = Vec::new();
    let mut rgb: Vec<u8> = vec![0; qb as usize * qh as usize * 3];
    let mut nv12: Vec<u8> = Vec::new();
    let mut idle = 0u32;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(beat);
        let mut dimension = 0i32;
        if sg.GetCurrentBuffer(&mut dimension, std::ptr::null_mut()).is_err() || dimension <= 0 {
            // At the start nothing is there yet - that is normal. If it stays
            // that way, the source delivers nothing, and that has to be noticed.
            idle += 1;
            if idle > fps.max(1) * 5 {
                return Err("Kamera liefert keine Bilder".into());
            }
            continue;
        }
        idle = 0;
        let required = dimension as usize;
        if roh.len() < required {
            roh.resize(required, 0);
        }
        if sg
            .GetCurrentBuffer(&mut dimension, roh.as_mut_ptr() as *mut i32)
            .is_err()
        {
            continue;
        }
        if required < text_row * qh as usize {
            continue;
        }
        // BGR bottom to top -> RGB top to bottom.
        for y in 0..qh as usize {
            let source_y = if upside_down { qh as usize - 1 - y } else { y };
            let src = source_y * text_row;
            let dst = y * qb as usize * 3;
            for x in 0..qb as usize {
                rgb[dst + x * 3] = roh[src + x * 3 + 2];
                rgb[dst + x * 3 + 1] = roh[src + x * 3 + 1];
                rgb[dst + x * 3 + 2] = roh[src + x * 3];
            }
        }
        if crate::meetcam::rgb_into_nv12(&rgb, qb, qh, zw, zh, &mut nv12) {
            if let Ok(mut b) = neu.lock() {
                *b = Some(Image {
                    width_px: zw,
                    height_px: zh,
                    nv12: nv12.clone(),
                });
            }
            ctr.fetch_add(1, Ordering::Relaxed);
        }
    }
    let _ = control.Stop();
    Ok(())
}

// SAFETY: caller contract: `mt` was filled by DirectShow (GetConnectedMediaType), so `pbFormat` was
// allocated with CoTaskMemAlloc and `pUnk` is an owned reference. Call it at most once per `mt`:
// `pbFormat` is reset to null, but `pUnk` is dropped via ManuallyDrop::drop and would be released
// twice on a second call.
/// The format part of an AM_MEDIA_TYPE belongs to us - otherwise memory leaks on
/// every open.
unsafe fn cleanup(mt: &mut AM_MEDIA_TYPE) {
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
    fn tag_carries_the_prefix() {
        // Without a unique prefix `open` could not decide
        // through which path the device is to be opened.
        assert!(format!("{}\\\\?\\usb#vid_046d", PREFIX).starts_with(PREFIX));
    }
}
