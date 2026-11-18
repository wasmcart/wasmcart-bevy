//! A small software mixer that feeds the wasmcart audio ring.
//!
//! `bevy_audio` plays through rodio and cpal, which reach the OS (or the
//! browser's Web Audio) directly; a cart has neither. Instead the cart mixes
//! its own samples and the host plays them: each frame the mixer renders
//! exactly the stereo frames the elapsed `delta_ms` covers at the cart's
//! sample rate into the ring (wasmcart SPEC, "Audio").
//!
//! ```ignore
//! fn setup(mut commands: Commands) {
//!     commands.insert_resource(Sounds { jump: AudioClip::tone(660.0, 0.12, Wave::Square) });
//! }
//! fn jump(audio: Res<Audio>, sounds: Res<Sounds>) {
//!     audio.play(&sounds.jump);
//! }
//! ```
//!
//! WAV files (PCM 8/16/24/32-bit or float, any rate, mono or stereo) load with
//! [`AudioClip::from_wav`], or through the asset server with the `assets`
//! feature.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::abi;

/// Decoded audio: interleaved stereo `f32` at the cart's mix rate.
#[derive(Clone, Debug, TypePath)]
#[cfg_attr(feature = "assets", derive(Asset))]
pub struct AudioClip {
    samples: Arc<[f32]>,
}

/// Oscillator shape for [`AudioClip::tone`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    /// Pure tone.
    Sine,
    /// 50% pulse, chiptune lead.
    Square,
    /// Bright, buzzy.
    Saw,
    /// Soft, flute-like.
    Triangle,
    /// White noise (frequency ignored), for hits and explosions.
    Noise,
}

fn mix_rate() -> u32 {
    let r = abi::config().audio_rate;
    if r == 0 { 48000 } else { r }
}

impl AudioClip {
    /// Wrap interleaved stereo samples already at the mix rate.
    pub fn from_stereo(samples: Vec<f32>) -> Self {
        Self { samples: samples.into() }
    }

    /// Length in stereo frames.
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// Synthesize a tone of `freq` Hz lasting `secs`, with a short attack and
    /// an exponential decay so it never clicks.
    pub fn tone(freq: f32, secs: f32, wave: Wave) -> Self {
        Self::sweep(freq, freq, secs, wave)
    }

    /// Like [`tone`](Self::tone) but gliding from `f0` to `f1` Hz.
    pub fn sweep(f0: f32, f1: f32, secs: f32, wave: Wave) -> Self {
        let rate = mix_rate() as f32;
        let n = (secs * rate) as usize;
        let mut out = Vec::with_capacity(n * 2);
        let mut phase = 0.0f32;
        let mut seed = 0x1234_5678u32;
        for i in 0..n {
            let t = i as f32 / n.max(1) as f32;
            let f = f0 + (f1 - f0) * t;
            phase = (phase + f / rate).fract();
            let s = match wave {
                Wave::Sine => (phase * core::f32::consts::TAU).sin(),
                Wave::Square => if phase < 0.5 { 1.0 } else { -1.0 },
                Wave::Saw => phase * 2.0 - 1.0,
                Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
                Wave::Noise => {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
                }
            };
            let attack = (i as f32 / (rate * 0.005)).min(1.0);
            let env = attack * (-4.0 * t).exp() * (1.0 - t).min(1.0);
            let v = s * env * 0.5;
            out.push(v);
            out.push(v);
        }
        Self::from_stereo(out)
    }

