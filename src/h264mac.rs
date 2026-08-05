//! H.264 auf dem Mac - VideoToolbox, in Hardware.
//!
//! Derselbe Vertrag wie die Windows-Fassung in `h264.rs`: NV12 hinein,
//! Annex-B heraus (und beim Dekodieren umgekehrt). Damit laufen Meeting,
//! Kamera und Bildschirmfreigabe auf dem Mac ueber genau denselben Weg wie
//! unter Windows - nur die Maschine darunter ist eine andere.
//!
//! Bewusst OHNE zusaetzliche Fremdbibliothek: die noetigen Apple-Funktionen
//! sind hier direkt deklariert. Das spart vier neue Abhaengigkeiten samt
//! ihrer Versions- und Merkmalsfallen; die Signaturen sind seit Jahren
//! unveraendert und stehen so in Apples Kopfdateien.

#![allow(non_upper_case_globals, non_snake_case)]

use super::{nv12_to_rgba, Chunk};
use anyhow::{anyhow, Result};
use std::ffi::{c_int, c_void};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------- Typen ----

type OSStatus = i32;
type OSType = u32;
type CFTypeRef = *const c_void;
type CFAllocatorRef = *const c_void;
type CFStringRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFNumberRef = *const c_void;
type CFBooleanRef = *const c_void;
type CVPixelBufferRef = *mut c_void;
type CVImageBufferRef = *mut c_void;
type CMSampleBufferRef = *mut c_void;
type CMBlockBufferRef = *mut c_void;
type CMFormatDescriptionRef = *const c_void;
type VTCompressionSessionRef = *mut c_void;
type VTDecompressionSessionRef = *mut c_void;
type VTSessionRef = *mut c_void;

const kCMVideoCodecType_H264: OSType = 0x6176_6331; // 'avc1'
const kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange: OSType = 0x3432_3076; // '420v'
const kCVPixelFormatType_420YpCbCr8BiPlanarFullRange: OSType = 0x3432_3066; // '420f'
const kCFNumberSInt32Type: c_int = 3;
const kCFNumberFloat64Type: c_int = 6;
/// Der Dekodierer soll das Bild sofort liefern, nicht in einer Warteschlange.
const kVTDecodeFrame_EnableTemporalProcessing: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

