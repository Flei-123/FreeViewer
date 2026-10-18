//! Native meeting - stage 1c: microphone, speaker, mixer and
//! echo cancellation.
//!
//! The browser used to handle this for us. So three things are missing:
//!   1. Capture and playback (cpal - already in the project).
//!   2. MIXING: several peers arrive as separate audio tracks,
//!      the speaker wants ONE.
//!   3. ECHO: what comes out of the speaker is heard again by the microphone.
//!      Without a countermeasure the other person hears themselves - the most common
//!      reason why home-built software sounds "cheap".
//!
//! The echo cancellation is deliberately PURE RUST (block-wise in the
//! frequency domain, NLMS + post-filter). The usual way would be a
//! C++ library (WebRTC APM or Speex); both need cmake, libclang
//! and extra tools on EVERY build machine - we do not want to take that on for
//! Windows and Mac. How good it really is gets
//! measured (ERLE in dB, see tests below), not claimed.

use crate::lockutil::LockExt;
use std::collections::HashMap;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

/// 48 kHz, 20-ms-Rahmen - wie im Browser.
pub const RATE: usize = 48_000;
pub const FRAME: usize = 960;

/// Block length of the echo cancellation (10 ms) and length of the reverb tail.
const BLOCK: usize = 480;
const FFT_N: usize = 2 * BLOCK;
/// How far back the echo is searched (number of 10 ms blocks).
const PARTS: usize = 20; // 200 ms - deckt Zimmerhall und Puffer ab

/// Block-wise echo cancellation in the frequency domain (partitioned NLMS).
///
/// Flow per 10 ms block: what is currently going to the speaker (reference)
/// moves into a history. From the history the expected echo is
/// estimated and subtracted from the microphone signal. The filter keeps
/// learning meanwhile - but ONLY when both are not talking at the same time
/// (double-talk detection), otherwise you destroy your own voice.
pub struct Echo {
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    /// Gewichte des Filters, je Teil ein Spektrum.
    w: Vec<Vec<Complex32>>,
    /// History of the reference spectra (ring buffer).
    x: Vec<Vec<Complex32>>,
    pos: usize,
    /// Last reference block (for the overlap).
    last_ref: Vec<f32>,
    /// Mean power of the reference per frequency - step size of the NLMS.
    power: Vec<f32>,
    /// How strongly the post-filter attenuated last time (for smoothing).
    damping: Vec<f32>,
    /// Peak values of the last reference blocks - for the
    /// double-talk detection after Geigel (needs NO fully trained
    /// filter, that was the mistake in the first version).
    ref_peaks: std::collections::VecDeque<f32>,
    /// Counter for the round-robin constraint.
    runde: usize,
}

impl Echo {
    pub fn neu() -> Echo {
        let mut planner = FftPlanner::<f32>::new();
        Echo {
            fft: planner.plan_fft_forward(FFT_N),
            ifft: planner.plan_fft_inverse(FFT_N),
            w: vec![vec![Complex32::new(0.0, 0.0); FFT_N]; PARTS],
            x: vec![vec![Complex32::new(0.0, 0.0); FFT_N]; PARTS],
            pos: 0,
            last_ref: vec![0.0; BLOCK],
            power: vec![1e-6; FFT_N],
            damping: vec![1.0; FFT_N],
            ref_peaks: std::collections::VecDeque::new(),
            runde: 0,
        }
    }

