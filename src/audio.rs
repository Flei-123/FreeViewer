//! Voice for a running session - "FreeMeet inside FreeViewer".
//!
//! Both sides of a session may send speech. The audio travels as ordinary
//! `Msg::Audio` messages inside the AES-256-GCM channel that host and viewer
//! already share, so the relay never hears anything and no extra port, login
//! or server is needed.
//!
//! Wire format: mono, 24 kHz, 20 ms per packet (480 samples), IMA-ADPCM at
//! 4 bit per sample - 243 bytes per packet, about 97 kbit/s. Every packet
//! carries its own predictor state, so a lost packet costs 20 ms of sound and
//! nothing else (no drift, no click cascade).
//!
//! Test helpers built in:
//!   freeviewer --audiodev            list the sound devices we would use
//!   freeviewer --audioloop <s>       microphone -> codec -> speaker, with numbers
//!   FV_AUDIO_TONE=1                  send a 440 Hz tone instead of the microphone
//!   FV_AUDIO_DUMP=<file.wav>         write everything we receive into a WAV file

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::proto::Msg;

/// Sample rate on the wire.
pub const RATE: u32 = 24_000;
/// Samples per packet (20 ms).
pub const FRAME: usize = 480;
/// Bytes of one encoded packet: predictor + index + 4 bit per sample.
pub const PACKET: usize = 3 + FRAME / 2;
/// Never buffer more than this much sound before dropping the oldest - a
/// listener that fell behind should get current speech, not a growing delay.
const MAX_QUEUE: usize = FRAME * 25; // 500 ms
/// Start playing once this much has arrived (jitter cushion).
const START_QUEUE: usize = FRAME * 2; // 40 ms

// ---------------------------------------------------------------- IMA-ADPCM

const STEP: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

fn clamp_index(i: i32) -> i32 {
    i.clamp(0, 88)
}

/// Encode one packet. `pcm` must be exactly FRAME samples.
pub fn encode_frame(pcm: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(PACKET);
    let mut pred = pcm.first().copied().unwrap_or(0) as i32;
    let mut index = 24_i32; // a middling step size; converges within a few samples
    out.extend_from_slice(&(pred as i16).to_le_bytes());
    out.push(index as u8);
    let mut nib_hi: Option<u8> = None;
    for &s in pcm.iter() {
        let step = STEP[index as usize];
        let mut diff = s as i32 - pred;
        let mut code = 0u8;
        if diff < 0 {
            code = 8;
            diff = -diff;
        }
        let mut mask = 4;
        let mut tmp = step;
        for _ in 0..3 {
            if diff >= tmp {
                code |= mask;
                diff -= tmp;
            }
            tmp >>= 1;
            mask >>= 1;
        }
        // mirror the decoder so both sides walk the same path
        pred = step_decode(code, step, &mut index, pred);
        match nib_hi.take() {
            None => nib_hi = Some(code & 0x0f),
            Some(lo) => out.push(lo | ((code & 0x0f) << 4)),
        }
    }
    if let Some(lo) = nib_hi {
        out.push(lo);
    }
    out
}

fn step_decode(code: u8, step: i32, index: &mut i32, pred: i32) -> i32 {
    let mut delta = step >> 3;
    if code & 4 != 0 {
        delta += step;
    }
    if code & 2 != 0 {
        delta += step >> 1;
    }
    if code & 1 != 0 {
        delta += step >> 2;
    }
    let next = if code & 8 != 0 { pred - delta } else { pred + delta };
    *index = clamp_index(*index + INDEX[(code & 0x0f) as usize]);
    next.clamp(-32768, 32767)
}

/// Decode one packet back into PCM. Returns an empty vector for garbage.
pub fn decode_frame(b: &[u8]) -> Vec<i16> {
    if b.len() < 3 {
        return Vec::new();
    }
    let mut pred = i16::from_le_bytes([b[0], b[1]]) as i32;
    let mut index = clamp_index(b[2] as i32);
    let mut out = Vec::with_capacity((b.len() - 3) * 2);
    for &byte in &b[3..] {
        for code in [byte & 0x0f, byte >> 4] {
            let step = STEP[index as usize];
            pred = step_decode(code, step, &mut index, pred);
            out.push(pred as i16);
        }
    }
    out
}