impl CMTime {
    fn neu(value: i64, timescale: i32) -> CMTime {
        CMTime {
            value,
            timescale,
            flags: 1, // kCMTimeFlags_Valid
            epoch: 0,
        }
    }
    fn ungueltig() -> CMTime {
        CMTime {
            value: 0,
            timescale: 0,
            flags: 0,
            epoch: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CMSampleTimingInfo {
    duration: CMTime,
    presentation: CMTime,
    decode: CMTime,
}

#[repr(C)]
struct VTDecompressionOutputCallbackRecord {
    callback: Option<
        unsafe extern "C" fn(*mut c_void, *mut c_void, OSStatus, u32, CVImageBufferRef, CMTime, CMTime),
    >,
    refcon: *mut c_void,
}

type VTCompressionOutputCallback =
    Option<unsafe extern "C" fn(*mut c_void, *mut c_void, OSStatus, u32, CMSampleBufferRef)>;

// ------------------------------------------------------------- Apple-API ---

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFNumberCreate(alloc: CFAllocatorRef, art: c_int, wert: *const c_void) -> CFNumberRef;
    fn CFDictionaryCreate(
        alloc: CFAllocatorRef,
        keys: *const *const c_void,
        values: *const *const c_void,
        anzahl: isize,
        key_cb: *const c_void,
        val_cb: *const c_void,
    ) -> CFDictionaryRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    static kCFBooleanTrue: CFBooleanRef;
    static kCFBooleanFalse: CFBooleanRef;
}

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferCreate(
        alloc: CFAllocatorRef,
        breite: usize,
        hoehe: usize,
        format: OSType,
        attrs: CFDictionaryRef,
        raus: *mut CVPixelBufferRef,
    ) -> i32;
    fn CVPixelBufferLockBaseAddress(pb: CVPixelBufferRef, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pb: CVPixelBufferRef, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(pb: CVPixelBufferRef, ebene: usize) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRowOfPlane(pb: CVPixelBufferRef, ebene: usize) -> usize;
    fn CVPixelBufferGetWidthOfPlane(pb: CVPixelBufferRef, ebene: usize) -> usize;
    fn CVPixelBufferGetHeightOfPlane(pb: CVPixelBufferRef, ebene: usize) -> usize;
    fn CVPixelBufferGetWidth(pb: CVPixelBufferRef) -> usize;
    fn CVPixelBufferGetHeight(pb: CVPixelBufferRef) -> usize;
    fn CVPixelBufferGetPixelFormatType(pb: CVPixelBufferRef) -> OSType;
}

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    fn CMSampleBufferGetDataBuffer(sb: CMSampleBufferRef) -> CMBlockBufferRef;
    fn CMSampleBufferGetFormatDescription(sb: CMSampleBufferRef) -> CMFormatDescriptionRef;
    fn CMBlockBufferGetDataPointer(
        bb: CMBlockBufferRef,
        offset: usize,
        laenge_hier: *mut usize,
        laenge_gesamt: *mut usize,
        daten: *mut *mut u8,
    ) -> OSStatus;
    fn CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
        fd: CMFormatDescriptionRef,
        index: usize,
        zeiger: *mut *const u8,
        groesse: *mut usize,
        anzahl: *mut usize,
        nal_kopf: *mut c_int,
    ) -> OSStatus;
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        alloc: CFAllocatorRef,
        anzahl: usize,
        zeiger: *const *const u8,
        groessen: *const usize,
        nal_kopf: c_int,
        raus: *mut CMFormatDescriptionRef,
    ) -> OSStatus;
    fn CMBlockBufferCreateWithMemoryBlock(
        struct_alloc: CFAllocatorRef,
        speicher: *mut c_void,
        block_laenge: usize,
        block_alloc: CFAllocatorRef,
        quelle: *const c_void,
        offset: usize,
        laenge: usize,
        flags: u32,
        raus: *mut CMBlockBufferRef,
    ) -> OSStatus;
    fn CMBlockBufferReplaceDataBytes(
        quelle: *const c_void,
        bb: CMBlockBufferRef,
        offset: usize,
        laenge: usize,
    ) -> OSStatus;
    fn CMSampleBufferCreateReady(
        alloc: CFAllocatorRef,
        daten: CMBlockBufferRef,
        format: CMFormatDescriptionRef,
        anzahl_proben: isize,
        anzahl_zeiten: isize,
        zeiten: *const CMSampleTimingInfo,
        anzahl_groessen: isize,
        groessen: *const usize,
        raus: *mut CMSampleBufferRef,
    ) -> OSStatus;
}