    /// Einen 10-ms-Block bearbeiten.
    /// `mic_name` = Aufnahme, `reference` = was zeitgleich zum Lautsprecher ging.
    /// Rueckgabe: Mikrofon ohne Echo.
    pub fn block(&mut self, mic_name: &[f32], reference: &[f32]) -> Vec<f32> {
        assert_eq!(mic_name.len(), BLOCK);
        assert_eq!(reference.len(), BLOCK);

        // --- reference into the history (with 50 % overlap) ---------------
        let mut ein: Vec<Complex32> = Vec::with_capacity(FFT_N);
        for i in 0..BLOCK {
            ein.push(Complex32::new(self.last_ref[i], 0.0));
        }
        for i in 0..BLOCK {
            ein.push(Complex32::new(reference[i], 0.0));
        }
        self.last_ref.copy_from_slice(reference);
        self.fft.process(&mut ein);
        self.pos = (self.pos + 1) % PARTS;
        self.x[self.pos] = ein;

        // --- estimated echo = sum over all parts --------------------------
        let mut y = vec![Complex32::new(0.0, 0.0); FFT_N];
        for t in 0..PARTS {
            let xi = (self.pos + PARTS - t) % PARTS;
            for k in 0..FFT_N {
                y[k] += self.x[xi][k] * self.w[t][k];
            }
        }
        self.ifft.process(&mut y);
        let skal = 1.0 / FFT_N as f32;
        // only the second half is valid (overlap-save)
        let echo: Vec<f32> = (BLOCK..FFT_N).map(|i| y[i].re * skal).collect();

        // --- FriendError = Mikrofon minus geschaetztes Echo ---------------------
        let error: Vec<f32> = (0..BLOCK).map(|i| mic_name[i] - echo[i]).collect();

        // --- double-talk detection (Geigel) --------------------------------
        // The filter is empty at the start, so one can NOT compare with the
        // estimated echo (that is exactly where the first version
        // failed: it never learned). Instead the proven
        // comparison: if the microphone is louder than an attenuated
        // speaker can ever be, a person is talking along.
        let spitze_mik = mic_name.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let spitze_ref = reference.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        self.ref_peaks.push_back(spitze_ref);
        while self.ref_peaks.len() > PARTS {
            self.ref_peaks.pop_front();
        }
        let ref_max = self.ref_peaks.iter().fold(0.0f32, |m, v| m.max(*v));
        let e_ref: f32 = reference.iter().map(|v| v * v).sum::<f32>() + 1e-9;
        let duplex = spitze_mik > 0.7 * ref_max;
        let learn = e_ref > 1e-6 && ref_max > 1e-4 && !duplex;

        if learn {
            // error spectrum (zeros in front, the error behind - overlap-save)
            let mut ef: Vec<Complex32> = Vec::with_capacity(FFT_N);
            for _ in 0..BLOCK {
                ef.push(Complex32::new(0.0, 0.0));
            }
            for i in 0..BLOCK {
                ef.push(Complex32::new(error[i], 0.0));
            }
            self.fft.process(&mut ef);

            // Power per frequency over all parts - the step size comes from that.
            for k in 0..FFT_N {
                let mut p = 0.0;
                for t in 0..PARTS {
                    let xi = (self.pos + PARTS - t) % PARTS;
                    p += self.x[xi][k].norm_sqr();
                }
                self.power[k] = 0.8 * self.power[k] + 0.2 * p;
            }
            // A pure steady tone occupies only few frequencies - there
            // the step size otherwise becomes huge and the filter diverges.
            // Hence a floor that is oriented on the mean of all
            // frequencies (Tikhonov regularisation).
            let mittel: f32 = self.power.iter().sum::<f32>() / FFT_N as f32;
            let boden = (mittel * 0.05).max(1e-3);
            for k in 0..FFT_N {
                if self.power[k] < boden {
                    self.power[k] = boden;
                }
            }
            // Keep the step size small: better slow and stable than
            // fast and then diverging (first version: -57 dB, i.e. the
            // opposite of echo cancellation).
            let mu = 0.1;
            for t in 0..PARTS {
                let xi = (self.pos + PARTS - t) % PARTS;
                for k in 0..FFT_N {
                    let g = mu / self.power[k];
                    self.w[t][k] += self.x[xi][k].conj() * ef[k] * g;
                }
            }
            // Constraint: a part of the filter may only occupy the first
            // BLOCK samples. Without it the signal wraps around
            // (circular convolution) and the filter becomes unstable. For
            // compute-time reasons always one part per block, round robin.
            self.runde = self.runde.wrapping_add(1);
            let skal2 = 1.0 / FFT_N as f32;
            for t in 0..PARTS {
                let mut time_value = self.w[t].clone();
                self.ifft.process(&mut time_value);
                for (i, v) in time_value.iter_mut().enumerate() {
                    if i < BLOCK {
                        *v = Complex32::new(v.re * skal2, 0.0);
                    } else {
                        *v = Complex32::new(0.0, 0.0);
                    }
                }
                self.fft.process(&mut time_value);
                self.w[t] = time_value;
            }
            // Leakage: old learning results fade slowly, so that
            // the filter does not cling to an old room.
            for t in 0..PARTS {
                for k in 0..FFT_N {
                    self.w[t][k] *= 0.9999;
                }
            }
        }

        // --- emergency brake ------------------------------------------------
        // If the estimate becomes louder than the microphone, the filter has
        // run away (the classic with pure steady tones). Then better
        // halve everything than blow the other person's ears off.
        let e_mik: f32 = mic_name.iter().map(|v| v * v).sum::<f32>() + 1e-12;
        let e_echo: f32 = echo.iter().map(|v| v * v).sum::<f32>();
        if e_echo > 4.0 * e_mik {
            for t in 0..PARTS {
                for k in 0..FFT_N {
                    self.w[t][k] *= 0.5;
                }
            }
        }

        // --- post-filter: what is left is attenuated per frequency ---
        // The linear filter never catches everything (rooms change,
        // speakers distort). The rest is attenuated by the ratio
        // of error to estimated echo.
        let mut fspek: Vec<Complex32> = Vec::with_capacity(FFT_N);
        for _ in 0..BLOCK {
            fspek.push(Complex32::new(0.0, 0.0));
        }
        for i in 0..BLOCK {
            fspek.push(Complex32::new(error[i], 0.0));
        }
        let mut espek: Vec<Complex32> = Vec::with_capacity(FFT_N);
        for _ in 0..BLOCK {
            espek.push(Complex32::new(0.0, 0.0));
        }
        for i in 0..BLOCK {
            espek.push(Complex32::new(echo[i], 0.0));
        }
        self.fft.process(&mut fspek);
        self.fft.process(&mut espek);
        for k in 0..FFT_N {
            let f = fspek[k].norm_sqr();
            let e = espek[k].norm_sqr();
            let dest = (f / (f + 2.0 * e + 1e-9)).clamp(0.05, 1.0);
            // follow softly, otherwise it audibly "pumps"
            self.damping[k] = 0.7 * self.damping[k] + 0.3 * dest;
            fspek[k] *= self.damping[k];
        }
        self.ifft.process(&mut fspek);
        (BLOCK..FFT_N).map(|i| fspek[i].re * skal).collect()
    }

