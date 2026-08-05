//! Hardware H.264 video codec through Media Foundation.
//!
//! The JPEG tile path sends every changed rectangle as an independent picture.
//! That is robust and simple, but it cannot exploit the fact that consecutive
//! frames are almost identical - a video codec can. On this machine the
//! encoding runs on the GPU (NVENC/QuickSync/AMF are all exposed as Media
//! Foundation Transforms), so the CPU only moves the finished bitstream.
//!
//! Layout of the pipeline:
//!
//! ```text
//! host:    desktop -> (D3D11 video processor) -> NV12 -> H264 MFT -> bytes
//! viewer:  bytes -> H264 decoder MFT -> NV12 -> RGBA -> egui texture
//! ```
//!
//! Colour: everything uses BT.601 with studio range (16..235), which is what
//! every H.264 encoder assumes by default. Encoder side and decoder side of
//! this file use the exact same matrix, so a round trip is loss free apart
//! from the codec itself.

#[allow(unused_imports)]
use anyhow::{anyhow, Result};

/// One encoded access unit.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub data: Vec<u8>,
    pub key: bool,
}

// ------------------------------------------------------------- colour ------

/// Packed RGB -> NV12 (BT.601, studio range). Used when no GPU scaler is
/// available; the fast path lets the D3D11 video processor do this.
pub fn rgb_to_nv12(rgb: &[u8], w: u32, h: u32, out: &mut Vec<u8>) {
    let (w, h) = (w as usize, h as usize);
    let ysize = w * h;
    out.clear();
    out.resize(ysize + ysize / 2, 128);
    if rgb.len() < ysize * 3 {
        return;
    }
    // luma
    for y in 0..h {
        for x in 0..w {
            let s = (y * w + x) * 3;
            let (r, g, b) = (rgb[s] as i32, rgb[s + 1] as i32, rgb[s + 2] as i32);
            let l = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
            out[y * w + x] = l.clamp(0, 255) as u8;
        }
    }
    // chroma, 2x2 averaged
    let cw = w / 2;
    let ch = h / 2;
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut r, mut g, mut b) = (0i32, 0i32, 0i32);
            for dy in 0..2 {
                for dx in 0..2 {
                    let s = ((cy * 2 + dy) * w + cx * 2 + dx) * 3;
                    r += rgb[s] as i32;
                    g += rgb[s + 1] as i32;
                    b += rgb[s + 2] as i32;
                }
            }
            let (r, g, b) = (r / 4, g / 4, b / 4);
            let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
            let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
            let o = ysize + cy * w + cx * 2;
            out[o] = u.clamp(0, 255) as u8;
            out[o + 1] = v.clamp(0, 255) as u8;
        }
    }
}

/// NV12 -> RGBA (BT.601, studio range).
///
/// `stride` is the byte pitch of the Y plane and `plane_h` the number of Y
/// rows actually stored in the buffer - H.264 decoders pad the picture up to
/// a multiple of 16 (1080 becomes 1088), so the UV plane does NOT start at
/// `stride * h` but behind the padded luma plane. `w`/`h` are the visible
/// pixels that end up in `out`.
pub fn nv12_to_rgba(
    nv12: &[u8],
    w: u32,
    h: u32,
    stride: usize,
    plane_h: u32,
    out: &mut Vec<u8>,
) -> bool {
    let (wu, hu) = (w as usize, h as usize);
    let ph = (plane_h as usize).max(hu);
    if stride < wu || nv12.len() < stride * ph + stride * (hu.div_ceil(2)) {
        return false;
    }
    out.clear();
    out.resize(wu * hu * 4, 255);
    let uv_base = stride * ph;
    for y in 0..hu {
        let yrow = y * stride;
        let uvrow = uv_base + (y / 2) * stride;
        let orow = y * wu * 4;
        for x in 0..wu {
            let c = nv12[yrow + x] as i32 - 16;
            let uv = uvrow + (x & !1);
            let d = nv12[uv] as i32 - 128;
            let e = nv12[uv + 1] as i32 - 128;
            let r = (298 * c + 409 * e + 128) >> 8;
            let g = (298 * c - 100 * d - 208 * e + 128) >> 8;
            let b = (298 * c + 516 * d + 128) >> 8;
            let o = orow + x * 4;
            out[o] = r.clamp(0, 255) as u8;
            out[o + 1] = g.clamp(0, 255) as u8;
            out[o + 2] = b.clamp(0, 255) as u8;
            out[o + 3] = 255;
        }
    }
    true
}

/// Mean absolute error per channel between a RGB and a RGBA picture.
/// Used by the self test to prove the round trip actually works.
pub fn rgb_vs_rgba_error(rgb: &[u8], rgba: &[u8], w: u32, h: u32) -> (f64, u32) {
    let n = (w * h) as usize;
    if rgb.len() < n * 3 || rgba.len() < n * 4 {
        return (255.0, 255);
    }
    let mut sum = 0f64;
    let mut max = 0u32;
    for i in 0..n {
        for c in 0..3 {
            let d = (rgb[i * 3 + c] as i32 - rgba[i * 4 + c] as i32).unsigned_abs();
            sum += d as f64;
            max = max.max(d);
        }
    }
    (sum / (n * 3) as f64, max)
}