#[link(name = "VideoToolbox", kind = "framework")]
extern "C" {
    fn VTCompressionSessionCreate(
        alloc: CFAllocatorRef,
        breite: i32,
        hoehe: i32,
        codec: OSType,
        enc_spec: CFDictionaryRef,
        quell_attrs: CFDictionaryRef,
        daten_alloc: CFAllocatorRef,
        rueckruf: VTCompressionOutputCallback,
        refcon: *mut c_void,
        raus: *mut VTCompressionSessionRef,
    ) -> OSStatus;
    fn VTCompressionSessionEncodeFrame(
        s: VTCompressionSessionRef,
        bild: CVImageBufferRef,
        pts: CMTime,
        dauer: CMTime,
        rahmen_eigenschaften: CFDictionaryRef,
        quell_refcon: *mut c_void,
        info: *mut u32,
    ) -> OSStatus;
    fn VTCompressionSessionCompleteFrames(s: VTCompressionSessionRef, bis: CMTime) -> OSStatus;
    fn VTCompressionSessionInvalidate(s: VTCompressionSessionRef);
    fn VTDecompressionSessionCreate(
        alloc: CFAllocatorRef,
        format: CMFormatDescriptionRef,
        dec_spec: CFDictionaryRef,
        ziel_attrs: CFDictionaryRef,
        rueckruf: *const VTDecompressionOutputCallbackRecord,
        raus: *mut VTDecompressionSessionRef,
    ) -> OSStatus;
    fn VTDecompressionSessionDecodeFrame(
        s: VTDecompressionSessionRef,
        probe: CMSampleBufferRef,
        flags: u32,
        quell_refcon: *mut c_void,
        info: *mut u32,
    ) -> OSStatus;
    fn VTDecompressionSessionWaitForAsynchronousFrames(s: VTDecompressionSessionRef) -> OSStatus;
    fn VTDecompressionSessionInvalidate(s: VTDecompressionSessionRef);
    fn VTSessionSetProperty(s: VTSessionRef, schluessel: CFStringRef, wert: CFTypeRef) -> OSStatus;

    static kVTCompressionPropertyKey_RealTime: CFStringRef;
    static kVTCompressionPropertyKey_AllowFrameReordering: CFStringRef;
    static kVTCompressionPropertyKey_MaxKeyFrameInterval: CFStringRef;
    static kVTCompressionPropertyKey_AverageBitRate: CFStringRef;
    static kVTCompressionPropertyKey_ExpectedFrameRate: CFStringRef;
    static kVTCompressionPropertyKey_ProfileLevel: CFStringRef;
    static kVTProfileLevel_H264_Baseline_AutoLevel: CFStringRef;
    static kVTEncodeFrameOptionKey_ForceKeyFrame: CFStringRef;
}

// ----------------------------------------------------------- Hilfsmittel ---

fn zahl_i32(v: i32) -> CFNumberRef {
    unsafe { CFNumberCreate(std::ptr::null(), kCFNumberSInt32Type, &v as *const i32 as *const c_void) }
}

fn zahl_f64(v: f64) -> CFNumberRef {
    unsafe { CFNumberCreate(std::ptr::null(), kCFNumberFloat64Type, &v as *const f64 as *const c_void) }
}

/// Ein Woerterbuch mit genau einem Eintrag - mehr braucht hier niemand.
fn dict1(schluessel: CFStringRef, wert: CFTypeRef) -> CFDictionaryRef {
    unsafe {
        let k = [schluessel as *const c_void];
        let v = [wert];
        CFDictionaryCreate(
            std::ptr::null(),
            k.as_ptr(),
            v.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const c_void,
            &kCFTypeDictionaryValueCallBacks as *const c_void,
        )
    }
}

/// Zerlegt einen Annex-B-Strom in seine NAL-Einheiten (ohne Startmarken).
fn nal_zerlegen(au: &[u8]) -> Vec<(usize, usize)> {
    let mut teile = Vec::new();
    let mut i = 0usize;
    let mut start: Option<usize> = None;
    while i + 2 < au.len() {
        let drei = au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 1;
        let vier = i + 3 < au.len() && au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 0 && au[i + 3] == 1;
        if drei || vier {
            let marke = if vier { 4 } else { 3 };
            if let Some(s) = start.take() {
                if i > s {
                    teile.push((s, i - s));
                }
            }
            i += marke;
            start = Some(i);
            continue;
        }
        i += 1;
    }
    if let Some(s) = start {
        if au.len() > s {
            teile.push((s, au.len() - s));
        }
    }
    teile
}

fn nal_typ(b: u8) -> u8 {
    b & 0x1f
}

// -------------------------------------------------------------- Kodierer ---

/// Was der Rueckruf des Kodierers einsammelt.
#[derive(Default)]
struct Ausgabe {
    fertig: Vec<Chunk>,
    fehler: Option<String>,
}

pub struct Encoder {
    session: VTCompressionSessionRef,
    pixel: CVPixelBufferRef,
    w: u32,
    h: u32,
    fps: u32,
    zaehler: i64,
    schluessel_anfordern: bool,
    ausgabe: Arc<Mutex<Ausgabe>>,
}

// Die Sitzung selbst ist von Apple aus fadensicher; der Rest liegt hinter
// einem Mutex. Ohne das liesse sich der Kodierer nicht in den Arbeitsfaden
// des Meetings stecken.
unsafe impl Send for Encoder {}