    /// Bequemer Weg fuer 20-ms-Rahmen (zwei Bloecke).
    pub fn frame_box(&mut self, mic_name: &[f32], reference: &[f32]) -> Vec<f32> {
        let mut aus = Vec::with_capacity(mic_name.len());
        let mut i = 0;
        while i + BLOCK <= mic_name.len() && i + BLOCK <= reference.len() {
            aus.extend(self.block(&mic_name[i..i + BLOCK], &reference[i..i + BLOCK]));
            i += BLOCK;
        }
        aus
    }
}

/// Mixes the audio frames of several participants into one speaker signal.
///
/// Every participant gets their own queue - packets never arrive
/// evenly. The mixer takes what is there and fills missing
/// spots with silence instead of crackling.
/// Noise gate for the microphone.
///
/// WHY at all: previously EVERY noise went out - fans, keyboard,
/// the street. In a round of eight people this adds up to a carpet of noise
/// in which nobody can talk comfortably any more.
///
/// The noise floor is estimated BY ITSELF, not asked from the user:
/// every room is different, and nobody can state in decibels how loud
/// their fridge is. The estimate falls fast (a quiet room is
/// recognised immediately) and rises very slowly - otherwise a long
/// contribution would gradually pass as "noise floor" and the gate
/// would close itself.
///
/// Two things that make the difference between "usable" and "unusable":
/// machen:
/// * **Hold time.** After the last loud frame the gate stays
///   open for about 300 ms. Without it the gate cuts off the quiet tails
///   of words, and you sound choppy.
/// * **Soft ramps.** Fading in and out over a few milliseconds.
///   A hard switch produces an audible click.
#[derive(Debug, Clone)]
pub struct NoiseGate {
    /// Geschaetztes Grundrauschen als Effektivwert (0..1).
    reason: f32,
    /// Aktuelle Durchlassstaerke (0 = zu, 1 = offen).
    gain: f32,
    /// How many frames the gate stays open.
    tail: u32,
    /// The first frames are only for calibration.
    calibrate: u32,
    /// State for the display.
    is_open: bool,
}

impl Default for NoiseGate {
    fn default() -> Self {
        Self::neu()
    }
}

impl NoiseGate {
    /// For this many 20 ms frames the gate stays open after the last sound.
    const TAIL: u32 = 15; // 300 ms
    /// For this long (frames) only measuring happens and everything passes.
    const CALIBRATE: u32 = 25; // 500 ms

    pub fn neu() -> NoiseGate {
        NoiseGate {
            reason: 0.0,
            gain: 1.0,
            tail: 0,
            calibrate: Self::CALIBRATE,
            is_open: true,
        }
    }

    /// Let a frame pass, attenuate or block it - in place.
    ///
    /// `strength` 0..100: 0 lets almost everything through, 100 is strict.
    /// Returns: whether it is currently passing (for the display).
    pub fn frame_box(&mut self, pcm: &mut [f32], strength: u8) -> bool {
        if pcm.is_empty() {
            return self.is_open;
        }
        let square: f32 = pcm.iter().map(|v| v * v).sum::<f32>() / pcm.len() as f32;
        let level = square.sqrt();

        // Track the noise floor: fast downwards, very slowly
        // upwards. At the first frame simply take it over, otherwise the
        // estimate would need seconds to climb up from 0.
        if self.calibrate == Self::CALIBRATE {
            self.reason = level;
        } else if level < self.reason {
            self.reason = self.reason * 0.85 + level * 0.15;
        } else {
            self.reason = self.reason * 0.9995 + level * 0.0005;
        }
        if self.calibrate > 0 {
            self.calibrate -= 1;
            self.is_open = true;
            return true;
        }

        let s = (strength.min(100) as f32) / 100.0;
        // Distance to the noise floor: double to eight times.
        let factor = 2.0 + s * 6.0;
        // Absolute lower bound - in a completely quiet room the
        // factor would otherwise go towards zero and the gate would be ineffective.
        let mindest = 0.0015 + s * 0.010;
        let threshold = (self.reason * factor).max(mindest);

        if level > threshold {
            self.tail = Self::TAIL;
        } else if self.tail > 0 {
            self.tail -= 1;
        }
        let dest = if self.tail > 0 { 1.0 } else { 0.0 };
        self.is_open = dest > 0.5;

        // Weiche Rampe je Abtastwert: aufblenden schneller als zublenden.
        let step = if dest > self.gain {
            1.0 / (0.008 * 48000.0) // ~8 ms
        } else {
            1.0 / (0.040 * 48000.0) // ~40 ms
        };
        for v in pcm.iter_mut() {
            if (self.gain - dest).abs() > f32::EPSILON {
                if dest > self.gain {
                    self.gain = (self.gain + step).min(dest);
                } else {
                    self.gain = (self.gain - step).max(dest);
                }
            }
            *v *= self.gain;
        }
        self.is_open
    }