// ------------------------------------------------------------- windows -----

#[cfg(windows)]
mod win {
    use super::{Chunk, Result};
    use anyhow::anyhow;
    use std::sync::Once;
    use windows::core::{Interface, GUID, PWSTR, VARIANT};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::CoTaskMemFree;

    static MF_INIT: Once = Once::new();

    fn mf_startup() {
        MF_INIT.call_once(|| unsafe {
            let _ = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
        });
    }

    /// MF_MT_FRAME_SIZE and friends pack two 32 bit values into one u64.
    fn pack(a: u32, b: u32) -> u64 {
        ((a as u64) << 32) | b as u64
    }

    fn type_info(major: GUID, sub: GUID) -> MFT_REGISTER_TYPE_INFO {
        MFT_REGISTER_TYPE_INFO {
            guidMajorType: major,
            guidSubtype: sub,
        }
    }

    fn activate_name(a: &IMFActivate) -> String {
        unsafe {
            let mut p = PWSTR::null();
            let mut len = 0u32;
            if a.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut p, &mut len)
                .is_err()
            {
                return "MFT".to_string();
            }
            let s = p.to_string().unwrap_or_else(|_| "MFT".to_string());
            CoTaskMemFree(Some(p.0 as *const _));
            s
        }
    }

    /// Enumerates transforms and hands every candidate to `pick` until one is
    /// accepted. The COM references of the rejected ones are released here.
    fn find_mft<T>(
        category: GUID,
        flags: MFT_ENUM_FLAG,
        input: MFT_REGISTER_TYPE_INFO,
        output: MFT_REGISTER_TYPE_INFO,
        mut pick: impl FnMut(&IMFActivate, &str) -> Option<T>,
    ) -> Option<T> {
        unsafe {
            let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count = 0u32;
            if MFTEnumEx(
                category,
                flags,
                Some(&input),
                Some(&output),
                &mut list,
                &mut count,
            )
            .is_err()
                || list.is_null()
            {
                return None;
            }
            let mut found = None;
            for i in 0..count as usize {
                if found.is_none() {
                    if let Some(a) = (*list.add(i)).as_ref() {
                        let name = activate_name(a);
                        found = pick(a, &name);
                    }
                }
            }
            for i in 0..count as usize {
                std::ptr::drop_in_place(list.add(i));
            }
            CoTaskMemFree(Some(list as *const _));
            found
        }
    }

    fn set_codec_api(t: &IMFTransform, bitrate: u32, gop: u32) {
        unsafe {
            let api: ICodecAPI = match t.cast() {
                Ok(a) => a,
                Err(_) => return,
            };
            // real time, low latency, no B frames, bitrate driven
            let _ = api.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
            let _ = api.SetValue(&CODECAPI_AVEncCommonRealTime, &VARIANT::from(true));
            let _ = api.SetValue(
                &CODECAPI_AVEncCommonRateControlMode,
                &VARIANT::from(eAVEncCommonRateControlMode_UnconstrainedVBR.0 as u32),
            );
            let _ = api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &VARIANT::from(bitrate));
            let _ = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &VARIANT::from(gop));
        }
    }

    fn force_key(t: &IMFTransform) {
        unsafe {
            if let Ok(api) = t.cast::<ICodecAPI>() {
                let _ = api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(1u32));
            }
        }
    }

    /// Copies an IMFSample into a plain byte vector.
    unsafe fn sample_bytes(s: &IMFSample) -> Result<Vec<u8>> {
        let buf = s.ConvertToContiguousBuffer()?;
        let mut p: *mut u8 = std::ptr::null_mut();
        let mut len = 0u32;
        buf.Lock(&mut p, None, Some(&mut len))?;
        let out = std::slice::from_raw_parts(p, len as usize).to_vec();
        let _ = buf.Unlock();
        Ok(out)
    }

    unsafe fn sample_from_bytes(data: &[u8], time: i64, dur: i64) -> Result<IMFSample> {
        let buf = MFCreateMemoryBuffer(data.len() as u32)?;
        let mut p: *mut u8 = std::ptr::null_mut();
        buf.Lock(&mut p, None, None)?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        buf.Unlock()?;
        buf.SetCurrentLength(data.len() as u32)?;
        let s = MFCreateSample()?;
        s.AddBuffer(&buf)?;
        s.SetSampleTime(time)?;
        s.SetSampleDuration(dur)?;
        Ok(s)
    }

    // ------------------------------------------------------------ encoder --

    pub struct Encoder {
        t: IMFTransform,
        events: Option<IMFMediaEventGenerator>,
        provides_samples: bool,
        out_size: u32,
        w: u32,
        h: u32,
        fps: u32,
        name: String,
        hardware: bool,
        /// How many "send me a frame" tokens the async MFT still owes us.
        need: i32,
        frames: u64,
    }

    impl Encoder {
        pub fn new(w: u32, h: u32, fps: u32, bitrate: u32) -> Result<Self> {
            mf_startup();
            let w = w & !1;
            let h = h & !1;
            if w < 32 || h < 32 {
                return Err(anyhow!("Aufloesung zu klein fuer H.264"));
            }
            let input = type_info(MFMediaType_Video, MFVideoFormat_NV12);
            let output = type_info(MFMediaType_Video, MFVideoFormat_H264);

            // hardware first, software encoder only as a fallback
            let mut enc: Option<Self> = None;
            for (flags, hardware) in [
                (
                    MFT_ENUM_FLAG(
                        MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0,
                    ),
                    true,
                ),
                (
                    MFT_ENUM_FLAG(MFT_ENUM_FLAG_SYNCMFT.0 | MFT_ENUM_FLAG_SORTANDFILTER.0),
                    false,
                ),
            ] {
                if enc.is_some() {
                    break;
                }
                enc = find_mft(
                    MFT_CATEGORY_VIDEO_ENCODER,
                    flags,
                    input,
                    output,
                    |a, name| match Self::build(a, name, hardware, w, h, fps, bitrate) {
                        Ok(e) => Some(e),
                        Err(e) => {
                            crate::capture::log_line(&format!("h264 {} faellt aus: {}", name, e));
                            None
                        }
                    },
                );
            }
            enc.ok_or_else(|| anyhow!("kein H.264 Encoder gefunden"))
        }

        fn build(
            a: &IMFActivate,
            name: &str,
            hardware: bool,
            w: u32,
            h: u32,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self> {
            unsafe {
                let t: IMFTransform = a.ActivateObject()?;
                let mut events = None;
                if let Ok(attrs) = t.GetAttributes() {
                    let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
                    if attrs.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) == 1 {
                        attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
                        events = t.cast::<IMFMediaEventGenerator>().ok();
                    }
                }
                set_codec_api(&t, bitrate, fps * 4);

                // output first, that is what the MFT documentation demands
                let out = MFCreateMediaType()?;
                out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
                out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
                out.SetUINT64(&MF_MT_FRAME_SIZE, pack(w, h))?;
                out.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
                out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
                out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Main.0 as u32)?;
                out.SetUINT32(&MF_MT_MAX_KEYFRAME_SPACING, fps * 4)?;
                t.SetOutputType(0, &out, 0)
                    .map_err(|e| anyhow!("SetOutputType: {}", e))?;

                let inp = MFCreateMediaType()?;
                inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
                inp.SetUINT64(&MF_MT_FRAME_SIZE, pack(w, h))?;
                inp.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
                inp.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
                inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                t.SetInputType(0, &inp, 0)
                    .map_err(|e| anyhow!("SetInputType: {}", e))?;

                let info = t.GetOutputStreamInfo(0)?;
                let provides = info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
                let out_size = info.cbSize.max(w * h);

                t.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0)?;
                t.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
                t.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;

                crate::capture::log_line(&format!(
                    "h264 encoder: {} ({}, {}x{} @{} fps, {} kbit/s){}",
                    name,
                    if hardware { "GPU" } else { "CPU" },
                    w,
                    h,
                    fps,
                    bitrate / 1000,
                    if events.is_some() { ", async" } else { "" }
                ));

                Ok(Self {
                    t,
                    events,
                    provides_samples: provides,
                    out_size,
                    w,
                    h,
                    fps,
                    name: name.to_string(),
                    hardware,
                    need: 0,
                    frames: 0,
                })
            }
        }

        pub fn name(&self) -> &str {
            &self.name
        }
        pub fn hardware(&self) -> bool {
            self.hardware
        }
        pub fn size(&self) -> (u32, u32) {
            (self.w, self.h)
        }
        pub fn nv12_len(&self) -> usize {
            self.w as usize * self.h as usize * 3 / 2
        }

        /// Requests an IDR frame - used when a viewer joins or reports a gap.
        pub fn request_keyframe(&mut self) {
            force_key(&self.t);
        }

        /// Drains everything the async MFT has to say. Returns the encoded
        /// chunks that were ready.
        unsafe fn drain(&mut self, blocking: bool, out: &mut Vec<Chunk>) -> Result<()> {
            let ev = match self.events.clone() {
                Some(e) => e,
                None => return Ok(()),
            };
            loop {
                let flags = if blocking && self.need <= 0 {
                    MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)
                } else {
                    MF_EVENT_FLAG_NO_WAIT
                };
                let e = match ev.GetEvent(flags) {
                    Ok(e) => e,
                    Err(err) if err.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(()),
                    Err(err) => return Err(anyhow!("GetEvent: {}", err)),
                };
                match MF_EVENT_TYPE(e.GetType()? as i32) {
                    METransformNeedInput => {
                        self.need += 1;
                        if blocking {
                            return Ok(());
                        }
                    }
                    METransformHaveOutput => {
                        if let Some(c) = self.pull(out)? {
                            let _ = c;
                        }
                    }
                    _ => {}
                }
            }
        }

        /// One ProcessOutput round.
        unsafe fn pull(&mut self, out: &mut Vec<Chunk>) -> Result<Option<()>> {
            let sample = if self.provides_samples {
                None
            } else {
                let buf = MFCreateMemoryBuffer(self.out_size)?;
                let s = MFCreateSample()?;
                s.AddBuffer(&buf)?;
                Some(s)
            };
            let mut db = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: std::mem::ManuallyDrop::new(sample),
                dwStatus: 0,
                pEvents: std::mem::ManuallyDrop::new(None),
            };
            let mut status = 0u32;
            let res = self
                .t
                .ProcessOutput(0, std::slice::from_mut(&mut db), &mut status);
            let sample = std::mem::ManuallyDrop::take(&mut db.pSample);
            let _ = std::mem::ManuallyDrop::take(&mut db.pEvents);
            match res {
                Ok(()) => {}
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    // the encoder renegotiated - accept its type and go on
                    if let Ok(t) = self.t.GetOutputAvailableType(0, 0) {
                        let _ = self.t.SetOutputType(0, &t, 0);
                    }
                    return Ok(None);
                }
                Err(e) => return Err(anyhow!("ProcessOutput: {}", e)),
            }
            let s = match sample {
                Some(s) => s,
                None => return Ok(None),
            };
            let key = s.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) == 1;
            let data = sample_bytes(&s)?;
            if !data.is_empty() {
                out.push(Chunk { data, key });
            }
            Ok(Some(()))
        }

        /// Feeds one NV12 frame and returns whatever the encoder produced.
        /// Hardware encoders are pipelined, so the first call or two can come
        /// back empty - that is normal and not an error.
        pub fn encode(&mut self, nv12: &[u8]) -> Result<Vec<Chunk>> {
            if nv12.len() < self.nv12_len() {
                return Err(anyhow!(
                    "NV12 Puffer zu klein: {} < {}",
                    nv12.len(),
                    self.nv12_len()
                ));
            }
            let dur = 10_000_000i64 / self.fps.max(1) as i64;
            let time = self.frames as i64 * dur;
            let mut out = Vec::new();
            unsafe {
                let sample = sample_from_bytes(&nv12[..self.nv12_len()], time, dur)?;
                if self.events.is_some() {
                    // async MFT: wait for a token, then push the frame in
                    self.drain(false, &mut out)?;
                    if self.need <= 0 {
                        self.drain(true, &mut out)?;
                    }
                    if self.need <= 0 {
                        return Ok(out); // encoder is busy, drop this frame
                    }
                    self.t.ProcessInput(0, &sample, 0)?;
                    self.need -= 1;
                    self.drain(false, &mut out)?;
                } else {
                    self.t.ProcessInput(0, &sample, 0)?;
                    while self.pull(&mut out)?.is_some() {}
                }
            }
            self.frames += 1;
            Ok(out)
        }
    }

    impl Drop for Encoder {
        fn drop(&mut self) {
            unsafe {
                let _ = self.t.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
                let _ = self.t.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            }
        }
    }

    // ------------------------------------------------------------ decoder --

    pub struct Decoder {
        t: IMFTransform,
        provides_samples: bool,
        out_size: u32,
        /// geometry the decoder decided on (padded up to whole macroblocks)
        w: u32,
        h: u32,
        stride: usize,
        /// visible picture the host announced
        want_w: u32,
        want_h: u32,
        /// true = die Groesse kommt aus dem Strom, nicht vom Aufrufer.
        /// Im Meeting weiss niemand vorher, wie gross das Bild der anderen
        /// ist - mit einer festen Wunschgroesse saehe man nur einen
        /// Ausschnitt (genau das war beim Bildschirmteilen zu sehen).
        frei: bool,
        name: String,
        nv12: Vec<u8>,
    }

    impl Decoder {
        pub fn new(w: u32, h: u32) -> Result<Self> {
            Self::mit_groesse(w, h, false)
        }

        /// Dekodierer, der die Bildgroesse aus dem Strom uebernimmt. `w`/`h`
        /// sind nur ein erster Anhaltspunkt fuer die Aushandlung.
        pub fn new_auto(w: u32, h: u32) -> Result<Self> {
            Self::mit_groesse(w, h, true)
        }

        fn mit_groesse(w: u32, h: u32, frei: bool) -> Result<Self> {
            mf_startup();
            let input = type_info(MFMediaType_Video, MFVideoFormat_H264);
            let output = type_info(MFMediaType_Video, MFVideoFormat_NV12);
            // hardware decoders keep the CPU free; the Microsoft software
            // decoder is always there as a fallback
            let mut dec: Option<Self> = None;
            for flags in [
                MFT_ENUM_FLAG(MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0),
                MFT_ENUM_FLAG(MFT_ENUM_FLAG_SYNCMFT.0 | MFT_ENUM_FLAG_SORTANDFILTER.0),
            ] {
                if dec.is_some() {
                    break;
                }
                dec = find_mft(
                    MFT_CATEGORY_VIDEO_DECODER,
                    flags,
                    input,
                    output,
                    |a, name| match Self::build(a, name, w, h, frei) {
                        Ok(d) => Some(d),
                        Err(e) => {
                            crate::capture::log_line(&format!("h264 decoder {}: {}", name, e));
                            None
                        }
                    },
                );
            }
            dec.ok_or_else(|| anyhow!("kein H.264 Decoder gefunden"))
        }

        fn build(a: &IMFActivate, name: &str, w: u32, h: u32, frei: bool) -> Result<Self> {
            unsafe {
                let t: IMFTransform = a.ActivateObject()?;
                if let Ok(attrs) = t.GetAttributes() {
                    let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
                }
                let inp = MFCreateMediaType()?;
                inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
                inp.SetUINT64(&MF_MT_FRAME_SIZE, pack(w, h))?;
                inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                t.SetInputType(0, &inp, 0)
                    .map_err(|e| anyhow!("SetInputType: {}", e))?;

                let mut d = Self {
                    t,
                    provides_samples: false,
                    out_size: w * h * 3,
                    w,
                    h,
                    stride: w as usize,
                    want_w: w,
                    want_h: h,
                    frei,
                    name: name.to_string(),
                    nv12: Vec::new(),
                };
                d.pick_output()?;
                d.t.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
                d.t.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
                crate::capture::log_line(&format!("h264 decoder: {} {}x{}", name, w, h));
                Ok(d)
            }
        }

        /// Selects the first NV12 output type the decoder offers and remembers
        /// the geometry it decided on.
        fn pick_output(&mut self) -> Result<()> {
            unsafe {
                let mut i = 0u32;
                loop {
                    let t = match self.t.GetOutputAvailableType(0, i) {
                        Ok(t) => t,
                        Err(_) => return Err(anyhow!("kein NV12 Ausgabeformat")),
                    };
                    if t.GetGUID(&MF_MT_SUBTYPE).ok() == Some(MFVideoFormat_NV12) {
                        self.t
                            .SetOutputType(0, &t, 0)
                            .map_err(|e| anyhow!("SetOutputType: {}", e))?;
                        if let Ok(size) = t.GetUINT64(&MF_MT_FRAME_SIZE) {
                            self.w = (size >> 32) as u32;
                            self.h = (size & 0xffff_ffff) as u32;
                        }
                        if self.frei {
                            // Sichtbarer Bereich, falls der Dekodierer ihn
                            // nennt (1080 in 1088 aufgefuellt o. ae.),
                            // sonst die volle Flaeche.
                            let mut blob = [0u8; 32];
                            let mut len = 0u32;
                            let sicht = t
                                .GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut blob, Some(&mut len))
                                .is_ok()
                                && len >= 16;
                            if sicht {
                                let b = |i: usize| {
                                    i32::from_le_bytes([
                                        blob[i],
                                        blob[i + 1],
                                        blob[i + 2],
                                        blob[i + 3],
                                    ]) as u32
                                };
                                let (bw, bh) = (b(8), b(12));
                                if bw >= 16 && bh >= 16 && bw <= self.w && bh <= self.h {
                                    self.want_w = bw;
                                    self.want_h = bh;
                                } else {
                                    self.want_w = self.w;
                                    self.want_h = self.h;
                                }
                            } else {
                                self.want_w = self.w;
                                self.want_h = self.h;
                            }
                        }
                        self.stride = match t.GetUINT32(&MF_MT_DEFAULT_STRIDE) {
                            Ok(s) if s as usize >= self.w as usize => s as usize,
                            _ => self.w as usize,
                        };
                        let info = self.t.GetOutputStreamInfo(0)?;
                        self.provides_samples =
                            info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
                        self.out_size = info.cbSize.max(self.w * self.h * 3);
                        return Ok(());
                    }
                    i += 1;
                    if i > 32 {
                        return Err(anyhow!("kein NV12 Ausgabeformat"));
                    }
                }
            }
        }

        pub fn name(&self) -> &str {
            &self.name
        }
        pub fn size(&self) -> (u32, u32) {
            (self.want_w.min(self.w), self.want_h.min(self.h))
        }
        /// Geometry the decoder itself works in (padded, for diagnostics).
        pub fn raw_size(&self) -> (u32, u32, usize) {
            (self.w, self.h, self.stride)
        }

        /// Decodes one access unit. Returns the picture as RGBA plus its size,
        /// or `None` when the decoder still needs more data.
        pub fn decode(&mut self, au: &[u8], rgba: &mut Vec<u8>) -> Result<Option<(u32, u32)>> {
            unsafe {
                let s = sample_from_bytes(au, 0, 0)?;
                match self.t.ProcessInput(0, &s, 0) {
                    Ok(()) => {}
                    Err(e) if e.code() == MF_E_NOTACCEPTING => {}
                    Err(e) => return Err(anyhow!("ProcessInput: {}", e)),
                }
                let mut got = None;
                loop {
                    match self.pull()? {
                        Some(()) => {
                            got = Some(());
                        }
                        None => break,
                    }
                }
                if got.is_none() {
                    return Ok(None);
                }
                let w = self.want_w.min(self.w);
                let h = self.want_h.min(self.h);
                if !super::nv12_to_rgba(&self.nv12, w, h, self.stride, self.h, rgba) {
                    return Err(anyhow!("NV12 Bild unvollstaendig"));
                }
                Ok(Some((w, h)))
            }
        }

        unsafe fn pull(&mut self) -> Result<Option<()>> {
            let sample = if self.provides_samples {
                None
            } else {
                let buf = MFCreateMemoryBuffer(self.out_size)?;
                let s = MFCreateSample()?;
                s.AddBuffer(&buf)?;
                Some(s)
            };
            let mut db = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: std::mem::ManuallyDrop::new(sample),
                dwStatus: 0,
                pEvents: std::mem::ManuallyDrop::new(None),
            };
            let mut status = 0u32;
            let res = self
                .t
                .ProcessOutput(0, std::slice::from_mut(&mut db), &mut status);
            let sample = std::mem::ManuallyDrop::take(&mut db.pSample);
            let _ = std::mem::ManuallyDrop::take(&mut db.pEvents);
            match res {
                Ok(()) => {}
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    self.pick_output()?;
                    return Ok(None);
                }
                Err(e) => return Err(anyhow!("ProcessOutput: {}", e)),
            }
            let s = match sample {
                Some(s) => s,
                None => return Ok(None),
            };
            self.nv12 = sample_bytes(&s)?;
            Ok(Some(()))
        }
    }

    impl Drop for Decoder {
        fn drop(&mut self) {
            unsafe {
                let _ = self.t.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            }
        }
    }

    /// Is there any H.264 encoder on this machine?
    pub fn available() -> bool {
        Encoder::new(640, 480, 30, 2_000_000).is_ok()
    }
}