unsafe extern "C" fn kodier_rueckruf(
    refcon: *mut c_void,
    _quelle: *mut c_void,
    status: OSStatus,
    _info: u32,
    probe: CMSampleBufferRef,
) {
    if refcon.is_null() {
        return;
    }
    let ausgabe = &*(refcon as *const Mutex<Ausgabe>);
    if status != 0 || probe.is_null() {
        if let Ok(mut a) = ausgabe.lock() {
            a.fehler = Some(format!("Kodierer meldet {}", status));
        }
        return;
    }
    // 1) Die Nutzdaten liegen als AVCC vor: je NAL eine Laenge davor.
    let bb = CMSampleBufferGetDataBuffer(probe);
    if bb.is_null() {
        return;
    }
    let (mut hier, mut gesamt, mut zeiger) = (0usize, 0usize, std::ptr::null_mut::<u8>());
    if CMBlockBufferGetDataPointer(bb, 0, &mut hier, &mut gesamt, &mut zeiger) != 0 || zeiger.is_null()
    {
        return;
    }
    let avcc = std::slice::from_raw_parts(zeiger, gesamt);

    // 2) Nach Annex-B umschreiben (Startmarke statt Laenge) - das ist das
    //    Format, das der Rest des Programms und str0m erwarten.
    let mut annexb: Vec<u8> = Vec::with_capacity(gesamt + 64);
    let mut idr = false;
    let mut i = 0usize;
    while i + 4 <= avcc.len() {
        let len = u32::from_be_bytes([avcc[i], avcc[i + 1], avcc[i + 2], avcc[i + 3]]) as usize;
        i += 4;
        if len == 0 || i + len > avcc.len() {
            break;
        }
        if nal_typ(avcc[i]) == 5 {
            idr = true;
        }
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(&avcc[i..i + len]);
        i += len;
    }
    if annexb.is_empty() {
        return;
    }

    // 3) Vor jedem Schluesselbild muessen SPS und PPS stehen, sonst kann
    //    niemand einsteigen, der spaeter dazukommt.
    if idr {
        let fd = CMSampleBufferGetFormatDescription(probe);
        if !fd.is_null() {
            let mut kopf: Vec<u8> = Vec::new();
            let mut anzahl = 0usize;
            let (mut p, mut groesse, mut nal_len) = (std::ptr::null::<u8>(), 0usize, 0 as c_int);
            if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                fd,
                0,
                &mut p,
                &mut groesse,
                &mut anzahl,
                &mut nal_len,
            ) == 0
            {
                for idx in 0..anzahl {
                    let (mut pp, mut gg) = (std::ptr::null::<u8>(), 0usize);
                    if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                        fd,
                        idx,
                        &mut pp,
                        &mut gg,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    ) == 0
                        && !pp.is_null()
                        && gg > 0
                    {
                        kopf.extend_from_slice(&[0, 0, 0, 1]);
                        kopf.extend_from_slice(std::slice::from_raw_parts(pp, gg));
                    }
                }
            }
            if !kopf.is_empty() {
                kopf.extend_from_slice(&annexb);
                annexb = kopf;
            }
        }
    }
    if let Ok(mut a) = ausgabe.lock() {
        a.fertig.push(Chunk {
            data: annexb,
            key: idr,
        });
    }
}