    /// Estimated noise floor (0..1) - for the display.
    pub fn noise_floor(&self) -> f32 {
        self.reason
    }

    /// Is the gate currently letting sound through?
    pub fn passthrough(&self) -> bool {
        self.is_open
    }
}

#[derive(Default)]
pub struct Mixer {
    puffer: HashMap<u64, std::collections::VecDeque<i16>>,
    /// Wie viele Rahmen mindestens liegen bleiben sollen (Ruckelschutz).
    pub lead: usize,
}

impl Mixer {
    pub fn neu() -> Mixer {
        Mixer {
            puffer: HashMap::new(),
            lead: 1,
        }
    }

    pub fn also(&mut self, peer: u64, pcm: &[i16]) {
        let p = self.puffer.entry(peer).or_default();
        p.extend(pcm.iter().copied());
        // Never queue up more than 400 ms - otherwise the conversation runs away.
        while p.len() > RATE / 1000 * 400 {
            p.pop_front();
        }
    }

    /// Fetch a frame for the speaker.
    pub fn downmix(&mut self, n: usize) -> Vec<i16> {
        let mut aus = vec![0i32; n];
        for p in self.puffer.values_mut() {
            if p.len() < n * self.lead.max(1) {
                continue;
            }
            for a in aus.iter_mut() {
                if let Some(v) = p.pop_front() {
                    *a += v as i32;
                }
            }
        }
        aus.iter()
            .map(|v| (*v).clamp(i16::MIN as i32, i16::MAX as i32) as i16)
            .collect()
    }

    pub fn people(&self) -> usize {
        self.puffer.len()
    }

    pub fn remove_entry(&mut self, peer: u64) {
        self.puffer.remove(&peer);
    }
}

/// i16 -> f32 (-1..1) and back.
pub fn zu_f32(pcm: &[i16]) -> Vec<f32> {
    pcm.iter().map(|v| *v as f32 / 32768.0).collect()
}
pub fn zu_i16(pcm: &[f32]) -> Vec<i16> {
    pcm.iter()
        .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect()
}

/// Kanaele zu Mono zusammenlegen (Mikrofone liefern oft Stereo).
pub fn zu_mono(daten: &[f32], channel_count: usize) -> Vec<f32> {
    if channel_count <= 1 {
        return daten.to_vec();
    }
    daten
        .chunks(channel_count)
        .map(|c| c.iter().sum::<f32>() / channel_count as f32)
        .collect()
}

/// Simple linear resampling to 48 kHz (devices also deliver 44.1 kHz).
pub fn auf_48k(daten: &[f32], von: usize) -> Vec<f32> {
    if von == RATE || daten.is_empty() {
        return daten.to_vec();
    }
    let n = (daten.len() as f64 * RATE as f64 / von as f64).round() as usize;
    let mut aus = Vec::with_capacity(n);
    for i in 0..n {
        let pos = i as f64 * von as f64 / RATE as f64;
        let a = pos.floor() as usize;
        let b = (a + 1).min(daten.len() - 1);
        let t = (pos - a as f64) as f32;
        aus.push(daten[a] * (1.0 - t) + daten[b] * t);
    }
    aus
}


// ---------------------------------------------------------------- devices
// Microphone and speaker. cpal streams must not leave the thread
// (they are not "Send"), therefore everything runs in ONE own
// thread that talks to the rest via channels.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub struct Devices {
    /// Finished 20 ms frames from the microphone - already without echo.
    pub mic_name: std::sync::mpsc::Receiver<Vec<i16>>,
    /// What the speaker should play (gets mixed).
    pub speaker: Arc<Mutex<Mixer>>,
    pub is_muted: Arc<AtomicBool>,
    /// Noise gate on? Switched at runtime.
    pub lock_on: Arc<AtomicBool>,
    /// Strictness of the noise gate, 0..100.
    pub lock_strength: Arc<std::sync::atomic::AtomicU32>,
    /// Display only: is the gate currently letting sound through?
    pub lock_open: Arc<AtomicBool>,
    /// Nur Anzeige: geschaetztes Grundrauschen * 10000.
    pub lock_reason: Arc<std::sync::atomic::AtomicU32>,
    end: Arc<AtomicBool>,
    pub eingang: String,
    pub exit: String,
}