#[cfg(windows)]
pub use win::{available, Decoder, Encoder};

// Auf dem Mac uebernimmt VideoToolbox (eigene Datei, sonst wird h264.rs
// unuebersichtlich); alles andere bekommt weiter den ehrlichen Platzhalter.
#[cfg(target_os = "macos")]
#[path = "h264mac.rs"]
mod mac;

#[cfg(target_os = "macos")]
pub use mac::{available, Decoder, Encoder};

#[cfg(not(any(windows, target_os = "macos")))]
mod stub {
    use super::{Chunk, Result};
    use anyhow::anyhow;

    pub struct Encoder;
    impl Encoder {
        pub fn new(_w: u32, _h: u32, _fps: u32, _b: u32) -> Result<Self> {
            Err(anyhow!("H.264 nur unter Windows"))
        }
        pub fn name(&self) -> &str {
            "-"
        }
        pub fn hardware(&self) -> bool {
            false
        }
        pub fn size(&self) -> (u32, u32) {
            (0, 0)
        }
        pub fn nv12_len(&self) -> usize {
            0
        }
        pub fn request_keyframe(&mut self) {}
        pub fn encode(&mut self, _nv12: &[u8]) -> Result<Vec<Chunk>> {
            Err(anyhow!("H.264 nur unter Windows"))
        }
    }