// ------------------------------------------------------------- resampling

/// Linear resample of mono PCM. Cheap, and at 24 kHz good enough for speech.
fn resample(src: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || src.is_empty() {
        return src.to_vec();
    }
    let n = ((src.len() as u64 * to as u64) / from as u64).max(1) as usize;
    let mut out = Vec::with_capacity(n);
    let ratio = from as f32 / to as f32;
    for i in 0..n {
        let pos = i as f32 * ratio;
        let a = pos as usize;
        let f = pos - a as f32;
        let s0 = *src.get(a).unwrap_or(&0) as f32;
        let s1 = *src.get(a + 1).unwrap_or(&(src[src.len() - 1])) as f32;
        out.push((s0 + (s1 - s0) * f) as i16);
    }
    out
}

// ----------------------------------------------------------------- the link

/// Everything the GUI wants to know and switch about the voice link.
pub struct VoiceState {
    /// Send my microphone?
    pub mic: AtomicBool,
    /// Play what the other side sends?
    pub speaker: AtomicBool,
    /// Packets sent / received (diagnostics).
    pub sent: AtomicU64,
    pub got: AtomicU64,
    /// Loudness 0..100 of the last moment, for the little level bars.
    pub level_out: AtomicU32,
    pub level_in: AtomicU32,
    /// Something went wrong with a sound device.
    pub problem: Mutex<String>,
}

impl Default for VoiceState {
    fn default() -> Self {
        Self {
            mic: AtomicBool::new(std::env::var("FV_AUDIO_MIC").as_deref() == Ok("1")),
            speaker: AtomicBool::new(true),
            sent: AtomicU64::new(0),
            got: AtomicU64::new(0),
            level_out: AtomicU32::new(0),
            level_in: AtomicU32::new(0),
            problem: Mutex::new(String::new()),
        }
    }
}

/// Live voice link of one session. Dropping it stops both sound devices.
pub struct Voice {
    stop: Arc<AtomicBool>,
    play: Arc<Mutex<VecDeque<i16>>>,
    state: Arc<VoiceState>,
    seq_in: Arc<AtomicU32>,
    dump: Arc<Mutex<Option<WavWriter>>>,
}

impl Drop for Voice {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.dump.lock().unwrap().take() {
            w.finish();
        }
    }
}

fn level_of(pcm: &[i16]) -> u32 {
    if pcm.is_empty() {
        return 0;
    }
    let sum: u64 = pcm.iter().map(|s| (*s as i32).unsigned_abs() as u64).sum();
    let avg = (sum / pcm.len() as u64) as f32;
    ((avg / 3000.0 * 100.0) as u32).min(100)
}

impl Voice {
    /// Starts microphone and speaker for this session. `send` puts a message
    /// into the session's outgoing pipeline (host or viewer, same signature).
    pub fn start(state: Arc<VoiceState>, send: Arc<dyn Fn(Msg) + Send + Sync>) -> Voice {
        let stop = Arc::new(AtomicBool::new(false));
        let play: Arc<Mutex<VecDeque<i16>>> = Arc::new(Mutex::new(VecDeque::new()));
        let dump = Arc::new(Mutex::new(
            std::env::var("FV_AUDIO_DUMP").ok().and_then(|p| WavWriter::create(&p)),
        ));

        // ---- microphone thread
        {
            let stop = stop.clone();
            let state = state.clone();
            let tone = std::env::var("FV_AUDIO_TONE").is_ok();
            std::thread::spawn(move || {
                if tone {
                    tone_loop(stop, state, send);
                } else if let Err(e) = mic_loop(stop.clone(), state.clone(), send) {
                    *state.problem.lock().unwrap() = format!("Mikrofon: {}", e);
                }
            });
        }
        // ---- speaker thread
        {
            let stop = stop.clone();
            let state = state.clone();
            let play = play.clone();
            std::thread::spawn(move || {
                if let Err(e) = speaker_loop(stop, state.clone(), play) {
                    *state.problem.lock().unwrap() = format!("Lautsprecher: {}", e);
                }
            });
        }

        Voice {
            stop,
            play,
            state,
            seq_in: Arc::new(AtomicU32::new(0)),
            dump,
        }
    }

