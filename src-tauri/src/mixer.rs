//! The sound page's sound, played by the app itself. LIVE Studio's Application source captures a
//! program by its `.exe`, and a web view plays from WebView2's processes, never `Overtok.exe`: so
//! the sound page plays to no device and hands its samples here (`app_audio`), and the app plays
//! them on the computer's output. Each audio context on the page is one stream (stereo, at its own
//! rate); this mixes them onto the device, at the device's rate and channels.

use std::collections::{HashMap, VecDeque};

/// Frames a stream holds before it starts (and after it ran dry): the page hands its sound over in
/// chunks of 1024 frames (about 21 ms), so two absorb the gaps between them.
pub const PREFILL: usize = 2048;
/// The most a stream may run behind (frames, about 0.5 s at 48 kHz): older sound is dropped, so a
/// stall never leaves every later sound late.
pub const MAX_QUEUED: usize = 24_000;

#[derive(Default)]
struct Stream {
    /// Stereo frames, interleaved (left, right).
    queue: VecDeque<f32>,
    rate: u32,
    playing: bool,
    /// Where between the first two queued frames the next output frame falls (0..1).
    pos: f64,
}

impl Stream {
    fn frames(&self) -> usize {
        self.queue.len() / 2
    }
}

#[derive(Default)]
pub struct Mixer {
    streams: HashMap<u32, Stream>,
}

impl Mixer {
    /// Queues a chunk of stereo frames (interleaved) from one stream, made at `rate` Hz.
    pub fn push(&mut self, id: u32, rate: u32, samples: &[f32]) {
        if rate == 0 || samples.len() < 2 {
            return;
        }
        let stream = self.streams.entry(id).or_default();
        stream.rate = rate;
        let frames = samples.len() / 2;
        stream.queue.extend(samples[..frames * 2].iter().map(|s| s.clamp(-1.0, 1.0)));
        let over = stream.frames().saturating_sub(MAX_QUEUED);
        if over > 0 {
            stream.queue.drain(..over * 2);
            stream.pos = 0.0;
        }
    }

    /// Fills `out` (interleaved, `channels` per frame, at `rate` Hz) with every stream mixed.
    pub fn fill(&mut self, out: &mut [f32], channels: usize, rate: u32) {
        out.fill(0.0);
        if channels == 0 || rate == 0 {
            return;
        }
        let frames = out.len() / channels;
        for stream in self.streams.values_mut() {
            if !stream.playing && stream.frames() >= PREFILL {
                stream.playing = true;
                stream.pos = 0.0;
            }
            if !stream.playing {
                continue;
            }
            let step = f64::from(stream.rate) / f64::from(rate);
            for frame in 0..frames {
                // Two frames to blend between; one left is the last sample of a sound.
                if stream.frames() < 2 {
                    if stream.frames() == 1 && stream.pos == 0.0 {
                        let (l, r) = (stream.queue[0], stream.queue[1]);
                        write(out, frame, channels, l, r);
                        stream.queue.clear();
                    }
                    stream.playing = false;
                    stream.pos = 0.0;
                    break;
                }
                let t = stream.pos as f32;
                let l = stream.queue[0] + (stream.queue[2] - stream.queue[0]) * t;
                let r = stream.queue[1] + (stream.queue[3] - stream.queue[1]) * t;
                write(out, frame, channels, l, r);
                stream.pos += step;
                while stream.pos >= 1.0 && stream.frames() >= 1 {
                    stream.queue.pop_front();
                    stream.queue.pop_front();
                    stream.pos -= 1.0;
                }
            }
        }
        self.streams.retain(|_, stream| stream.playing || !stream.queue.is_empty());
        for sample in out.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }
}

/// Adds one stereo frame to the device's: mono gets both sides' mean, more channels the first two.
fn write(out: &mut [f32], frame: usize, channels: usize, l: f32, r: f32) {
    let at = frame * channels;
    if channels == 1 {
        out[at] += (l + r) / 2.0;
    } else {
        out[at] += l;
        out[at + 1] += r;
    }
}

