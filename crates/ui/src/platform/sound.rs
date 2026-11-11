//! The app's two sounds: a message arrived, a message left. Both are made
//! here from sine waves (a few hundred bytes of code instead of sound files
//! and something to decode them) and played by what each system has for
//! it: `PlaySoundW` on Windows, `afplay` on macOS, and on Linux the player
//! of whichever sound system is there (`paplay`, `aplay`, `pw-play`).

use std::time::{Duration, Instant};

/// Samples per second.
const RATE: u32 = 44_100;

/// One of the app's sounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// A message arrived: two soft notes, the second higher.
    Arrived,
    /// A message of the user's own left: one short rising note.
    Sent,
}

impl Sound {
    /// Its notes: for each, where its pitch starts and ends (Hz), how long
    /// it sounds and how fast it dies away (seconds), and how loud it is.
    fn notes(self) -> &'static [(f32, f32, f32, f32, f32)] {
        match self {
            Self::Arrived => &[(880.0, 880.0, 0.09, 0.05, 0.22), (1318.5, 1318.5, 0.26, 0.085, 0.24)],
            Self::Sent => &[(620.0, 930.0, 0.09, 0.03, 0.16)],
        }
    }

    /// How soon after itself it may be heard again: a burst of messages is one sound.
    fn rest(self) -> Duration {
        match self {
            Self::Arrived => Duration::from_millis(900),
            Self::Sent => Duration::from_millis(250),
        }
    }

    /// Its file's name, where it is played from a file.
    #[cfg_attr(windows, expect(dead_code, reason = "Windows plays it from memory"))]
    fn file(self) -> &'static str {
        match self {
            Self::Arrived => "adelite-arrived.wav",
            Self::Sent => "adelite-sent.wav",
        }
    }
}

/// The samples of `sound`: each note a sine with a little of its octave
/// over it (a bell rather than a beep), let in over a few milliseconds and
/// dying away, so nothing clicks.
fn samples(sound: Sound) -> Vec<i16> {
    let mut out = Vec::new();
    for &(from, to, length, decay, loudness) in sound.notes() {
        let count = (length * RATE as f32) as usize;
        let mut phase = 0.0_f32;
        for index in 0..count {
            let at = index as f32 / RATE as f32;
            let pitch = from + (to - from) * (index as f32 / count as f32);
            phase += std::f32::consts::TAU * pitch / RATE as f32;
            // In over four milliseconds, out towards nothing; the last few are pulled to silence.
            let tail = ((count - index) as f32 / (0.008 * RATE as f32)).min(1.0);
            let envelope = (at / 0.004).min(1.0) * (-at / decay).exp() * tail;
            let wave = phase.sin() + 0.18 * (2.0 * phase).sin();
            out.push((wave * envelope * loudness * f32::from(i16::MAX)) as i16);
        }
    }
    out
}

/// `sound` as a WAV file: one channel, sixteen bits.
#[must_use]
pub fn wav(sound: Sound) -> Vec<u8> {
    let samples = samples(sound);
    let bytes = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    // A format block of 16 bytes: PCM, one channel, the rate, bytes per second, bytes per sample, bits.
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    out.extend(samples.iter().flat_map(|sample| sample.to_le_bytes()));
    out
}

/// Plays the app's sounds. Made once: the sounds are kept for as long as it is.
#[derive(Debug)]
pub struct Sounds {
    /// Each sound as a WAV file in memory.
    arrived: Vec<u8>,
    sent: Vec<u8>,
    /// The sound heard last, and when.
    last: Option<(Sound, Instant)>,
}

impl Default for Sounds {
    fn default() -> Self {
        Self { arrived: wav(Sound::Arrived), sent: wav(Sound::Sent), last: None }
    }
}