    /// Decode a RIFF WAV file and resample it to the mix rate.
    pub fn from_wav(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err("not a RIFF WAVE file");
        }
        let (mut fmt, mut data) = (None, None);
        let mut p = 12;
        while p + 8 <= bytes.len() {
            let id = &bytes[p..p + 4];
            let len = u32::from_le_bytes(bytes[p + 4..p + 8].try_into().unwrap()) as usize;
            let body = &bytes[p + 8..(p + 8 + len).min(bytes.len())];
            match id {
                b"fmt " => fmt = Some(body),
                b"data" => data = Some(body),
                _ => {}
            }
            p += 8 + len + (len & 1);
        }
        let fmt = fmt.ok_or("no fmt chunk")?;
        let data = data.ok_or("no data chunk")?;
        if fmt.len() < 16 {
            return Err("short fmt chunk");
        }
        let format = u16::from_le_bytes([fmt[0], fmt[1]]);
        let channels = u16::from_le_bytes([fmt[2], fmt[3]]) as usize;
        let rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
        let bits = u16::from_le_bytes([fmt[14], fmt[15]]);
        let format = if format == 0xFFFE && fmt.len() >= 26 {
            u16::from_le_bytes([fmt[24], fmt[25]])
        } else {
            format
        };
        if channels == 0 || rate == 0 {
            return Err("bad channel count or rate");
        }
        let bps = (bits as usize).div_ceil(8);
        let frame_bytes = bps * channels;
        let frames = data.len() / frame_bytes;
        let read = |i: usize| -> f32 {
            let s = &data[i * bps..i * bps + bps];
            match (format, bits) {
                (1, 8) => (s[0] as f32 - 128.0) / 128.0,
                (1, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
                (1, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
                (1, 32) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
                (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                _ => 0.0,
            }
        };
        if !matches!((format, bits), (1, 8) | (1, 16) | (1, 24) | (1, 32) | (3, 32)) {
            return Err("unsupported WAV sample format");
        }
        let src: Vec<[f32; 2]> = (0..frames)
            .map(|f| {
                let l = read(f * channels);
                let r = if channels > 1 { read(f * channels + 1) } else { l };
                [l, r]
            })
            .collect();
        let out_rate = mix_rate();
        let out_frames = (frames as u64 * out_rate as u64 / rate as u64) as usize;
        let mut out = Vec::with_capacity(out_frames * 2);
        for i in 0..out_frames {
            let pos = i as f64 * rate as f64 / out_rate as f64;
            let a = pos as usize;
            let t = (pos - a as f64) as f32;
            let s0 = src[a.min(frames - 1)];
            let s1 = src[(a + 1).min(frames - 1)];
            out.push(s0[0] + (s1[0] - s0[0]) * t);
            out.push(s0[1] + (s1[1] - s0[1]) * t);
        }
        Ok(Self::from_stereo(out))
    }
}

/// How a clip plays.
#[derive(Clone, Copy, Debug)]
pub struct PlaySettings {
    /// Linear gain, 1.0 = as recorded.
    pub volume: f32,
    /// -1.0 = left, 0.0 = centre, 1.0 = right.
    pub pan: f32,
    /// Playback rate, 1.0 = as recorded (also shifts pitch).
    pub speed: f32,
    /// Restart at the end instead of stopping.
    pub looped: bool,
}

impl Default for PlaySettings {
    fn default() -> Self {
        Self { volume: 1.0, pan: 0.0, speed: 1.0, looped: false }
    }
}

/// Identifies a playing voice, to stop it or change its volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoiceId(u64);

struct Voice {
    id: u64,
    clip: AudioClip,
    pos: f64,
    settings: PlaySettings,
}

#[derive(Default)]
struct Mixer {
    voices: Vec<Voice>,
    next_id: u64,
    master: f32,
}

/// The cart's mixer. `Res<Audio>` is enough to play: the methods take `&self`.
#[derive(Resource, Clone)]
pub struct Audio {
    inner: Arc<Mutex<Mixer>>,
}

impl Default for Audio {
    fn default() -> Self {
        Self { inner: Arc::new(Mutex::new(Mixer { master: 1.0, ..default() })) }
    }
}

impl Audio {
    /// Play `clip` once at full volume.
    pub fn play(&self, clip: &AudioClip) -> VoiceId {
        self.play_with(clip, PlaySettings::default())
    }
    /// Play `clip` with explicit settings.
    pub fn play_with(&self, clip: &AudioClip, settings: PlaySettings) -> VoiceId {
        let mut m = self.inner.lock().unwrap();
        m.next_id += 1;
        let id = m.next_id;
        m.voices.push(Voice { id, clip: clip.clone(), pos: 0.0, settings });
        VoiceId(id)
    }
    /// Stop a voice (no-op if it already ended).
    pub fn stop(&self, id: VoiceId) {
        self.inner.lock().unwrap().voices.retain(|v| v.id != id.0);
    }
    /// Stop everything.
    pub fn stop_all(&self) {
        self.inner.lock().unwrap().voices.clear();
    }
    /// Change a playing voice's volume.
    pub fn set_volume(&self, id: VoiceId, volume: f32) {
        if let Some(v) = self.inner.lock().unwrap().voices.iter_mut().find(|v| v.id == id.0) {
            v.settings.volume = volume;
        }
    }
    /// Master gain applied to the whole mix.
    pub fn set_master_volume(&self, volume: f32) {
        self.inner.lock().unwrap().master = volume;
    }
    /// True if the voice is still playing.
    pub fn is_playing(&self, id: VoiceId) -> bool {
        self.inner.lock().unwrap().voices.iter().any(|v| v.id == id.0)
    }
    /// Number of voices playing.
    pub fn voices(&self) -> usize {
        self.inner.lock().unwrap().voices.len()
    }
}

#[derive(Resource, Default)]
struct MixClock {
    owed: f64,
}

/// Adds [`Audio`] and the mixer system. Part of [`WasmcartPlugin`](crate::WasmcartPlugin).
pub struct WasmcartAudioPlugin;

impl Plugin for WasmcartAudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Audio>().init_resource::<MixClock>();
        #[cfg(feature = "assets")]
        {
            app.init_asset::<AudioClip>().register_asset_loader(WavLoader);
        }
        if abi::config().audio_frames > 0 {
            app.add_systems(Last, mix_system);
        }
    }
}