/// A chunk as the page sends it: little-endian 32-bit floats, stereo frames interleaved.
pub fn samples_of(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .filter(|s| s.is_finite())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(frames: usize, value: f32) -> Vec<f32> {
        std::iter::repeat([value, -value]).take(frames).flatten().collect()
    }

    #[test]
    fn waits_for_two_chunks_then_plays_them_in_order() {
        let mut mixer = Mixer::default();
        mixer.push(1, 48_000, &ramp(1024, 0.5));
        let mut out = vec![0.0; 256 * 2];
        mixer.fill(&mut out, 2, 48_000);
        assert!(out.iter().all(|s| *s == 0.0), "one chunk isn't enough to start");
        mixer.push(1, 48_000, &ramp(1024, 0.5));
        mixer.fill(&mut out, 2, 48_000);
        assert_eq!(&out[..4], &[0.5, -0.5, 0.5, -0.5]);
    }

    #[test]
    fn mixes_streams_and_keeps_the_sum_in_range() {
        let mut mixer = Mixer::default();
        mixer.push(1, 48_000, &ramp(PREFILL, 0.25));
        mixer.push(2, 48_000, &ramp(PREFILL, 0.5));
        mixer.push(3, 48_000, &ramp(PREFILL, 0.75));
        let mut out = vec![0.0; 4 * 2];
        mixer.fill(&mut out, 2, 48_000);
        assert_eq!(&out[..2], &[1.0, -1.0]);
        let mut out = vec![0.0; 4 * 2];
        let mut quiet = Mixer::default();
        quiet.push(1, 48_000, &ramp(PREFILL, 0.25));
        quiet.push(2, 48_000, &ramp(PREFILL, 0.25));
        quiet.fill(&mut out, 2, 48_000);
        assert_eq!(&out[..2], &[0.5, -0.5]);
    }

    #[test]
    fn fits_the_device_channels() {
        let mut mixer = Mixer::default();
        mixer.push(1, 48_000, &[0.2, 0.6].repeat(PREFILL));
        let mut mono = vec![0.0; 2];
        mixer.fill(&mut mono, 1, 48_000);
        assert!((mono[0] - 0.4).abs() < 1e-6);
        let mut six = vec![0.0; 6];
        mixer.fill(&mut six, 6, 48_000);
        assert_eq!(six, vec![0.2, 0.6, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn plays_a_44_1_khz_page_on_a_48_khz_device_at_its_own_speed() {
        let mut mixer = Mixer::default();
        mixer.push(1, 44_100, &ramp(44_100 / 10, 0.5));
        // A tenth of a second of the page's sound is a tenth of a second on the device.
        let mut out = vec![0.0; 4800 * 2];
        mixer.fill(&mut out, 2, 48_000);
        let heard = out.chunks(2).filter(|f| f[0] != 0.0).count();
        assert!((4790..=4800).contains(&heard), "heard {heard} frames");
    }

    #[test]
    fn drops_what_runs_too_far_behind() {
        let mut mixer = Mixer::default();
        mixer.push(1, 48_000, &ramp(MAX_QUEUED + 5000, 0.5));
        assert_eq!(mixer.streams[&1].frames(), MAX_QUEUED);
    }

    #[test]
    fn forgets_a_stream_once_it_ran_dry() {
        let mut mixer = Mixer::default();
        mixer.push(7, 48_000, &ramp(PREFILL, 0.5));
        let mut out = vec![0.0; (PREFILL + 10) * 2];
        mixer.fill(&mut out, 2, 48_000);
        assert!(mixer.streams.is_empty());
    }

    #[test]
    fn reads_the_pages_bytes() {
        let bytes: Vec<u8> = [0.5f32, -0.25].iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(samples_of(&bytes), vec![0.5, -0.25]);
        assert_eq!(samples_of(&f32::NAN.to_le_bytes()), Vec::<f32>::new());
    }
}