    /// One packet arrived from the other side.
    pub fn feed(&self, seq: u32, data: &[u8]) {
        self.state.got.fetch_add(1, Ordering::Relaxed);
        self.seq_in.store(seq, Ordering::Relaxed);
        let pcm = decode_frame(data);
        if pcm.is_empty() {
            return;
        }
        self.state.level_in.store(level_of(&pcm), Ordering::Relaxed);
        if let Some(w) = self.dump.lock().unwrap().as_mut() {
            w.write(&pcm);
        }
        if !self.state.speaker.load(Ordering::Relaxed) {
            return;
        }
        let mut q = self.play.lock().unwrap();
        q.extend(pcm);
        while q.len() > MAX_QUEUE {
            q.pop_front();
        }
    }

    pub fn state(&self) -> Arc<VoiceState> {
        self.state.clone()
    }
}

/// Picks the default input device and pushes 20 ms packets into `send`.
fn mic_loop(
    stop: Arc<AtomicBool>,
    state: Arc<VoiceState>,
    send: Arc<dyn Fn(Msg) + Send + Sync>,
) -> anyhow::Result<()> {
    let host = cpal::default_host();
    let dev = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("kein Aufnahmegeraet"))?;
    let cfg = dev.default_input_config()?;
    let rate = cfg.sample_rate().0;
    let channels = cfg.channels() as usize;
    let pending: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
    let p2 = pending.clone();
    let err = |e| crate::capture::log_line(&format!("Mikrofon-Fehler: {}", e));

    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => dev.build_input_stream(
            &cfg.clone().into(),
            move |data: &[f32], _: &_| {
                let mut m = Vec::with_capacity(data.len() / channels);
                for f in data.chunks(channels) {
                    let s: f32 = f.iter().sum::<f32>() / channels as f32;
                    m.push((s.clamp(-1.0, 1.0) * 32767.0) as i16);
                }
                p2.lock().unwrap().extend(m);
            },
            err,
            None,
        )?,
        cpal::SampleFormat::I16 => dev.build_input_stream(
            &cfg.clone().into(),
            move |data: &[i16], _: &_| {
                let mut m = Vec::with_capacity(data.len() / channels);
                for f in data.chunks(channels) {
                    let s: i32 = f.iter().map(|x| *x as i32).sum::<i32>() / channels as i32;
                    m.push(s as i16);
                }
                p2.lock().unwrap().extend(m);
            },
            err,
            None,
        )?,
        cpal::SampleFormat::U16 => dev.build_input_stream(
            &cfg.clone().into(),
            move |data: &[u16], _: &_| {
                let mut m = Vec::with_capacity(data.len() / channels);
                for f in data.chunks(channels) {
                    let s: i32 = f.iter().map(|x| *x as i32 - 32768).sum::<i32>() / channels as i32;
                    m.push(s as i16);
                }
                p2.lock().unwrap().extend(m);
            },
            err,
            None,
        )?,
        other => anyhow::bail!("Aufnahmeformat {:?} nicht unterstuetzt", other),
    };
    stream.play()?;
    crate::capture::log_line(&format!(
        "Mikrofon: {} @ {} Hz, {} Kanal(e)",
        dev.name().unwrap_or_default(),
        rate,
        channels
    ));

    let mut carry: Vec<i16> = Vec::new();
    let mut seq: u32 = 0;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(10));
        let raw: Vec<i16> = {
            let mut p = pending.lock().unwrap();
            if p.is_empty() {
                continue;
            }
            std::mem::take(&mut *p)
        };
        if !state.mic.load(Ordering::Relaxed) {
            carry.clear();
            state.level_out.store(0, Ordering::Relaxed);
            continue;
        }
        carry.extend(resample(&raw, rate, RATE));
        while carry.len() >= FRAME {
            let chunk: Vec<i16> = carry.drain(..FRAME).collect();
            state.level_out.store(level_of(&chunk), Ordering::Relaxed);
            seq = seq.wrapping_add(1);
            send(Msg::Audio {
                seq,
                data: encode_frame(&chunk),
            });
            state.sent.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(())
}

