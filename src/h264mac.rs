//! H.264 on the Mac - VideoToolbox, in hardware.
//!
//! The same contract as the Windows version in `h264.rs`: NV12 in,
//! Annex-B out (and the reverse when decoding). This way meeting,
//! camera and screen share run on the Mac through exactly the same path as
//! on Windows - only the machine underneath is a different one.
//!
//! Deliberately WITHOUT an additional third-party library: the necessary Apple functions
//! are declared directly here. That saves four new dependencies including
//! their version and feature traps; the signatures have been
//! unchanged for years and appear like this in Apple's headers.

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
/// Allocate memory for the block buffer immediately (kCMBlockBufferAssureMemoryNowFlag).
const kCMBlockBufferAssureMemoryNowFlag: u32 = 1 << 0;

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
    fn invalid() -> CMTime {
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
    // SAFETY: function-pointer type only (no operation happens here). The signature must match
    // Apple's VTDecompressionOutputCallback exactly; VideoToolbox calls it with the `refcon` stored
    // next to it, which is a pointer to the decoder's `Mutex<Image>` kept alive by the `Decoder`
    // until the session is invalidated.
    callback: Option<
        unsafe extern "C" fn(*mut c_void, *mut c_void, OSStatus, u32, CVImageBufferRef, CMTime, CMTime),
    >,
    refcon: *mut c_void,
}

// SAFETY: function-pointer type only (no operation happens here). The signature must match Apple's
// VTCompressionOutputCallback; VideoToolbox passes back the `refcon` given to
// VTCompressionSessionCreate, a pointer to the encoder's `Mutex<Output>`.
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
        item_count: isize,
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
        width_px: usize,
        height_px: usize,
        format: OSType,
        attrs: CFDictionaryRef,
        out: *mut CVPixelBufferRef,
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
        length_here: *mut usize,
        length_total: *mut usize,
        daten: *mut *mut u8,
    ) -> OSStatus;
    fn CMBlockBufferCopyDataBytes(
        bb: CMBlockBufferRef,
        offset: usize,
        length: usize,
        destination: *mut c_void,
    ) -> OSStatus;
    fn CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
        fd: CMFormatDescriptionRef,
        index: usize,
        pointer: *mut *const u8,
        dimension: *mut usize,
        item_count: *mut usize,
        nal_head: *mut c_int,
    ) -> OSStatus;
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        alloc: CFAllocatorRef,
        item_count: usize,
        pointer: *const *const u8,
        sizes: *const usize,
        nal_head: c_int,
        out: *mut CMFormatDescriptionRef,
    ) -> OSStatus;
    fn CMBlockBufferCreateWithMemoryBlock(
        struct_alloc: CFAllocatorRef,
        speicher: *mut c_void,
        block_length: usize,
        block_alloc: CFAllocatorRef,
        origin: *const c_void,
        offset: usize,
        length: usize,
        flags: u32,
        out: *mut CMBlockBufferRef,
    ) -> OSStatus;
    fn CMBlockBufferReplaceDataBytes(
        origin: *const c_void,
        bb: CMBlockBufferRef,
        offset: usize,
        length: usize,
    ) -> OSStatus;
    fn CMSampleBufferCreateReady(
        alloc: CFAllocatorRef,
        daten: CMBlockBufferRef,
        format: CMFormatDescriptionRef,
        sample_count: isize,
        row_count: isize,
        times: *const CMSampleTimingInfo,
        size_count: isize,
        sizes: *const usize,
        out: *mut CMSampleBufferRef,
    ) -> OSStatus;
}