impl Encoder {
    pub fn new(w: u32, h: u32, fps: u32, bitrate: u32) -> Result<Self> {
        if w < 16 || h < 16 {
            return Err(anyhow!("Bild zu klein fuer H.264"));
        }
        // Gerade Kantenlaengen - alles andere mag kein 4:2:0.
        let (w, h) = (w & !1, h & !1);
        let ausgabe = Arc::new(Mutex::new(Ausgabe::default()));
        let refcon = Arc::as_ptr(&ausgabe) as *mut c_void;
        let mut session: VTCompressionSessionRef = std::ptr::null_mut();
        let st = unsafe {
            VTCompressionSessionCreate(
                std::ptr::null(),
                w as i32,
                h as i32,
                kCMVideoCodecType_H264,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                Some(kodier_rueckruf),
                refcon,
                &mut session,
            )
        };
        if st != 0 || session.is_null() {
            return Err(anyhow!("VideoToolbox-Kodierer nicht verfuegbar ({})", st));
        }
        unsafe {
            let setzen = |k: CFStringRef, v: CFTypeRef, freigeben: bool| {
                VTSessionSetProperty(session, k, v);
                if freigeben && !v.is_null() {
                    CFRelease(v);
                }
            };
            setzen(kVTCompressionPropertyKey_RealTime, kCFBooleanTrue, false);
            setzen(
                kVTCompressionPropertyKey_AllowFrameReordering,
                kCFBooleanFalse,
                false,
            );
            setzen(
                kVTCompressionPropertyKey_ProfileLevel,
                kVTProfileLevel_H264_Baseline_AutoLevel,
                false,
            );
            setzen(
                kVTCompressionPropertyKey_AverageBitRate,
                zahl_i32(bitrate.max(200_000) as i32),
                true,
            );
            setzen(
                kVTCompressionPropertyKey_ExpectedFrameRate,
                zahl_f64(fps.max(1) as f64),
                true,
            );
            // Alle zwei Sekunden ein Schluesselbild - dazwischen holt sich
            // jeder Neuling eines ueber die Anforderung.
            setzen(
                kVTCompressionPropertyKey_MaxKeyFrameInterval,
                zahl_i32((fps.max(1) * 2) as i32),
                true,
            );
        }
        // Ein einziger Pixelpuffer, der immer wieder gefuellt wird.
        let mut pixel: CVPixelBufferRef = std::ptr::null_mut();
        let st = unsafe {
            CVPixelBufferCreate(
                std::ptr::null(),
                w as usize,
                h as usize,
                kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                std::ptr::null(),
                &mut pixel,
            )
        };
        if st != 0 || pixel.is_null() {
            unsafe {
                VTCompressionSessionInvalidate(session);
                CFRelease(session as CFTypeRef);
            }
            return Err(anyhow!("Pixelpuffer nicht angelegt ({})", st));
        }
        Ok(Encoder {
            session,
            pixel,
            w,
            h,
            fps: fps.max(1),
            zaehler: 0,
            schluessel_anfordern: true,
            ausgabe,
        })
    }

    pub fn name(&self) -> &str {
        "VideoToolbox H.264"
    }

    pub fn hardware(&self) -> bool {
        true
    }

    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    pub fn nv12_len(&self) -> usize {
        (self.w as usize * self.h as usize) * 3 / 2
    }

    pub fn request_keyframe(&mut self) {
        self.schluessel_anfordern = true;
    }