/// Test source: a 440 Hz sine instead of a microphone.
fn tone_loop(stop: Arc<AtomicBool>, state: Arc<VoiceState>, send: Arc<dyn Fn(Msg) + Send + Sync>) {
    let mut phase = 0.0_f32;
    let mut seq: u32 = 0;
    let step = 2.0 * std::f32::consts::PI * 440.0 / RATE as f32;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(20));
        if !state.mic.load(Ordering::Relaxed) {
            continue;
        }
        let mut pcm = Vec::with_capacity(FRAME);
        for _ in 0..FRAME {
            pcm.push((phase.sin() * 12000.0) as i16);
            phase += step;
            if phase > 2.0 * std::f32::consts::PI {
                phase -= 2.0 * std::f32::consts::PI;
            }
        }
        state.level_out.store(level_of(&pcm), Ordering::Relaxed);
        seq = seq.wrapping_add(1);
        send(Msg::Audio {
            seq,
            data: encode_frame(&pcm),
        });
        state.sent.fetch_add(1, Ordering::Relaxed);
    }
}

/// Plays whatever is in the queue, silence when it runs dry.
fn speaker_loop(
    stop: Arc<AtomicBool>,
    state: Arc<VoiceState>,
    play: Arc<Mutex<VecDeque<i16>>>,
) -> anyhow::Result<()> {
    let host = cpal::default_host();
    let dev = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("kein Wiedergabegeraet"))?;
    let cfg = dev.default_output_config()?;
    let rate = cfg.sample_rate().0;
    let channels = cfg.channels() as usize;
    let started = Arc::new(AtomicBool::new(false));
    let err = |e| crate::capture::log_line(&format!("Lautsprecher-Fehler: {}", e));

    // pull `want` mono samples at 24 kHz, stretched to the device rate
    let q = play.clone();
    let st = started.clone();
    let pull = move |frames: usize| -> Vec<i16> {
        let need = ((frames as u64 * RATE as u64) / rate as u64) as usize + 1;
        let mut queue = q.lock().unwrap();
        if !st.load(Ordering::Relaxed) {
            if queue.len() < START_QUEUE {
                return vec![0; frames];
            }
            st.store(true, Ordering::Relaxed);
        }
        if queue.len() < need {
            st.store(false, Ordering::Relaxed);
            return vec![0; frames];
        }
        let src: Vec<i16> = queue.drain(..need).collect();
        drop(queue);
        let mut out = resample(&src, RATE, rate);
        out.resize(frames, 0);
        out
    };

    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => {
            let pull = pull.clone();
            dev.build_output_stream(
                &cfg.clone().into(),
                move |data: &mut [f32], _: &_| {
                    let frames = data.len() / channels;
                    let mono = pull(frames);
                    for (i, f) in data.chunks_mut(channels).enumerate() {
                        let s = *mono.get(i).unwrap_or(&0) as f32 / 32768.0;
                        for c in f.iter_mut() {
                            *c = s;
                        }
                    }
                },
                err,
                None,
            )?
        }
        cpal::SampleFormat::I16 => {
            let pull = pull.clone();
            dev.build_output_stream(
                &cfg.clone().into(),
                move |data: &mut [i16], _: &_| {
                    let frames = data.len() / channels;
                    let mono = pull(frames);
                    for (i, f) in data.chunks_mut(channels).enumerate() {
                        let s = *mono.get(i).unwrap_or(&0);
                        for c in f.iter_mut() {
                            *c = s;
                        }
                    }
                },
                err,
                None,
            )?
        }
        cpal::SampleFormat::U16 => {
            let pull = pull.clone();
            dev.build_output_stream(
                &cfg.clone().into(),
                move |data: &mut [u16], _: &_| {
                    let frames = data.len() / channels;
                    let mono = pull(frames);
                    for (i, f) in data.chunks_mut(channels).enumerate() {
                        let s = (*mono.get(i).unwrap_or(&0) as i32 + 32768) as u16;
                        for c in f.iter_mut() {
                            *c = s;
                        }
                    }
                },
                err,
                None,
            )?
        }
        other => anyhow::bail!("Wiedergabeformat {:?} nicht unterstuetzt", other),
    };
    stream.play()?;
    crate::capture::log_line(&format!(
        "Lautsprecher: {} @ {} Hz, {} Kanal(e)",
        dev.name().unwrap_or_default(),
        rate,
        channels
    ));
    let _ = &state;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}