#[link(name = "VideoToolbox", kind = "framework")]
extern "C" {
    fn VTCompressionSessionCreate(
        alloc: CFAllocatorRef,
        width_px: i32,
        height_px: i32,
        codec: OSType,
        enc_spec: CFDictionaryRef,
        quell_attrs: CFDictionaryRef,
        daten_alloc: CFAllocatorRef,
        callback_fn: VTCompressionOutputCallback,
        refcon: *mut c_void,
        out: *mut VTCompressionSessionRef,
    ) -> OSStatus;
    fn VTCompressionSessionEncodeFrame(
        s: VTCompressionSessionRef,
        picture: CVImageBufferRef,
        pts: CMTime,
        dur: CMTime,
        frame_properties: CFDictionaryRef,
        quell_refcon: *mut c_void,
        info: *mut u32,
    ) -> OSStatus;
    fn VTCompressionSessionCompleteFrames(s: VTCompressionSessionRef, bis: CMTime) -> OSStatus;
    fn VTCompressionSessionInvalidate(s: VTCompressionSessionRef);
    fn VTDecompressionSessionCreate(
        alloc: CFAllocatorRef,
        format: CMFormatDescriptionRef,
        dec_spec: CFDictionaryRef,
        target_attrs: CFDictionaryRef,
        callback_fn: *const VTDecompressionOutputCallbackRecord,
        out: *mut VTDecompressionSessionRef,
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
    fn VTSessionSetProperty(s: VTSessionRef, prop_key: CFStringRef, wert: CFTypeRef) -> OSStatus;

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

fn number_i32(v: i32) -> CFNumberRef {
    // SAFETY: `&v` points to a live local i32 for the whole call and `kCFNumberSInt32Type` tells
    // CFNumberCreate to read exactly 4 bytes from it; the returned CFNumber is an owned (+1) object
    // that the caller must release.
    unsafe { CFNumberCreate(std::ptr::null(), kCFNumberSInt32Type, &v as *const i32 as *const c_void) }
}

fn number_f64(v: f64) -> CFNumberRef {
    // SAFETY: `&v` points to a live local f64 for the whole call and `kCFNumberFloat64Type` tells
    // CFNumberCreate to read exactly 8 bytes from it; the returned CFNumber is an owned (+1) object
    // that the caller must release.
    unsafe { CFNumberCreate(std::ptr::null(), kCFNumberFloat64Type, &v as *const f64 as *const c_void) }
}

/// A dictionary with exactly one entry - nobody needs more here.
fn dict1(prop_key: CFStringRef, wert: CFTypeRef) -> CFDictionaryRef {
    // SAFETY: `k` and `v` are local one-element arrays that outlive the call and `item_count` is 1,
    // matching their length; CFDictionaryCreate copies the entries. The key/value callbacks are the
    // CoreFoundation statics (only their addresses are taken), so the dictionary retains and
    // releases its entries correctly; `prop_key` and `wert` must be valid CF objects (the callers
    // pass Apple's constants). The result is an owned (+1) dictionary.
    unsafe {
        let k = [prop_key as *const c_void];
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

/// Splits an Annex-B stream into its NAL units (without start codes).
fn nal_split(au: &[u8]) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let mut i = 0usize;
    let mut start: Option<usize> = None;
    while i + 2 < au.len() {
        let drei = au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 1;
        let vier = i + 3 < au.len() && au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 0 && au[i + 3] == 1;
        if drei || vier {
            let marke = if vier { 4 } else { 3 };
            if let Some(s) = start.take() {
                if i > s {
                    parts.push((s, i - s));
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
            parts.push((s, au.len() - s));
        }
    }
    parts
}

fn nal_typ(b: u8) -> u8 {
    b & 0x1f
}

// -------------------------------------------------------------- Kodierer ---

/// What the encoder's callback collects.
#[derive(Default)]
struct Output {
    done: Vec<Chunk>,
    error: Option<String>,
}

pub struct Encoder {
    session: VTCompressionSessionRef,
    pixel: CVPixelBufferRef,
    w: u32,
    h: u32,
    fps: u32,
    ctr: i64,
    request_key: bool,
    output: Arc<Mutex<Output>>,
}

// The session itself is thread-safe from Apple's side; the rest sits behind
// a mutex. Without that the encoder could not be put into the meeting's
// worker thread.
// SAFETY: the VTCompressionSession is documented as thread-safe, so moving it to another thread is
// fine. The other raw pointer, `pixel`, is only touched through `&mut self` (encode/drop), and the
// data shared with the VideoToolbox callback thread is the `Arc<Mutex<Output>>`. The struct is not
// `Sync`, so there is never concurrent access from two threads.
unsafe impl Send for Encoder {}

// SAFETY: callback invoked by VideoToolbox on its own thread. `refcon` is the pointer to the
// `Mutex<Output>` inside the Encoder's Arc (null is checked); the session is invalidated in Drop
// before that Arc is released, so the reference is valid whenever the callback runs. `probe` is a
// live CMSampleBuffer owned by VideoToolbox for the duration of the call (null checked); `bb`, `fd`
// and the parameter-set pointers belong to it and are only used inside the call. The slice over
// `data_ptr` is only valid for `here_len` bytes (contiguous length at offset 0). If the block
// buffer is not contiguous (`here_len < total_len`), the whole payload is copied into an owned Vec
// with CMBlockBufferCopyDataBytes instead of slicing `total_len` bytes from the pointer. The AVCC
// parser bounds-checks every length against the slice. Parameter-set pointers are checked for
// null and non-zero size before they are turned into slices.
unsafe extern "C" fn encode_callback(
    refcon: *mut c_void,
    _source: *mut c_void,
    status: OSStatus,
    _info: u32,
    probe: CMSampleBufferRef,
) {
    if refcon.is_null() {
        return;
    }
    let output = &*(refcon as *const Mutex<Output>);
    if status != 0 || probe.is_null() {
        if let Ok(mut a) = output.lock() {
            a.error = Some(format!("Kodierer meldet {}", status));
        }
        return;
    }
    // 1) The payload is in AVCC form: a length in front of each NAL.
    let bb = CMSampleBufferGetDataBuffer(probe);
    if bb.is_null() {
        return;
    }
    let (mut here_len, mut total_len, mut data_ptr) = (0usize, 0usize, std::ptr::null_mut::<u8>());
    if CMBlockBufferGetDataPointer(bb, 0, &mut here_len, &mut total_len, &mut data_ptr) != 0
        || data_ptr.is_null()
    {
        return;
    }
    // Only `here_len` bytes behind `data_ptr` are guaranteed contiguous; never slice more than that.
    let mut copy: Vec<u8> = Vec::new();
    let avcc: &[u8] = if here_len >= total_len {
        std::slice::from_raw_parts(data_ptr, total_len)
    } else {
        copy.resize(total_len, 0);
        if CMBlockBufferCopyDataBytes(bb, 0, total_len, copy.as_mut_ptr() as *mut c_void) != 0 {
            return;
        }
        &copy
    };

    // 2) Rewrite to Annex-B (start code instead of length) - that is the
    //    format the rest of the program and str0m expect.
    let mut annexb: Vec<u8> = Vec::with_capacity(total_len + 64);
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

    // 3) SPS and PPS must precede every keyframe, otherwise
    //    nobody who joins later can get in.
    if idr {
        let fd = CMSampleBufferGetFormatDescription(probe);
        if !fd.is_null() {
            let mut head: Vec<u8> = Vec::new();
            let mut item_count = 0usize;
            let (mut p, mut dimension, mut nal_len) = (std::ptr::null::<u8>(), 0usize, 0 as c_int);
            if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                fd,
                0,
                &mut p,
                &mut dimension,
                &mut item_count,
                &mut nal_len,
            ) == 0
            {
                for idx in 0..item_count {
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
                        head.extend_from_slice(&[0, 0, 0, 1]);
                        head.extend_from_slice(std::slice::from_raw_parts(pp, gg));
                    }
                }
            }
            if !head.is_empty() {
                head.extend_from_slice(&annexb);
                annexb = head;
            }
        }
    }
    if let Ok(mut a) = output.lock() {
        a.done.push(Chunk {
            data: annexb,
            key: idr,
        });
    }
}

impl Encoder {
    pub fn new(w: u32, h: u32, fps: u32, bitrate: u32) -> Result<Self> {
        if w < 16 || h < 16 {
            return Err(anyhow!("frame too small for H.264"));
        }
        // Gerade Kantenlaengen - alles andere mag kein 4:2:0.
        let (w, h) = (w & !1, h & !1);
        let output = Arc::new(Mutex::new(Output::default()));
        let refcon = Arc::as_ptr(&output) as *mut c_void;
        let mut session: VTCompressionSessionRef = std::ptr::null_mut();
        // SAFETY: all pointer arguments are valid for the call: the null allocator/dictionaries
        // select the defaults, `refcon` is the address of the `Mutex<Output>` inside `output`
        // (the Arc stored in the Encoder, which outlives the session because Drop invalidates the
        // session first), and `session` is a local out-slot that is checked for null afterwards.
        // `encode_callback` matches the callback signature.
        let st = unsafe {
            VTCompressionSessionCreate(
                std::ptr::null(),
                w as i32,
                h as i32,
                kCMVideoCodecType_H264,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                Some(encode_callback),
                refcon,
                &mut session,
            )
        };
        if st != 0 || session.is_null() {
            return Err(anyhow!("VideoToolbox-Kodierer nicht verfuegbar ({})", st));
        }
        // SAFETY: `session` was just created successfully (non-null, checked above) and is
        // exclusively ours. Property keys/values are Apple constants or CFNumbers created here;
        // values created here (`release == true`) are released exactly once right after the call
        // (VTSessionSetProperty retains what it needs). The kCFBoolean statics are only read.
        unsafe {
            let setzen = |k: CFStringRef, v: CFTypeRef, release: bool| {
                VTSessionSetProperty(session, k, v);
                if release && !v.is_null() {
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
                number_i32(bitrate.max(200_000) as i32),
                true,
            );
            setzen(
                kVTCompressionPropertyKey_ExpectedFrameRate,
                number_f64(fps.max(1) as f64),
                true,
            );
            // A keyframe every two seconds - in between every
            // newcomer gets one via the request.
            setzen(
                kVTCompressionPropertyKey_MaxKeyFrameInterval,
                number_i32((fps.max(1) * 2) as i32),
                true,
            );
        }
        // A single pixel buffer that is filled again and again.
        let mut pixel: CVPixelBufferRef = std::ptr::null_mut();
        // SAFETY: `pixel` is a local out-slot that is checked for non-null together with the
        // status; null attribute/allocator arguments select defaults. The width/height are even and
        // at least 16 (checked in `new`).
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
            // SAFETY: `session` is a valid compression session created above that is not stored
            // anywhere else yet; it is invalidated and released exactly once on this error path,
            // and `Encoder` is never constructed, so Drop does not run for it.
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
            ctr: 0,
            request_key: true,
            output,
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
        self.request_key = true;
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
        // SAFETY: `self.pixel` and `self.session` are live objects owned by this Encoder (created
        // in `new`, released only in Drop). Lock/Unlock are paired; between them no early return
        // exists. Source reads stay inside `nv12`: its length was checked against w*h*3/2 and the Y
        // rows read w bytes at `text_row * w` (zeile < h) and the UV rows w bytes at `w*h + text_row*w`
        // (zeile < h/2). Destination writes use the plane base address and the row stride reported
        // by CoreVideo (>= w for both planes of a 4:2:0 buffer with width w); the planes have h and
        // h/2 rows, so each row write is inside the plane. Null plane pointers are skipped. `props`
        // is an owned dictionary that is released once after EncodeFrame (VT retains what it
        // needs), and CompleteFrames blocks until the callback has run for every frame, so `pixel`
        // is no longer read by VideoToolbox when it is rewritten next time.
        unsafe {
            if CVPixelBufferLockBaseAddress(self.pixel, 0) != 0 {
                return Err(anyhow!("pixel buffer cannot be locked"));
            }
            // Y-Ebene
            let y = CVPixelBufferGetBaseAddressOfPlane(self.pixel, 0) as *mut u8;
            let ys = CVPixelBufferGetBytesPerRowOfPlane(self.pixel, 0);
            if !y.is_null() {
                for text_row in 0..h {
                    std::ptr::copy_nonoverlapping(
                        nv12.as_ptr().add(text_row * w),
                        y.add(text_row * ys),
                        w,
                    );
                }
            }
            // UV-Ebene (halbe Hoehe, gleiche Width)
            let uv = CVPixelBufferGetBaseAddressOfPlane(self.pixel, 1) as *mut u8;
            let uvs = CVPixelBufferGetBytesPerRowOfPlane(self.pixel, 1);
            if !uv.is_null() {
                for text_row in 0..h / 2 {
                    std::ptr::copy_nonoverlapping(
                        nv12.as_ptr().add(w * h + text_row * w),
                        uv.add(text_row * uvs),
                        w,
                    );
                }
            }
            CVPixelBufferUnlockBaseAddress(self.pixel, 0);

            let pts = CMTime::neu(self.ctr, self.fps as i32);
            let dur = CMTime::neu(1, self.fps as i32);
            self.ctr += 1;
            let props = if self.request_key {
                self.request_key = false;
                dict1(kVTEncodeFrameOptionKey_ForceKeyFrame, kCFBooleanTrue)
            } else {
                std::ptr::null()
            };
            let mut info = 0u32;
            let st = VTCompressionSessionEncodeFrame(
                self.session,
                self.pixel,
                pts,
                dur,
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
            // Wait for the result: otherwise the callback runs at some point
            // later, and we want to send the frame NOW.
            VTCompressionSessionCompleteFrames(self.session, CMTime::invalid());
        }
        let mut a = self
            .output
            .lock()
            .map_err(|_| anyhow!("Kodierer-Ausgabe blockiert"))?;
        if let Some(f) = a.error.take() {
            return Err(anyhow!(f));
        }
        Ok(std::mem::take(&mut a.done))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: Drop runs exactly once and nothing uses the Encoder afterwards. CompleteFrames
        // flushes pending frames, then the session is invalidated (no callback can run afterwards,
        // so the `refcon` into the Arc can no longer dangle) and the session and pixel buffer are
        // each released once; `pixel` is checked for null.
        unsafe {
            VTCompressionSessionCompleteFrames(self.session, CMTime::invalid());
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
struct Image {
    width_px: u32,
    height_px: u32,
    rgba: Vec<u8>,
    da: bool,
    error: Option<String>,
}

pub struct Decoder {
    session: VTDecompressionSessionRef,
    format: CMFormatDescriptionRef,
    sps: Vec<u8>,
    pps: Vec<u8>,
    w: u32,
    h: u32,
    picture: Arc<Mutex<Image>>,
}

// SAFETY: the VTDecompressionSession is documented as thread-safe, so moving it to another thread
// is fine. `format` is an immutable CoreFoundation object, only changed through `&mut self`. The
// data shared with the VideoToolbox callback thread is the `Arc<Mutex<Image>>`. The struct is not
// `Sync`, so there is never concurrent access from two threads.
unsafe impl Send for Decoder {}

// SAFETY: callback invoked by VideoToolbox, possibly on another thread. `refcon` is the pointer to
// the `Mutex<Image>` inside the Decoder's Arc (null is checked); the session is invalidated before
// the Decoder drops, so the reference is valid whenever the callback runs. `picture` is a live
// CVPixelBuffer for the call (null and pixel format are checked) and is locked read-only (flag 1)
// with a matching unlock at the end; there is no early return in between. Plane pointers are
// checked for null, and the copies stay in bounds: source rows are < h (Y plane) and < ceil(h/2)
// (UV plane), each at most `ys`/`uvs` bytes, and the destination `nv12` has stride*yh +
// stride*ceil(h/2) bytes with stride >= ys, uvs, so every destination row fits.
unsafe extern "C" fn decode_callback(
    refcon: *mut c_void,
    _source: *mut c_void,
    status: OSStatus,
    _info: u32,
    picture: CVImageBufferRef,
    _pts: CMTime,
    _duration: CMTime,
) {
    if refcon.is_null() {
        return;
    }
    let dest = &*(refcon as *const Mutex<Image>);
    if status != 0 || picture.is_null() {
        if let Ok(mut z) = dest.lock() {
            if status != 0 {
                z.error = Some(format!("Dekodierer meldet {}", status));
            }
        }
        return;
    }
    let fmt = CVPixelBufferGetPixelFormatType(picture);
    if fmt != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        && fmt != kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
    {
        if let Ok(mut z) = dest.lock() {
            z.error = Some(format!("unerwartetes Pixelformat {:x}", fmt));
        }
        return;
    }
    if CVPixelBufferLockBaseAddress(picture, 1) != 0 {
        return;
    }
    let w = CVPixelBufferGetWidth(picture) as u32;
    let h = CVPixelBufferGetHeight(picture) as u32;
    let yp = CVPixelBufferGetBaseAddressOfPlane(picture, 0) as *const u8;
    let ys = CVPixelBufferGetBytesPerRowOfPlane(picture, 0);
    let uvp = CVPixelBufferGetBaseAddressOfPlane(picture, 1) as *const u8;
    let uvs = CVPixelBufferGetBytesPerRowOfPlane(picture, 1);
    let yh = CVPixelBufferGetHeightOfPlane(picture, 0);
    let _uvw = CVPixelBufferGetWidthOfPlane(picture, 1);
    if !yp.is_null() && !uvp.is_null() && w > 0 && h > 0 {
        // The two planes do NOT necessarily lie one after the other - hence
        // first copy into a contiguous NV12 block and use the
        // shared converter.
        let stride = ys.max(uvs);
        let mut nv12 = vec![0u8; stride * yh + stride * (h as usize).div_ceil(2)];
        for text_row in 0..h as usize {
            std::ptr::copy_nonoverlapping(
                yp.add(text_row * ys),
                nv12.as_mut_ptr().add(text_row * stride),
                ys.min(stride),
            );
        }
        let basis = stride * yh;
        for text_row in 0..(h as usize).div_ceil(2) {
            std::ptr::copy_nonoverlapping(
                uvp.add(text_row * uvs),
                nv12.as_mut_ptr().add(basis + text_row * stride),
                uvs.min(stride),
            );
        }
        if let Ok(mut z) = dest.lock() {
            let mut out = std::mem::take(&mut z.rgba);
            if nv12_to_rgba(&nv12, w, h, stride, yh as u32, &mut out) {
                z.width_px = w;
                z.height_px = h;
                z.da = true;
            }
            z.rgba = out;
        }
    }
    CVPixelBufferUnlockBaseAddress(picture, 1);
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
            picture: Arc::new(Mutex::new(Image::default())),
        })
    }

    /// On the Mac there is only one path - the distinction exists only
    /// so that the callers look the same on both platforms.
    pub fn new_auto(w: u32, h: u32) -> Result<Self> {
        Decoder::new(w, h)
    }

    pub fn name(&self) -> &str {
        "VideoToolbox H.264"
    }

    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// Width, height, row stride - as in the Windows version.
    pub fn raw_size(&self) -> (u32, u32, usize) {
        (self.w, self.h, self.w as usize)
    }

    /// Create a session as soon as SPS and PPS are known.
    fn build_session(&mut self) -> Result<()> {
        if !self.session.is_null() || self.sps.is_empty() || self.pps.is_empty() {
            return Ok(());
        }
        // SAFETY: `pointer`/`sizes` point into `self.sps`/`self.pps` and local arrays that are
        // alive for the call (the SPS/PPS are copied by CoreMedia into the format description).
        // `fd` is a local out-slot that is checked for null. `callback_fn.refcon` is the address of
        // the `Mutex<Image>` in `self.picture`, which outlives the session because `close_session`
        // invalidates the session before the Arc is dropped. On failure `fd` is released once; on
        // success ownership of `fd` and `s` moves into `self`.
        unsafe {
            let pointer: [*const u8; 2] = [self.sps.as_ptr(), self.pps.as_ptr()];
            let sizes: [usize; 2] = [self.sps.len(), self.pps.len()];
            let mut fd: CMFormatDescriptionRef = std::ptr::null();
            let st = CMVideoFormatDescriptionCreateFromH264ParameterSets(
                std::ptr::null(),
                2,
                pointer.as_ptr(),
                sizes.as_ptr(),
                4,
                &mut fd,
            );
            if st != 0 || fd.is_null() {
                return Err(anyhow!("Formatbeschreibung: {}", st));
            }
            let callback_fn = VTDecompressionOutputCallbackRecord {
                callback: Some(decode_callback),
                refcon: Arc::as_ptr(&self.picture) as *mut c_void,
            };
            let mut s: VTDecompressionSessionRef = std::ptr::null_mut();
            let st = VTDecompressionSessionCreate(
                std::ptr::null(),
                fd,
                std::ptr::null(),
                std::ptr::null(),
                &callback_fn,
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

    /// Decode one access unit. Returns the image as RGBA plus size
    /// - or `None` if nothing finished was included yet.
    pub fn decode(&mut self, au: &[u8], rgba: &mut Vec<u8>) -> Result<Option<(u32, u32)>> {
        // 1) NALs sortieren: Parametersaetze merken, Bilddaten sammeln.
        let mut avcc: Vec<u8> = Vec::with_capacity(au.len() + 16);
        for (off, len) in nal_split(au) {
            let nal = &au[off..off + len];
            match nal_typ(nal[0]) {
                7 => {
                    if self.sps != nal {
                        self.sps = nal.to_vec();
                        self.close_session();
                    }
                }
                8 => {
                    if self.pps != nal {
                        self.pps = nal.to_vec();
                        self.close_session();
                    }
                }
                // Filler and access unit delimiters do not interest the decoder.
                9 | 12 => {}
                _ => {
                    avcc.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    avcc.extend_from_slice(nal);
                }
            }
        }
        self.build_session()?;
        if self.session.is_null() || avcc.is_empty() {
            return Ok(None);
        }
        // SAFETY: the block buffer is created with `avcc.len()` bytes allocated by CoreMedia (null
        // memory block + AssureMemoryNow flag) and filled with exactly `avcc.len()` bytes from the
        // live Vec, so the copy is in bounds. `sizes` and `zeit` are locals that outlive
        // CMSampleBufferCreateReady (arrays of length 1, as the counts say); `self.format` is
        // non-null because `self.session` is non-null (both are set together). The block buffer is
        // released once (the sample buffer retains it), the sample buffer once after the
        // synchronous decode (WaitForAsynchronousFrames), and every error path releases what was
        // created before it.
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
                kCMBlockBufferAssureMemoryNowFlag,
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
            let sizes = [avcc.len()];
            let zeit = CMSampleTimingInfo {
                duration: CMTime::invalid(),
                presentation: CMTime::invalid(),
                decode: CMTime::invalid(),
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
                sizes.as_ptr(),
                &mut sb,
            );
            CFRelease(bb as CFTypeRef);
            if st != 0 || sb.is_null() {
                return Err(anyhow!("Probenpuffer: {}", st));
            }
            let mut info = 0u32;
            // Flags = 0: decode synchronously and OUTPUT the image. (1<<1
            // would be kVTDecodeFrame_DoNotOutputFrame - with that no image
            // ever came out on the first attempt.)
            let st = VTDecompressionSessionDecodeFrame(
                self.session,
                sb,
                0,
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
            .picture
            .lock()
            .map_err(|_| anyhow!("Dekodierer-Ausgabe blockiert"))?;
        if let Some(f) = b.error.take() {
            return Err(anyhow!(f));
        }
        if !b.da {
            return Ok(None);
        }
        b.da = false;
        self.w = b.width_px;
        self.h = b.height_px;
        rgba.clear();
        rgba.extend_from_slice(&b.rgba);
        Ok(Some((b.width_px, b.height_px)))
    }

    fn close_session(&mut self) {
        // SAFETY: the session and format description are owned by this struct and each is released
        // at most once: both are reset to null right after, so a repeated call (or the one from
        // Drop) is a no-op. Invalidate stops the output callback before the `Arc<Mutex<Image>>` that
        // `refcon` points to can be dropped.
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
        self.close_session();
    }
}

/// Is there an H.264 encoder here at all?
pub fn available() -> bool {
    Encoder::new(640, 480, 30, 2_000_000).is_ok()
}