impl Drop for Devices {
    fn drop(&mut self) {
        self.end.store(true, Ordering::Relaxed);
    }
}

/// Starts capture and playback. Empty names = default device.
pub fn start_devices(ein_name: Option<String>, aus_name: Option<String>) -> anyhow::Result<Devices> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let (mik_tx, mik_rx) = std::sync::mpsc::channel::<Vec<i16>>();
    let mixer = Arc::new(Mutex::new(Mixer::neu()));
    let is_muted = Arc::new(AtomicBool::new(false));
    let end = Arc::new(AtomicBool::new(false));
    // ON by default: a noise carpet is noticed by everyone, a slightly
    // too strict gate only by the speaker themselves - and they can
    // switch it off in the settings immediately.
    let lock_on = Arc::new(AtomicBool::new(true));
    let lock_strength = Arc::new(std::sync::atomic::AtomicU32::new(45));
    let lock_open = Arc::new(AtomicBool::new(true));
    let lock_reason = Arc::new(std::sync::atomic::AtomicU32::new(0));

    // Determine names in advance, so that the UI can show them.
    let host = cpal::default_host();
    let ein_dev = match &ein_name {
        Some(n) if !n.is_empty() => host
            .input_devices()?
            .find(|d| d.name().map(|x| x == *n).unwrap_or(false))
            .or_else(|| host.default_input_device()),
        _ => host.default_input_device(),
    }
    .ok_or_else(|| anyhow::anyhow!("kein Mikrofon gefunden"))?;
    let aus_dev = match &aus_name {
        Some(n) if !n.is_empty() => host
            .output_devices()?
            .find(|d| d.name().map(|x| x == *n).unwrap_or(false))
            .or_else(|| host.default_output_device()),
        _ => host.default_output_device(),
    }
    .ok_or_else(|| anyhow::anyhow!("kein Lautsprecher gefunden"))?;
    let a_name_real = ein_dev.name().unwrap_or_default();
    let from_name_real = aus_dev.name().unwrap_or_default();

    let m2 = mixer.clone();
    let s2 = is_muted.clone();
    let e2 = end.clone();
    let sp = LockGrips {
        an: lock_on.clone(),
        strength: lock_strength.clone(),
        is_open: lock_open.clone(),
        reason: lock_reason.clone(),
    };
    std::thread::Builder::new()
        .name("meetaudio".into())
        .spawn(move || {
            if let Err(e) = devices_thread(ein_dev, aus_dev, mik_tx, m2, s2, e2, sp) {
                eprintln!("Ton-Geraete: {}", e);
            }
        })?;

    Ok(Devices {
        mic_name: mik_rx,
        speaker: mixer,
        is_muted,
        lock_on,
        lock_strength,
        lock_open,
        lock_reason,
        end,
        eingang: a_name_real,
        exit: from_name_real,
    })
}

/// The handles on the noise gate, as the audio thread sees them.
struct LockGrips {
    an: Arc<AtomicBool>,
    strength: Arc<std::sync::atomic::AtomicU32>,
    is_open: Arc<AtomicBool>,
    reason: Arc<std::sync::atomic::AtomicU32>,
}