// ------------------------------------------------------------- WAV for tests

pub struct WavWriter {
    file: std::fs::File,
    samples: u32,
    path: std::path::PathBuf,
}

impl WavWriter {
    pub fn create(path: &str) -> Option<WavWriter> {
        use std::io::Write;
        let mut file = std::fs::File::create(path).ok()?;
        let header = [0u8; 44];
        file.write_all(&header).ok()?;
        Some(WavWriter {
            file,
            samples: 0,
            path: std::path::PathBuf::from(path),
        })
    }
    pub fn write(&mut self, pcm: &[i16]) {
        use std::io::Write;
        let mut b = Vec::with_capacity(pcm.len() * 2);
        for s in pcm {
            b.extend_from_slice(&s.to_le_bytes());
        }
        if self.file.write_all(&b).is_ok() {
            self.samples += pcm.len() as u32;
        }
    }
    /// Writes the real RIFF header now that the length is known.
    pub fn finish(mut self) {
        use std::io::{Seek, SeekFrom, Write};
        let data = self.samples * 2;
        let mut h = Vec::with_capacity(44);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&(36 + data).to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes()); // PCM
        h.extend_from_slice(&1u16.to_le_bytes()); // mono
        h.extend_from_slice(&RATE.to_le_bytes());
        h.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
        h.extend_from_slice(&2u16.to_le_bytes()); // block align
        h.extend_from_slice(&16u16.to_le_bytes()); // bits
        h.extend_from_slice(b"data");
        h.extend_from_slice(&data.to_le_bytes());
        let _ = self.file.seek(SeekFrom::Start(0));
        let _ = self.file.write_all(&h);
        let _ = self.file.flush();
        crate::capture::log_line(&format!(
            "Audio-Mitschnitt: {} ({} Samples)",
            self.path.display(),
            self.samples
        ));
    }
}

// ------------------------------------------------------------- test commands

/// Klarnamen der beiden Geraete, fuer die Einstellungen.
pub fn device_names() -> (String, String) {
    let host = cpal::default_host();
    let mic = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_else(|| "-".to_string());
    let spk = host
        .default_output_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_else(|| "-".to_string());
    (mic, spk)
}

/// `freeviewer --audiodev`
pub fn list_devices() -> String {
    let host = cpal::default_host();
    let mut s = String::new();
    match host.default_input_device() {
        Some(d) => {
            let cfg = d.default_input_config();
            s.push_str(&format!(
                "Mikrofon:      {} ({})\n",
                d.name().unwrap_or_default(),
                cfg.map(|c| format!("{} Hz, {} Kanal(e), {:?}", c.sample_rate().0, c.channels(), c.sample_format()))
                    .unwrap_or_else(|e| e.to_string())
            ));
        }
        None => s.push_str("Mikrofon:      keines\n"),
    }
    match host.default_output_device() {
        Some(d) => {
            let cfg = d.default_output_config();
            s.push_str(&format!(
                "Lautsprecher:  {} ({})\n",
                d.name().unwrap_or_default(),
                cfg.map(|c| format!("{} Hz, {} Kanal(e), {:?}", c.sample_rate().0, c.channels(), c.sample_format()))
                    .unwrap_or_else(|e| e.to_string())
            ));
        }
        None => s.push_str("Lautsprecher:  keiner\n"),
    }
    s.push_str(&format!(
        "Wire:          {} Hz mono, {} ms je Paket, {} Byte = {:.0} kbit/s\n",
        RATE,
        FRAME * 1000 / RATE as usize,
        PACKET,
        PACKET as f32 * 8.0 * (RATE as f32 / FRAME as f32) / 1000.0
    ));
    s
}