    pub fn encode(&mut self, nv12: &[u8]) -> Result<Vec<Chunk>> {
        if nv12.len() < self.nv12_len() {
            return Err(anyhow!(
                "NV12 zu kurz ({} statt {})",
                nv12.len(),
                self.nv12_len()
            ));
        }
        let (w, h) = (self.w as usize, self.h as usize);
        unsafe {
            if CVPixelBufferLockBaseAddress(self.pixel, 0) != 0 {
                return Err(anyhow!("Pixelpuffer laesst sich nicht sperren"));
            }
            // Y-Ebene
            let y = CVPixelBufferGetBaseAddressOfPlane(self.pixel, 0) as *mut u8;
            let ys = CVPixelBufferGetBytesPerRowOfPlane(self.pixel, 0);
            if !y.is_null() {
                for zeile in 0..h {
                    std::ptr::copy_nonoverlapping(
                        nv12.as_ptr().add(zeile * w),
                        y.add(zeile * ys),
                        w,
                    );
                }
            }
            // UV-Ebene (halbe Hoehe, gleiche Breite)
            let uv = CVPixelBufferGetBaseAddressOfPlane(self.pixel, 1) as *mut u8;
            let uvs = CVPixelBufferGetBytesPerRowOfPlane(self.pixel, 1);
            if !uv.is_null() {
                for zeile in 0..h / 2 {
                    std::ptr::copy_nonoverlapping(
                        nv12.as_ptr().add(w * h + zeile * w),
                        uv.add(zeile * uvs),
                        w,
                    );
                }
            }
            CVPixelBufferUnlockBaseAddress(self.pixel, 0);

            let pts = CMTime::neu(self.zaehler, self.fps as i32);
            let dauer = CMTime::neu(1, self.fps as i32);
            self.zaehler += 1;
            let props = if self.schluessel_anfordern {
                self.schluessel_anfordern = false;
                dict1(kVTEncodeFrameOptionKey_ForceKeyFrame, kCFBooleanTrue)
            } else {
                std::ptr::null()
            };
            let mut info = 0u32;
            let st = VTCompressionSessionEncodeFrame(
                self.session,
                self.pixel,
                pts,
                dauer,
                props,
                std::ptr::null_mut(),
                &mut info,
            );
            if !props.is_null() {
                CFRelease(props);
            }
            if st != 0 {
                return Err(anyhow!("EncodeFrame: {}", st));
            }
            // Auf das Ergebnis warten: der Rueckruf laeuft sonst irgendwann
            // spaeter, und wir wollen den Rahmen JETZT verschicken.
            VTCompressionSessionCompleteFrames(self.session, CMTime::ungueltig());
        }
        let mut a = self
            .ausgabe
            .lock()
            .map_err(|_| anyhow!("Kodierer-Ausgabe blockiert"))?;
        if let Some(f) = a.fehler.take() {
            return Err(anyhow!(f));
        }
        Ok(std::mem::take(&mut a.fertig))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            VTCompressionSessionCompleteFrames(self.session, CMTime::ungueltig());
            VTCompressionSessionInvalidate(self.session);
            CFRelease(self.session as CFTypeRef);
            if !self.pixel.is_null() {
                CFRelease(self.pixel as CFTypeRef);
            }
        }
    }
}

// ------------------------------------------------------------ Dekodierer ---

#[derive(Default)]
struct Bild {
    breite: u32,
    hoehe: u32,
    rgba: Vec<u8>,
    da: bool,
    fehler: Option<String>,
}

pub struct Decoder {
    session: VTDecompressionSessionRef,
    format: CMFormatDescriptionRef,
    sps: Vec<u8>,
    pps: Vec<u8>,
    w: u32,
    h: u32,
    bild: Arc<Mutex<Bild>>,
}

unsafe impl Send for Decoder {}

unsafe extern "C" fn dekodier_rueckruf(
    refcon: *mut c_void,
    _quelle: *mut c_void,
    status: OSStatus,
    _info: u32,
    bild: CVImageBufferRef,
    _pts: CMTime,
    _dauer: CMTime,
) {
    if refcon.is_null() {
        return;
    }
    let ziel = &*(refcon as *const Mutex<Bild>);
    if status != 0 || bild.is_null() {
        if let Ok(mut z) = ziel.lock() {
            if status != 0 {
                z.fehler = Some(format!("Dekodierer meldet {}", status));
            }
        }
        return;
    }
    let fmt = CVPixelBufferGetPixelFormatType(bild);
    if fmt != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        && fmt != kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
    {
        if let Ok(mut z) = ziel.lock() {
            z.fehler = Some(format!("unerwartetes Pixelformat {:x}", fmt));
        }
        return;
    }
    if CVPixelBufferLockBaseAddress(bild, 1) != 0 {
        return;
    }
    let w = CVPixelBufferGetWidth(bild) as u32;
    let h = CVPixelBufferGetHeight(bild) as u32;
    let yp = CVPixelBufferGetBaseAddressOfPlane(bild, 0) as *const u8;
    let ys = CVPixelBufferGetBytesPerRowOfPlane(bild, 0);
    let uvp = CVPixelBufferGetBaseAddressOfPlane(bild, 1) as *const u8;
    let uvs = CVPixelBufferGetBytesPerRowOfPlane(bild, 1);
    let yh = CVPixelBufferGetHeightOfPlane(bild, 0);
    let _uvw = CVPixelBufferGetWidthOfPlane(bild, 1);
    if !yp.is_null() && !uvp.is_null() && w > 0 && h > 0 {
        // Die beiden Ebenen liegen NICHT zwingend hintereinander - deshalb
        // erst in einen zusammenhaengenden NV12-Block kopieren und den
        // gemeinsamen Umrechner benutzen.
        let stride = ys.max(uvs);
        let mut nv12 = vec![0u8; stride * yh + stride * (h as usize).div_ceil(2)];
        for zeile in 0..h as usize {
            std::ptr::copy_nonoverlapping(
                yp.add(zeile * ys),
                nv12.as_mut_ptr().add(zeile * stride),
                ys.min(stride),
            );
        }
        let basis = stride * yh;
        for zeile in 0..(h as usize).div_ceil(2) {
            std::ptr::copy_nonoverlapping(
                uvp.add(zeile * uvs),
                nv12.as_mut_ptr().add(basis + zeile * stride),
                uvs.min(stride),
            );
        }
        if let Ok(mut z) = ziel.lock() {
            let mut raus = std::mem::take(&mut z.rgba);
            if nv12_to_rgba(&nv12, w, h, stride, yh as u32, &mut raus) {
                z.breite = w;
                z.hoehe = h;
                z.da = true;
            }
            z.rgba = raus;
        }
    }
    CVPixelBufferUnlockBaseAddress(bild, 1);
}