impl Sounds {
    /// Plays `sound`, unless it was just heard. Never waits for it to end.
    pub fn play(&mut self, sound: Sound) {
        let now = Instant::now();
        if self.last.is_some_and(|(last, at)| last == sound && now.duration_since(at) < sound.rest()) {
            return;
        }
        self.last = Some((sound, now));
        let bytes = match sound {
            Sound::Arrived => &self.arrived,
            Sound::Sent => &self.sent,
        };
        #[cfg(windows)]
        {
            /// `SND_ASYNC | SND_NODEFAULT | SND_MEMORY`: from memory, without waiting, and no beep in its place.
            const FLAGS: u32 = 0x1 | 0x2 | 0x4;
            // SAFETY: with `SND_MEMORY` the first argument is a WAV file in memory, which `bytes`
            // is; it is kept by `self`, which stops the sound before it goes (`Drop`). No module.
            unsafe { PlaySoundW(bytes.as_ptr().cast(), std::ptr::null_mut(), FLAGS) };
        }
        #[cfg(not(windows))]
        {
            // Played from a file, written once: the players take no sound on their input.
            let path = std::env::temp_dir().join(sound.file());
            if !path.exists() && std::fs::write(&path, bytes).is_err() {
                return;
            }
            let players: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
                &[("afplay", &[])]
            } else {
                &[("paplay", &[]), ("aplay", &["-q"]), ("pw-play", &[])]
            };
            for (program, arguments) in players {
                let mut command = std::process::Command::new(program);
                command.args(*arguments).arg(&path);
                if super::launch(command).is_ok() {
                    break;
                }
            }
        }
    }
}

#[cfg(windows)]
impl Drop for Sounds {
    fn drop(&mut self) {
        // SAFETY: a null sound stops what is playing, so nothing reads the sounds after they are freed.
        unsafe { PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0) };
    }
}

#[cfg(windows)]
#[link(name = "winmm")]
unsafe extern "system" {
    fn PlaySoundW(sound: *const u16, module: *mut std::ffi::c_void, flags: u32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sounds_are_short_quiet_and_end_in_silence() {
        for sound in [Sound::Arrived, Sound::Sent] {
            let samples = samples(sound);
            let seconds = samples.len() as f32 / RATE as f32;
            assert!((0.05..0.5).contains(&seconds), "{sound:?} lasts {seconds} s");
            let peak = samples.iter().map(|sample| sample.unsigned_abs()).max().unwrap_or(0);
            assert!((1_500..12_000).contains(&peak), "{sound:?} peaks at {peak}: heard, and never loud");
            assert!(samples[0].abs() < 200 && samples[samples.len() - 1].abs() < 200, "{sound:?} would click");
            // A file any player takes: its header says what follows it.
            let file = wav(sound);
            assert_eq!((&file[..4], &file[8..16], &file[36..40]), (&b"RIFF"[..], &b"WAVEfmt "[..], &b"data"[..]));
            let said = u32::from_le_bytes([file[40], file[41], file[42], file[43]]) as usize;
            assert_eq!((said, file.len()), (samples.len() * 2, 44 + samples.len() * 2));
            assert_eq!(u32::from_le_bytes([file[4], file[5], file[6], file[7]]) as usize, file.len() - 8);
        }
        assert!(samples(Sound::Sent).len() < samples(Sound::Arrived).len(), "one's own is the shorter");
    }

    #[test]
    fn a_burst_is_heard_once() {
        let mut sounds = Sounds { arrived: Vec::new(), sent: Vec::new(), last: None };
        let now = Instant::now();
        sounds.last = Some((Sound::Arrived, now));
        let rested = |sounds: &Sounds, sound: Sound, at: Instant| {
            !sounds.last.is_some_and(|(last, then)| last == sound && at.duration_since(then) < sound.rest())
        };
        assert!(!rested(&sounds, Sound::Arrived, now + Duration::from_millis(300)));
        assert!(rested(&sounds, Sound::Arrived, now + Duration::from_secs(2)));
        assert!(rested(&sounds, Sound::Sent, now), "another sound is another matter");
    }
}