    pub struct Decoder;
    impl Decoder {
        pub fn new(_w: u32, _h: u32) -> Result<Self> {
            Err(anyhow!("H.264 nur unter Windows"))
        }
        pub fn new_auto(_w: u32, _h: u32) -> Result<Self> {
            Err(anyhow!("H.264 nur unter Windows"))
        }
        pub fn name(&self) -> &str {
            "-"
        }
        pub fn size(&self) -> (u32, u32) {
            (0, 0)
        }
        /// Breite, Hoehe, Zeilenabstand - wie beim echten Dekodierer.
        pub fn raw_size(&self) -> (u32, u32, usize) {
            (0, 0, 0)
        }
        pub fn decode(&mut self, _au: &[u8], _rgba: &mut Vec<u8>) -> Result<Option<(u32, u32)>> {
            Ok(None)
        }
    }

    pub fn available() -> bool {
        false
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub use stub::{available, Decoder, Encoder};

#[cfg(test)]
mod tests {
    use super::*;

    fn testcard(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let o = ((y * w + x) * 3) as usize;
                v[o] = (x * 255 / w.max(1)) as u8;
                v[o + 1] = (y * 255 / h.max(1)) as u8;
                v[o + 2] = if (x / 16 + y / 16) % 2 == 0 { 220 } else { 30 };
            }
        }
        v
    }