impl Decoder {
    pub fn new(w: u32, h: u32) -> Result<Self> {
        Ok(Decoder {
            session: std::ptr::null_mut(),
            format: std::ptr::null(),
            sps: Vec::new(),
            pps: Vec::new(),
            w,
            h,
            bild: Arc::new(Mutex::new(Bild::default())),
        })
    }

    /// Auf dem Mac gibt es nur einen Weg - die Unterscheidung existiert nur,
    /// damit die Aufrufer auf beiden Plattformen gleich aussehen.
    pub fn new_auto(w: u32, h: u32) -> Result<Self> {
        Decoder::new(w, h)
    }

    pub fn name(&self) -> &str {
        "VideoToolbox H.264"
    }

    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// Breite, Hoehe, Zeilenabstand - wie bei der Windows-Fassung.
    pub fn raw_size(&self) -> (u32, u32, usize) {
        (self.w, self.h, self.w as usize)
    }

    /// Sitzung anlegen, sobald SPS und PPS bekannt sind.
    fn sitzung_bauen(&mut self) -> Result<()> {
        if !self.session.is_null() || self.sps.is_empty() || self.pps.is_empty() {
            return Ok(());
        }
        unsafe {
            let zeiger: [*const u8; 2] = [self.sps.as_ptr(), self.pps.as_ptr()];
            let groessen: [usize; 2] = [self.sps.len(), self.pps.len()];
            let mut fd: CMFormatDescriptionRef = std::ptr::null();
            let st = CMVideoFormatDescriptionCreateFromH264ParameterSets(
                std::ptr::null(),
                2,
                zeiger.as_ptr(),
                groessen.as_ptr(),
                4,
                &mut fd,
            );
            if st != 0 || fd.is_null() {
                return Err(anyhow!("Formatbeschreibung: {}", st));
            }
            let rueckruf = VTDecompressionOutputCallbackRecord {
                callback: Some(dekodier_rueckruf),
                refcon: Arc::as_ptr(&self.bild) as *mut c_void,
            };
            let mut s: VTDecompressionSessionRef = std::ptr::null_mut();
            let st = VTDecompressionSessionCreate(
                std::ptr::null(),
                fd,
                std::ptr::null(),
                std::ptr::null(),
                &rueckruf,
                &mut s,
            );
            if st != 0 || s.is_null() {
                CFRelease(fd as CFTypeRef);
                return Err(anyhow!("Dekodierer nicht verfuegbar ({})", st));
            }
            self.format = fd;
            self.session = s;
        }
        Ok(())
    }