/// `freeviewer --audioloop <sekunden>`: microphone -> codec -> speaker on this
/// machine, and a number that says how much the codec changed the sound.
pub fn loopback_test(secs: u64) -> String {
    let state = Arc::new(VoiceState::default());
    state.mic.store(true, Ordering::Relaxed);
    let seen: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let sink: Arc<dyn Fn(Msg) + Send + Sync> = Arc::new(move |m: Msg| {
        if let Msg::Audio { data, .. } = m {
            s2.lock().unwrap().push(data);
        }
    });
    let voice = Voice::start(state.clone(), sink);
    let t0 = std::time::Instant::now();
    // play back what the microphone produced, one packet at a time
    let mut fed = 0usize;
    while t0.elapsed().as_secs() < secs {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let packets: Vec<Vec<u8>> = {
            let mut g = seen.lock().unwrap();
            std::mem::take(&mut *g)
        };
        for p in packets {
            voice.feed(fed as u32 + 1, &p);
            fed += 1;
        }
    }
    let sent = state.sent.load(Ordering::Relaxed);
    let got = state.got.load(Ordering::Relaxed);
    let secs_f = t0.elapsed().as_secs_f32();
    format!(
        "Pakete gesendet {} ({:.1}/s, erwartet {:.1}/s), zurueckgespielt {}, \
         Bitrate {:.0} kbit/s, Pegel raus {} / rein {}{}",
        sent,
        sent as f32 / secs_f,
        1000.0 / (FRAME as f32 * 1000.0 / RATE as f32),
        got,
        sent as f32 * PACKET as f32 * 8.0 / secs_f / 1000.0,
        state.level_out.load(Ordering::Relaxed),
        state.level_in.load(Ordering::Relaxed),
        {
            let p = state.problem.lock().unwrap().clone();
            if p.is_empty() {
                String::new()
            } else {
                format!("\nPROBLEM: {}", p)
            }
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(n: usize, hz: f32) -> Vec<i16> {
        (0..n)
            .map(|i| {
                ((i as f32 * 2.0 * std::f32::consts::PI * hz / RATE as f32).sin() * 12000.0) as i16
            })
            .collect()
    }

    #[test]
    fn packet_has_the_documented_size() {
        let pcm = sine(FRAME, 440.0);
        assert_eq!(encode_frame(&pcm).len(), PACKET);
    }

    #[test]
    fn codec_keeps_speech_recognizable() {
        // 300 Hz is in the middle of the voice band
        let pcm = sine(FRAME, 300.0);
        let back = decode_frame(&encode_frame(&pcm));
        assert_eq!(back.len(), FRAME);
        let (mut sig, mut noise) = (0f64, 0f64);
        // skip the first few samples: the predictor has to catch up first
        for i in 8..FRAME {
            let a = pcm[i] as f64;
            let b = back[i] as f64;
            sig += a * a;
            noise += (a - b) * (a - b);
        }
        let snr = 10.0 * (sig / noise.max(1e-9)).log10();
        assert!(snr > 20.0, "SNR zu schlecht: {:.1} dB", snr);
    }

    #[test]
    fn garbage_never_panics() {
        assert!(decode_frame(&[]).is_empty());
        assert!(decode_frame(&[1, 2]).is_empty());
        let _ = decode_frame(&[0, 0, 200, 7, 9, 250]);
        let _ = decode_frame(&vec![0xff; 300]);
    }

    #[test]
    fn resampling_keeps_the_length_ratio() {
        let src = sine(480, 300.0);
        assert_eq!(resample(&src, 24_000, 48_000).len(), 960);
        assert_eq!(resample(&src, 24_000, 24_000).len(), 480);
        assert_eq!(resample(&src, 48_000, 24_000).len(), 240);
    }

    #[test]
    fn quiet_input_stays_quiet() {
        let pcm = vec![0i16; FRAME];
        let back = decode_frame(&encode_frame(&pcm));
        assert!(back.iter().all(|s| s.abs() < 40), "Stille rauscht");
    }
}