    #[test]
    fn nv12_roundtrip_is_close_to_the_original() {
        let (w, h) = (64u32, 64u32);
        let rgb = testcard(w, h);
        let mut nv12 = Vec::new();
        rgb_to_nv12(&rgb, w, h, &mut nv12);
        assert_eq!(nv12.len(), (w * h * 3 / 2) as usize);
        let mut rgba = Vec::new();
        assert!(nv12_to_rgba(&nv12, w, h, w as usize, h, &mut rgba));
        let (mean, _max) = rgb_vs_rgba_error(&rgb, &rgba, w, h);
        // 4:2:0 subsampling on a hard checkerboard is the worst case, but the
        // average error still has to stay small
        assert!(mean < 12.0, "mittlerer Fehler {}", mean);
    }

    #[test]
    fn grey_survives_exactly() {
        let (w, h) = (16u32, 16u32);
        let rgb = vec![128u8; (w * h * 3) as usize];
        let mut nv12 = Vec::new();
        rgb_to_nv12(&rgb, w, h, &mut nv12);
        let mut rgba = Vec::new();
        assert!(nv12_to_rgba(&nv12, w, h, w as usize, h, &mut rgba));
        let (mean, max) = rgb_vs_rgba_error(&rgb, &rgba, w, h);
        assert!(mean < 1.5 && max <= 2, "mean {} max {}", mean, max);
    }