    /// Eine Zugriffseinheit dekodieren. Gibt das Bild als RGBA plus Groesse
    /// zurueck - oder `None`, wenn noch nichts Fertiges dabei war.
    pub fn decode(&mut self, au: &[u8], rgba: &mut Vec<u8>) -> Result<Option<(u32, u32)>> {
        // 1) NALs sortieren: Parametersaetze merken, Bilddaten sammeln.
        let mut avcc: Vec<u8> = Vec::with_capacity(au.len() + 16);
        for (off, len) in nal_zerlegen(au) {
            let nal = &au[off..off + len];
            match nal_typ(nal[0]) {
                7 => {
                    if self.sps != nal {
                        self.sps = nal.to_vec();
                        self.sitzung_schliessen();
                    }
                }
                8 => {
                    if self.pps != nal {
                        self.pps = nal.to_vec();
                        self.sitzung_schliessen();
                    }
                }
                // Fuellsel und Zugriffstrenner interessieren den Dekodierer nicht.
                9 | 12 => {}
                _ => {
                    avcc.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    avcc.extend_from_slice(nal);
                }
            }
        }
        self.sitzung_bauen()?;
        if self.session.is_null() || avcc.is_empty() {
            return Ok(None);
        }
        unsafe {
            let mut bb: CMBlockBufferRef = std::ptr::null_mut();
            let st = CMBlockBufferCreateWithMemoryBlock(
                std::ptr::null(),
                std::ptr::null_mut(),
                avcc.len(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                avcc.len(),
                0,
                &mut bb,
            );
            if st != 0 || bb.is_null() {
                return Err(anyhow!("Blockpuffer: {}", st));
            }
            let st = CMBlockBufferReplaceDataBytes(
                avcc.as_ptr() as *const c_void,
                bb,
                0,
                avcc.len(),
            );
            if st != 0 {
                CFRelease(bb as CFTypeRef);
                return Err(anyhow!("Blockpuffer fuellen: {}", st));
            }
            let groessen = [avcc.len()];
            let zeit = CMSampleTimingInfo {
                duration: CMTime::ungueltig(),
                presentation: CMTime::ungueltig(),
                decode: CMTime::ungueltig(),
            };
            let mut sb: CMSampleBufferRef = std::ptr::null_mut();
            let st = CMSampleBufferCreateReady(
                std::ptr::null(),
                bb,
                self.format,
                1,
                1,
                &zeit,
                1,
                groessen.as_ptr(),
                &mut sb,
            );
            CFRelease(bb as CFTypeRef);
            if st != 0 || sb.is_null() {
                return Err(anyhow!("Probenpuffer: {}", st));
            }
            let mut info = 0u32;
            let st = VTDecompressionSessionDecodeFrame(
                self.session,
                sb,
                kVTDecodeFrame_EnableTemporalProcessing,
                std::ptr::null_mut(),
                &mut info,
            );
            VTDecompressionSessionWaitForAsynchronousFrames(self.session);
            CFRelease(sb as CFTypeRef);
            if st != 0 {
                return Err(anyhow!("DecodeFrame: {}", st));
            }
        }
        let mut b = self
            .bild
            .lock()
            .map_err(|_| anyhow!("Dekodierer-Ausgabe blockiert"))?;
        if let Some(f) = b.fehler.take() {
            return Err(anyhow!(f));
        }
        if !b.da {
            return Ok(None);
        }
        b.da = false;
        self.w = b.breite;
        self.h = b.hoehe;
        rgba.clear();
        rgba.extend_from_slice(&b.rgba);
        Ok(Some((b.breite, b.hoehe)))
    }

    fn sitzung_schliessen(&mut self) {
        unsafe {
            if !self.session.is_null() {
                VTDecompressionSessionInvalidate(self.session);
                CFRelease(self.session as CFTypeRef);
                self.session = std::ptr::null_mut();
            }
            if !self.format.is_null() {
                CFRelease(self.format as CFTypeRef);
                self.format = std::ptr::null();
            }
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.sitzung_schliessen();
    }
}

/// Gibt es hier ueberhaupt einen H.264-Kodierer?
pub fn available() -> bool {
    Encoder::new(640, 480, 30, 2_000_000).is_ok()
}