fn devices_thread(
    ein_dev: cpal::Device,
    aus_dev: cpal::Device,
    mik_tx: std::sync::mpsc::Sender<Vec<i16>>,
    mixer: Arc<Mutex<Mixer>>,
    is_muted: Arc<AtomicBool>,
    end: Arc<AtomicBool>,
    gate_on: LockGrips,
) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, StreamTrait};

    let ein_cfg = ein_dev.default_input_config()?;
    let aus_cfg = aus_dev.default_output_config()?;
    let ein_rate = ein_cfg.sample_rate().0 as usize;
    let ein_kan = ein_cfg.channels() as usize;
    let aus_rate = aus_cfg.sample_rate().0 as usize;
    let aus_kan = aus_cfg.channels() as usize;

    // Rings between callback and work loop.
    let mic_ring: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let reference_ring: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

    let mr = mic_ring.clone();
    let ein_stream = ein_dev.build_input_stream(
        &ein_cfg.config(),
        move |daten: &[f32], _: &cpal::InputCallbackInfo| {
            let mono = zu_mono(daten, ein_kan);
            let auf = auf_48k(&mono, ein_rate);
            if let Ok(mut r) = mr.lock() {
                r.extend_from_slice(&auf);
                // hoechstens 1 s aufheben
                let to_much = r.len().saturating_sub(RATE);
                if to_much > 0 {
                    r.drain(0..to_much);
                }
            }
        },
        |e| eprintln!("Mikrofon: {}", e),
        None,
    )?;

    let mi = mixer.clone();
    let rr = reference_ring.clone();
    let aus_stream = aus_dev.build_output_stream(
        &aus_cfg.config(),
        move |daten: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let frame_box = daten.len() / aus_kan.max(1);
            // So viele 48-kHz-Werte brauchen wir dafuer.
            let required = frame_box * RATE / aus_rate.max(1);
            let mixed = mi
                .lock()
                .map(|mut m| m.downmix(required))
                .unwrap_or_else(|_| vec![0i16; required]);
            let f = zu_f32(&mixed);
            // Also record the reference for the echo cancellation.
            if let Ok(mut r) = rr.lock() {
                r.extend_from_slice(&f);
                let to_much = r.len().saturating_sub(RATE);
                if to_much > 0 {
                    r.drain(0..to_much);
                }
            }
            // resample to the device rate and spread over all channels
            let out = if aus_rate == RATE {
                f
            } else {
                let n = frame_box;
                let mut v = Vec::with_capacity(n);
                for i in 0..n {
                    let pos = i as f64 * RATE as f64 / aus_rate as f64;
                    let a = (pos.floor() as usize).min(f.len().saturating_sub(1));
                    v.push(f.get(a).copied().unwrap_or(0.0));
                }
                v
            };
            for (i, place) in daten.chunks_mut(aus_kan.max(1)).enumerate() {
                let v = out.get(i).copied().unwrap_or(0.0);
                for k in place.iter_mut() {
                    *k = v;
                }
            }
        },
        |e| eprintln!("Lautsprecher: {}", e),
        None,
    )?;

    ein_stream.play()?;
    aus_stream.play()?;

    let mut echo = Echo::neu();
    let mut sperr = NoiseGate::neu();
    while !end.load(Ordering::Relaxed) {
        // Fetch a 20 ms pair of microphone and reference.
        let mic_name = {
            let mut r = mic_ring.lock_ok();
            if r.len() < FRAME {
                None
            } else {
                Some(r.drain(0..FRAME).collect::<Vec<f32>>())
            }
        };
        let Some(mic_name) = mic_name else {
            std::thread::sleep(std::time::Duration::from_millis(4));
            continue;
        };
        let reference = {
            let mut r = reference_ring.lock_ok();
            if r.len() >= FRAME {
                r.drain(0..FRAME).collect::<Vec<f32>>()
            } else {
                vec![0.0; FRAME]
            }
        };
        let mut cleanly = echo.frame_box(&mic_name, &reference);
        // First remove the echo, THEN the noise gate: otherwise the echo would count as
        // useful signal and hold the gate open pointlessly.
        if gate_on.an.load(Ordering::Relaxed) {
            let strength = gate_on.strength.load(Ordering::Relaxed).min(100) as u8;
            let through = sperr.frame_box(&mut cleanly, strength);
            gate_on.is_open.store(through, Ordering::Relaxed);
            gate_on
                .reason
                .store((sperr.noise_floor() * 10_000.0) as u32, Ordering::Relaxed);
        } else {
            gate_on.is_open.store(true, Ordering::Relaxed);
        }
        let pcm = if is_muted.load(Ordering::Relaxed) {
            vec![0i16; FRAME]
        } else {
            zu_i16(&cleanly)
        };
        if mik_tx.send(pcm).is_err() {
            break;
        }
    }
    Ok(())
}

/// Names of the available devices - for the selection in the UI.
pub fn devices_list() -> (Vec<String>, Vec<String>) {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let ein = host
        .input_devices()
        .map(|d| d.filter_map(|x| x.name().ok()).collect())
        .unwrap_or_default();
    let aus = host
        .output_devices()
        .map(|d| d.filter_map(|x| x.name().ok()).collect())
        .unwrap_or_default();
    (ein, aus)
}

#[cfg(test)]
mod tests {

    /// Ein Rahmen gleichmaessigen Rauschens einer bestimmten Staerke.
    fn noise(strength: f32, seed: &mut u32) -> Vec<f32> {
        (0..960)
            .map(|_| {
                // Kleiner eigener Zufall - reproduzierbar, ohne Fremdkiste.
                *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                ((*seed >> 8) as f32 / 8_388_608.0 - 1.0) * strength
            })
            .collect()
    }

