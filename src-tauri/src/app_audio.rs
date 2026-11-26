//! Keeps Overtok in LIVE Studio's Application list (Windows). LIVE Studio lists only the apps
//! that have sound open, by their `.exe`, and the sound window plays through WebView2's own
//! processes (`msedgewebview2.exe`), so `Overtok.exe` would never show. While the sound window is
//! on, the app keeps a silent sound of its own open: `Overtok.exe` stays in the list, and picking
//! it takes the app's sound with its WebView2 processes (Windows captures an app with the
//! processes it started). Elsewhere this does nothing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::Thread;

/// The silent sound's switch: one thread owns the sound (a stream may not leave its thread).
pub struct AppAudio {
    wanted: Arc<AtomicBool>,
    worker: Option<Thread>,
}

impl AppAudio {
    pub fn start() -> Self {
        let wanted = Arc::new(AtomicBool::new(false));
        let worker = spawn(wanted.clone());
        Self { wanted, worker }
    }

    /// On while the sound window is on.
    pub fn set(&self, on: bool) {
        self.wanted.store(on, Ordering::SeqCst);
        if let Some(worker) = &self.worker {
            worker.unpark();
        }
    }
}

#[cfg(windows)]
fn spawn(wanted: Arc<AtomicBool>) -> Option<Thread> {
    use std::{thread, time::Duration};
    thread::Builder::new()
        .name("overtok-app-audio".into())
        .spawn(move || {
            let broken = Arc::new(AtomicBool::new(false));
            let mut sound = None;
            loop {
                if !wanted.load(Ordering::SeqCst) {
                    sound = None;
                } else if sound.is_none() || broken.swap(false, Ordering::SeqCst) {
                    // A device unplugged or changed: close it first, then open what plays now.
                    drop(sound.take());
                    sound = silent::open(broken.clone(), thread::current());
                }
                // Asleep until switched; a sound that couldn't open tries again soon.
                let wait = if wanted.load(Ordering::SeqCst) && sound.is_none() { 3 } else { 60 };
                thread::park_timeout(Duration::from_secs(wait));
            }
        })
        .ok()
        .map(|handle| handle.thread().clone())
}

#[cfg(not(windows))]
fn spawn(_wanted: Arc<AtomicBool>) -> Option<Thread> {
    None
}

#[cfg(windows)]
mod silent {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{Device, SampleFormat, SizedSample, Stream, StreamConfig};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread::Thread;

    /// Silence on the computer's output now, flagging `broken` (and waking `worker`) if it fails.
    pub fn open(broken: Arc<AtomicBool>, worker: Thread) -> Option<Stream> {
        let device = cpal::default_host().default_output_device()?;
        let supported = device.default_output_config().ok()?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let on_error = move |_| {
            broken.store(true, Ordering::SeqCst);
            worker.unpark();
        };
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, config, on_error),
            SampleFormat::I16 => build::<i16>(&device, config, on_error),
            SampleFormat::I32 => build::<i32>(&device, config, on_error),
            SampleFormat::U16 => build::<u16>(&device, config, on_error),
            _ => None,
        }?;
        stream.play().ok()?;
        Some(stream)
    }

    fn build<T: SizedSample + Send + 'static>(
        device: &Device,
        config: StreamConfig,
        on_error: impl FnMut(cpal::Error) + Send + 'static,
    ) -> Option<Stream> {
        device
            .build_output_stream::<T, _, _>(
                config,
                |data: &mut [T], _| data.fill(T::EQUILIBRIUM),
                on_error,
                None,
            )
            .ok()
    }
}
