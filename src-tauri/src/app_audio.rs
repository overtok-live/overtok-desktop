//! The sound window's sound, played by `Overtok.exe` itself (Windows). LIVE Studio's Application
//! source captures a program by its `.exe`, and a web view plays from WebView2's own processes
//! (`msedgewebview2.exe`), which it never takes: picking Overtok there stayed silent. So the sound
//! page plays to no device and hands its samples to the app (`app_audio`, `mixer.rs`), and the app
//! plays them on the computer's output while the sound window is on. That also keeps `Overtok.exe`
//! in LIVE Studio's list (it lists the programs with sound open). Elsewhere this does nothing,
//! unless `OVERTOK_AUDIO_TAP` is set on a Mac (to try it there).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::Thread;

use crate::mixer::Mixer;

/// Whether the app plays the sound page's sound itself (the page asks it to with `tap=1`).
pub fn tap_enabled() -> bool {
    cfg!(windows) || (cfg!(target_os = "macos") && std::env::var_os("OVERTOK_AUDIO_TAP").is_some())
}

/// The app's sound: one thread owns the device's stream (a stream may not leave its thread).
pub struct AppAudio {
    wanted: Arc<AtomicBool>,
    /// The device's stream is open: what the page hands over is heard.
    open: Arc<AtomicBool>,
    worker: Option<Thread>,
    mixer: Arc<Mutex<Mixer>>,
}

impl AppAudio {
    pub fn start() -> Self {
        let wanted = Arc::new(AtomicBool::new(false));
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        let open = Arc::new(AtomicBool::new(false));
        let worker = if tap_enabled() {
            spawn(wanted.clone(), open.clone(), mixer.clone())
        } else {
            None
        };
        Self { wanted, open, worker, mixer }
    }

    /// On while the sound window is on.
    pub fn set(&self, on: bool) {
        self.wanted.store(on, Ordering::SeqCst);
        if !on {
            *self.mixer.lock().unwrap() = Mixer::default();
        }
        if let Some(worker) = &self.worker {
            worker.unpark();
        }
    }

    /// A chunk of the sound page's sound (one audio context's stereo frames, at `rate` Hz):
    /// whether it's heard (false: no device open, and the page plays it itself instead).
    pub fn push(&self, stream: u32, rate: u32, samples: &[f32]) -> bool {
        if !self.wanted.load(Ordering::SeqCst) || !self.open.load(Ordering::SeqCst) {
            return false;
        }
        self.mixer.lock().unwrap().push(stream, rate, samples);
        true
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn spawn(wanted: Arc<AtomicBool>, open: Arc<AtomicBool>, mixer: Arc<Mutex<Mixer>>) -> Option<Thread> {
    use std::{thread, time::Duration};
    thread::Builder::new()
        .name("overtok-app-audio".into())
        .spawn(move || {
            let broken = Arc::new(AtomicBool::new(false));
            let mut sound: Option<device::Sound> = None;
            loop {
                if !wanted.load(Ordering::SeqCst) {
                    sound = None;
                } else if sound.is_none()
                    || broken.swap(false, Ordering::SeqCst)
                    || sound.as_ref().is_some_and(|s| s.moved())
                {
                    // A device unplugged, failed or another became the default: close it first,
                    // then open the one the computer plays on now.
                    drop(sound.take());
                    sound = device::open(mixer.clone(), broken.clone(), thread::current());
                }
                open.store(sound.is_some(), Ordering::SeqCst);
                // Asleep until switched; a look every few seconds for a new default device.
                let wait = if wanted.load(Ordering::SeqCst) { 3 } else { 60 };
                thread::park_timeout(Duration::from_secs(wait));
            }
        })
        .ok()
        .map(|handle| handle.thread().clone())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn spawn(
    _wanted: Arc<AtomicBool>,
    _open: Arc<AtomicBool>,
    _mixer: Arc<Mutex<Mixer>>,
) -> Option<Thread> {
    None
}

#[cfg(any(windows, target_os = "macos"))]
mod device {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::Thread;

    use crate::mixer::Mixer;

    /// The stream playing the mix, and the device it was opened on.
    pub struct Sound {
        _stream: Stream,
        name: Option<String>,
    }

    impl Sound {
        /// Whether the computer's default output is another device now.
        pub fn moved(&self) -> bool {
            let now = cpal::default_host().default_output_device().and_then(|d| name_of(&d));
            now.is_some() && now != self.name
        }
    }

    fn name_of(device: &Device) -> Option<String> {
        device.id().ok().map(|id| id.to_string())
    }

    /// The mix on the computer's output now, flagging `broken` (and waking `worker`) if it fails.
    pub fn open(mixer: Arc<Mutex<Mixer>>, broken: Arc<AtomicBool>, worker: Thread) -> Option<Sound> {
        let device = cpal::default_host().default_output_device()?;
        let supported = device.default_output_config().ok()?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let on_error = move |_| {
            broken.store(true, Ordering::SeqCst);
            worker.unpark();
        };
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, config, mixer, on_error),
            SampleFormat::I16 => build::<i16>(&device, config, mixer, on_error),
            SampleFormat::I32 => build::<i32>(&device, config, mixer, on_error),
            SampleFormat::U16 => build::<u16>(&device, config, mixer, on_error),
            _ => None,
        }?;
        stream.play().ok()?;
        Some(Sound { _stream: stream, name: name_of(&device) })
    }

    fn build<T: SizedSample + FromSample<f32> + Send + 'static>(
        device: &Device,
        config: StreamConfig,
        mixer: Arc<Mutex<Mixer>>,
        on_error: impl FnMut(cpal::Error) + Send + 'static,
    ) -> Option<Stream> {
        let channels = usize::from(config.channels);
        let rate = config.sample_rate;
        let mut mix: Vec<f32> = Vec::new();
        device
            .build_output_stream::<T, _, _>(
                config,
                move |data: &mut [T], _| {
                    mix.resize(data.len(), 0.0);
                    // Never wait on the page's thread here: a busy mixer is one quiet buffer.
                    match mixer.try_lock() {
                        Ok(mut mixer) => mixer.fill(&mut mix, channels, rate),
                        Err(_) => mix.fill(0.0),
                    }
                    for (out, sample) in data.iter_mut().zip(&mix) {
                        *out = T::from_sample(*sample);
                    }
                },
                on_error,
                None,
            )
            .ok()
    }
}