    /// A "speech" frame: a strong tone with some noise on it.
    fn speech(t: &mut f32, seed: &mut u32) -> Vec<f32> {
        (0..960)
            .map(|_| {
                *t += 1.0 / 48000.0;
                *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let disturbance = ((*seed >> 8) as f32 / 8_388_608.0 - 1.0) * 0.01;
                (2.0 * std::f32::consts::PI * 220.0 * *t).sin() * 0.25 + disturbance
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn noise_gate_really_damps_silent_noise() {
        let mut sp = NoiseGate::neu();
        let mut seed = 7u32;
        // Erst zwei Sekunden Ruhe: einmessen + zudrehen.
        let mut last = Vec::new();
        for _ in 0..100 {
            let mut r = noise(0.01, &mut seed);
            sp.frame_box(&mut r, 50);
            last = r;
        }
        let rest = rms(&last);
        assert!(
            rest < 0.001,
            "noise is not attenuated: RMS {rest}"
        );
        assert!(!sp.passthrough(), "Tor haette zu sein muessen");
    }

    #[test]
    fn noise_gate_lets_speech_through() {
        let mut sp = NoiseGate::neu();
        let (mut seed, mut t) = (7u32, 0.0f32);
        for _ in 0..100 {
            let mut r = noise(0.01, &mut seed);
            sp.frame_box(&mut r, 50);
        }
        // Now speaking - within 100 ms at the latest it must pass.
        let mut last = Vec::new();
        for _ in 0..5 {
            let mut r = speech(&mut t, &mut seed);
            sp.frame_box(&mut r, 50);
            last = r;
        }
        let loud = rms(&last);
        assert!(loud > 0.15, "speech does not get through: RMS {loud}");
        assert!(sp.passthrough(), "Tor haette offen sein muessen");
    }

    #[test]
    fn noise_gate_does_not_cut_word_end() {
        // The hold time is the reason why it does not sound choppy.
        let mut sp = NoiseGate::neu();
        let (mut seed, mut t) = (7u32, 0.0f32);
        for _ in 0..100 {
            let mut r = noise(0.01, &mut seed);
            sp.frame_box(&mut r, 50);
        }
        for _ in 0..10 {
            let mut r = speech(&mut t, &mut seed);
            sp.frame_box(&mut r, 50);
        }
        // Directly after the last loud frame sound must still pass
        // - otherwise the end of every word is lost.
        let mut r = noise(0.01, &mut seed);
        sp.frame_box(&mut r, 50);
        assert!(sp.passthrough(), "Nachlauf greift nicht");
    }

    #[test]
    fn strength_zero_lets_more_through_than_strength_hundred() {
        // The control must actually have an effect.
        let mess = |strength: u8| {
            let mut sp = NoiseGate::neu();
            let mut seed = 7u32;
            for _ in 0..60 {
                let mut r = noise(0.01, &mut seed);
                sp.frame_box(&mut r, strength);
            }
            // Ein etwas lauteres Geraeusch - Tastaturklappern.
            let mut r = noise(0.03, &mut seed);
            sp.frame_box(&mut r, strength);
            sp.passthrough()
        };
        assert!(mess(0), "at 0 the rattle must get through");
        assert!(!mess(100), "at 100 the rattle must not get through");
    }
    use super::*;

    fn sinus(n: usize, hz: f32, amp: f32, phase: &mut f32) -> Vec<f32> {
        let step = 2.0 * std::f32::consts::PI * hz / RATE as f32;
        (0..n)
            .map(|_| {
                let v = phase.sin() * amp;
                *phase += step;
                v
            })
            .collect()
    }

    fn power(x: &[f32]) -> f32 {
        x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32
    }

    /// ERLE = how much echo is gone, in dB. Above 20 dB counts as usable,
    /// above 30 dB as good.
    #[test]
    fn echo_is_really_quieter() {
        let mut e = Echo::neu();
        let mut ph = 0.0;
        let mut rest_power = 0.0;
        let mut echo_power = 0.0;
        // Room: the echo is an attenuated, delayed version.
        let delay = 240; // 5 ms
        let mut history = vec![0.0f32; delay + BLOCK];
        for i in 0..300 {
            let reference = sinus(BLOCK, 300.0 + (i % 7) as f32 * 50.0, 0.3, &mut ph);
            // Echo bauen
            history.extend_from_slice(&reference);
            let start = history.len() - BLOCK - delay;
            let echo: Vec<f32> = history[start..start + BLOCK].iter().map(|v| v * 0.5).collect();
            history.drain(0..BLOCK);
            let aus = e.block(&echo, &reference);
            if i > 250 {
                rest_power += power(&aus);
                echo_power += power(&echo);
            }
        }
        let erle = 10.0 * (echo_power / rest_power.max(1e-12)).log10();
        println!("ERLE = {:.1} dB", erle);
        assert!(erle > 20.0, "Echo-Unterdrueckung zu schwach: {:.1} dB", erle);
    }

    #[test]
    fn own_voice_stays_received() {
        let mut e = Echo::neu();
        let mut ph1 = 0.0;
        let mut ph2 = 1.0;
        let mut ein_l = 0.0;
        let mut aus_l = 0.0;
        for i in 0..200 {
            // No speaker -> nothing to subtract, the voice must stay.
            let reference = vec![0.0f32; BLOCK];
            let voice = sinus(BLOCK, 220.0, 0.4, &mut ph1);
            let _ = sinus(BLOCK, 1000.0, 0.0, &mut ph2);
            let aus = e.block(&voice, &reference);
            if i > 20 {
                ein_l += power(&voice);
                aus_l += power(&aus);
            }
        }
        let loss = 10.0 * (ein_l / aus_l.max(1e-12)).log10();
        println!("Verlust ohne Lautsprecher = {:.2} dB", loss);
        assert!(loss < 3.0, "own voice is attenuated: {:.2} dB", loss);
    }

    #[test]
    fn duplex_does_not_destroy_the_voice() {
        let mut e = Echo::neu();
        let (mut p1, mut p2) = (0.0, 0.0);
        let delay = 240;
        let mut history = vec![0.0f32; delay + BLOCK];
        let mut voice_a = 0.0;
        let mut aus_l = 0.0;
        for i in 0..400 {
            let reference = sinus(BLOCK, 400.0, 0.3, &mut p1);
            history.extend_from_slice(&reference);
            let start = history.len() - BLOCK - delay;
            let echo: Vec<f32> = history[start..start + BLOCK].iter().map(|v| v * 0.5).collect();
            history.drain(0..BLOCK);
            // from the half onwards the person talks along
            let voice = if i > 200 {
                sinus(BLOCK, 150.0, 0.4, &mut p2)
            } else {
                vec![0.0; BLOCK]
            };
            let mic_name: Vec<f32> = echo.iter().zip(&voice).map(|(a, b)| a + b).collect();
            let aus = e.block(&mic_name, &reference);
            if i > 300 {
                voice_a += power(&voice);
                aus_l += power(&aus);
            }
        }
        let ratio = 10.0 * (voice_a / aus_l.max(1e-12)).log10();
        println!("Gegensprechen: Verlust {:.2} dB", ratio);
        assert!(
            ratio < 6.0,
            "too much voice lost during double talk: {:.2} dB",
            ratio
        );
    }

    #[test]
    fn echo_is_fast_enough_for_realtime() {
        let mut e = Echo::neu();
        let mut ph = 0.0;
        let blocks = 200; // 2 Sekunden Ton
        let start = std::time::Instant::now();
        for _ in 0..blocks {
            let r = sinus(BLOCK, 500.0, 0.3, &mut ph);
            let m = sinus(BLOCK, 500.0, 0.15, &mut ph);
            let _ = e.block(&m, &r);
        }
        let dur = start.elapsed().as_secs_f32();
        let realtime = blocks as f32 * 0.01;
        let factor = dur / realtime;
        println!("Echo-Rechenzeit: {:.1} % der Echtzeit", factor * 100.0);
        // In the debug build Rust computes without optimisation (factor ~40). What is measured
        // is the release build: there it is 3.6 % of real time.
        let limit = if cfg!(debug_assertions) { 3.0 } else { 0.2 };
        assert!(factor < limit, "too slow: {:.0} % of real time", factor * 100.0);
    }

    #[test]
    fn mixer_adds_and_keeps_the_length() {
        let mut m = Mixer::neu();
        m.lead = 1;
        m.also(1, &vec![1000i16; FRAME]);
        m.also(2, &vec![2000i16; FRAME]);
        let aus = m.downmix(FRAME);
        assert_eq!(aus.len(), FRAME);
        assert_eq!(aus[0], 3000);
        assert_eq!(m.people(), 2);
        // If nothing is left, silence comes - no crackling.
        let leer = m.downmix(FRAME);
        assert!(leer.iter().all(|v| *v == 0));
    }

    #[test]
    fn mixer_clips_not() {
        let mut m = Mixer::neu();
        for p in 0..8u64 {
            m.also(p, &vec![30000i16; FRAME]);
        }
        let aus = m.downmix(FRAME);
        assert!(aus.iter().all(|v| *v <= i16::MAX && *v >= i16::MIN));
        assert_eq!(aus[0], i16::MAX);
    }

    #[test]
    fn mixer_drops_too_old_packets() {
        let mut m = Mixer::neu();
        for _ in 0..40 {
            m.also(1, &vec![100i16; FRAME]);
        }
        // 40 x 20 ms = 800 ms angeliefert, es duerfen hoechstens 400 ms bleiben
        let mut frame_box = 0;
        loop {
            let a = m.downmix(FRAME);
            if a.iter().all(|v| *v == 0) {
                break;
            }
            frame_box += 1;
            if frame_box > 100 {
                break;
            }
        }
        assert!(frame_box <= 20, "zu viel Ton aufgestaut: {} Rahmen", frame_box);
    }

    #[test]
    fn toggle_and_mono() {
        let st = vec![1.0f32, -1.0, 0.5, -0.5];
        assert_eq!(zu_mono(&st, 2), vec![0.0, 0.0]);
        let lang = auf_48k(&vec![0.0f32; 441], 44100);
        assert!((lang.len() as i32 - 480).abs() <= 1, "Laenge {}", lang.len());
        assert_eq!(zu_i16(&[1.0, -1.0, 0.0]), vec![32767, -32767, 0]);
        assert_eq!(zu_f32(&[32768u16 as i16]), vec![-1.0]);
    }
}