    /// Der ganze Weg einmal durch die Maschine: Bild -> NV12 -> H.264 ->
    /// zurueck. Laeuft ueberall, wo es einen Kodierer gibt (Windows, Mac);
    /// auf einem nackten Linux-Server gibt es keinen, dann wird der Test
    /// ehrlich uebersprungen statt falsch gruen zu melden.
    #[test]
    fn h264_hin_und_zurueck() {
        if !available() {
            println!("kein H.264-Kodierer auf dieser Maschine - uebersprungen");
            return;
        }
        let (w, h) = (320u32, 240u32);
        let rgb = testcard(w, h);
        let mut nv12 = Vec::new();
        rgb_to_nv12(&rgb, w, h, &mut nv12);
        let mut enc = Encoder::new(w, h, 30, 1_500_000).expect("Kodierer");
        let mut dec = Decoder::new_auto(w, h).expect("Dekodierer");
        let mut rgba = Vec::new();
        let mut bilder = 0;
        let mut bytes = 0usize;
        let mut schluessel = 0;
        for _ in 0..10 {
            for c in enc.encode(&nv12).expect("kodieren") {
                bytes += c.data.len();
                if c.key {
                    schluessel += 1;
                }
                // Startmarke muss vorne stehen, sonst versteht uns niemand.
                assert!(
                    c.data.starts_with(&[0, 0, 0, 1]) || c.data.starts_with(&[0, 0, 1]),
                    "kein Annex-B"
                );
                if let Ok(Some((dw, dh))) = dec.decode(&c.data, &mut rgba) {
                    assert_eq!((dw, dh), (w, h), "falsche Bildgroesse");
                    bilder += 1;
                }
            }
        }
        println!(
            "kodiert {} Bytes, {} Schluesselbilder, {} Bilder dekodiert",
            bytes, schluessel, bilder
        );
        assert!(bytes > 0, "nichts kodiert");
        assert!(schluessel > 0, "kein Schluesselbild");
        assert!(bilder > 0, "nichts dekodiert");
        // Und das Bild muss auch WIRKLICH dem Original aehneln.
        let (mean, _max) = rgb_vs_rgba_error(&rgb, &rgba, w, h);
        assert!(mean < 25.0, "Bild weicht zu stark ab: {}", mean);
    }