fn mix_system(audio: Res<Audio>, mut clock: ResMut<MixClock>) {
    let t = abi::time();
    let dt = if t.delta_ms.is_finite() { t.delta_ms.clamp(0.0, 250.0) } else { 0.0 };
    clock.owed += dt * mix_rate() as f64 / 1000.0;
    let frames = clock.owed as usize;
    clock.owed -= frames as f64;
    if frames == 0 {
        return;
    }
    let mut m = audio.inner.lock().unwrap();
    let master = m.master;
    let voices = &mut m.voices;
    abi::audio_write(frames, |out| {
        out.fill(0.0);
        for v in voices.iter_mut() {
            let s = &v.clip.samples;
            let len = s.len() / 2;
            if len == 0 {
                v.pos = f64::INFINITY;
                continue;
            }
            let pan = v.settings.pan.clamp(-1.0, 1.0);
            let gl = v.settings.volume * (1.0 - pan).min(1.0);
            let gr = v.settings.volume * (1.0 + pan).min(1.0);
            for frame in out.chunks_exact_mut(2) {
                let mut i = v.pos as usize;
                if i >= len {
                    if v.settings.looped {
                        v.pos -= len as f64;
                        i = v.pos as usize;
                    } else {
                        break;
                    }
                }
                frame[0] += s[i * 2] * gl;
                frame[1] += s[i * 2 + 1] * gr;
                v.pos += v.settings.speed as f64;
            }
        }
        for x in out.iter_mut() {
            // soft clip so stacked voices saturate instead of wrapping
            *x = (*x * master).tanh();
        }
    });
    voices.retain(|v| v.settings.looped || (v.pos as usize) < v.clip.samples.len() / 2);
}

#[cfg(feature = "assets")]
#[derive(TypePath)]
struct WavLoader;

#[cfg(feature = "assets")]
impl bevy::asset::AssetLoader for WavLoader {
    type Asset = AudioClip;
    type Settings = ();
    type Error = std::io::Error;
    async fn load(
        &self,
        reader: &mut dyn bevy::asset::io::Reader,
        _settings: &(),
        _ctx: &mut bevy::asset::LoadContext<'_>,
    ) -> Result<AudioClip, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        AudioClip::from_wav(&bytes).map_err(std::io::Error::other)
    }
    fn extensions(&self) -> &[&str] {
        &["wav"]
    }
}