    #[test]
    fn short_buffers_are_rejected_instead_of_panicking() {
        let mut rgba = Vec::new();
        assert!(!nv12_to_rgba(&[0u8; 10], 64, 64, 64, 64, &mut rgba));
    }
}

// ------------------------------------------------------------ self test ----

/// A moving test picture: colour ramps plus a hard checkerboard (worst case
/// for a video codec) plus a bright block that travels across the screen.
fn testframe(w: u32, h: u32, step: u32, out: &mut Vec<u8>) {
    out.clear();
    out.resize((w * h * 3) as usize, 0);
    let bx = (step * 17) % w.max(1);
    let by = (step * 11) % h.max(1);
    for y in 0..h {
        for x in 0..w {
            let o = ((y * w + x) * 3) as usize;
            let inside = x >= bx && x < bx + 160 && y >= by && y < by + 90;
            if inside {
                out[o] = 250;
                out[o + 1] = 250;
                out[o + 2] = 40;
            } else {
                out[o] = (x * 255 / w.max(1)) as u8;
                out[o + 1] = (y * 255 / h.max(1)) as u8;
                out[o + 2] = if ((x / 8) + (y / 8)) % 2 == 0 { 200 } else { 40 };
            }
        }
    }
}

/// `freeviewer --h264test [frames]` - does hardware H.264 work on this
/// machine, how fast is it and how much does the picture suffer?
pub fn selftest(rounds: u32) -> String {
    let mut out = String::new();
    let (w, h) = (1920u32, 1080u32);
    let fps = 30u32;
    let bitrate = 10_000_000u32;

    let mut enc = match Encoder::new(w, h, fps, bitrate) {
        Ok(e) => e,
        Err(e) => return format!("kein H.264 Encoder: {}\n", e),
    };
    out.push_str(&format!(
        "encoder: {} ({}) {}x{} @{} fps, Ziel {} kbit/s\n",
        enc.name(),
        if enc.hardware() { "Hardware/GPU" } else { "Software/CPU" },
        w,
        h,
        fps,
        bitrate / 1000
    ));
    let mut dec = match Decoder::new(w, h) {
        Ok(d) => d,
        Err(e) => return out + &format!("kein H.264 Decoder: {}\n", e),
    };
    out.push_str(&format!("decoder: {}\n", dec.name()));
    let mut geom_printed = false;

    let mut rgb = Vec::new();
    let mut nv12 = Vec::new();
    let mut originals: Vec<Vec<u8>> = Vec::new();
    let mut units: Vec<Chunk> = Vec::new();
    let (mut t_conv, mut t_enc) = (0u128, 0u128);
    let mut bytes = 0usize;

    for i in 0..rounds {
        testframe(w, h, i, &mut rgb);
        let t = std::time::Instant::now();
        rgb_to_nv12(&rgb, w, h, &mut nv12);
        t_conv += t.elapsed().as_micros();
        if i < 4 {
            originals.push(rgb.clone());
        }
        let t = std::time::Instant::now();
        match enc.encode(&nv12) {
            Ok(cs) => {
                t_enc += t.elapsed().as_micros();
                for c in cs {
                    bytes += c.data.len();
                    units.push(c);
                }
            }
            Err(e) => return out + &format!("encode fehlgeschlagen: {}\n", e),
        }
    }

    let keys = units.iter().filter(|c| c.key).count();
    out.push_str(&format!(
        "{} Frames -> {} Einheiten ({} Keyframes), {:.1} KB/Frame, {:.2} Mbit/s bei {} fps\n",
        rounds,
        units.len(),
        keys,
        bytes as f32 / units.len().max(1) as f32 / 1024.0,
        bytes as f32 * 8.0 / 1_000_000.0 / (units.len().max(1) as f32 / fps as f32),
        fps
    ));
    out.push_str(&format!(
        "RGB->NV12 {:.2} ms/Frame (CPU) | encode {:.2} ms/Frame\n",
        t_conv as f32 / rounds.max(1) as f32 / 1000.0,
        t_enc as f32 / rounds.max(1) as f32 / 1000.0
    ));

    if units.is_empty() {
        return out + "FAIL: der Encoder hat nichts geliefert\n";
    }

    let mut rgba = Vec::new();
    let mut t_dec = 0u128;
    let mut decoded = 0usize;
    let mut first_err = None;
    for (i, c) in units.iter().enumerate() {
        let t = std::time::Instant::now();
        match dec.decode(&c.data, &mut rgba) {
            Ok(Some((dw, dh))) => {
                t_dec += t.elapsed().as_micros();
                if !geom_printed {
                    geom_printed = true;
                    let (rw, rh, rs) = dec.raw_size();
                    out.push_str(&format!(
                        "Decoder-Geometrie: sichtbar {}x{}, intern {}x{} (Zeilenabstand {})\n",
                        dw, dh, rw, rh, rs
                    ));
                }
                if decoded < originals.len() && dw == w && dh == h {
                    let (mean, max) = rgb_vs_rgba_error(&originals[decoded], &rgba, w, h);
                    if first_err.is_none() {
                        first_err = Some((mean, max));
                    }
                }
                decoded += 1;
                let _ = i;
            }
            Ok(None) => {
                t_dec += t.elapsed().as_micros();
            }
            Err(e) => return out + &format!("decode fehlgeschlagen: {}\n", e),
        }
    }
    out.push_str(&format!(
        "decode {} Bilder, {:.2} ms/Bild\n",
        decoded,
        t_dec as f32 / decoded.max(1) as f32 / 1000.0
    ));
    match first_err {
        Some((mean, max)) => out.push_str(&format!(
            "Bildfehler nach Encode+Decode: Mittel {:.2}, Max {} (von 255)\n",
            mean, max
        )),
        None => out.push_str("WARN: kein Bild konnte mit dem Original verglichen werden\n"),
    }
    if decoded == 0 {
        out.push_str("FAIL: der Decoder hat kein Bild geliefert\n");
    } else {
        out.push_str("OK\n");
    }
    out
}