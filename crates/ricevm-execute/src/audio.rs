//! Audio output for /dev/audio.
//!
//! Behind the `audio` feature flag, PCM data written to /dev/audio
//! plays through the system audio device via cpal.
//! Without the feature, writes are silently discarded.

#[cfg(feature = "audio")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Upper bound on queued PCM data (about six seconds at 44.1 kHz stereo 16-bit).
/// Nothing drains the queue when playback is unavailable, so writes past this
/// bound discard the oldest samples instead of growing the buffer forever.
const MAX_BUFFER_BYTES: usize = 1024 * 1024;

pub(crate) struct AudioState {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
    #[cfg(feature = "audio")]
    stream: Option<cpal::Stream>,
    buffer: Arc<Mutex<VecDeque<u8>>>,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            sample_rate: 44100,
            channels: 2,
            bits: 16,
            #[cfg(feature = "audio")]
            stream: None,
            buffer: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Write raw PCM data to the audio buffer.
    ///
    /// With the `audio` feature enabled, data is queued for playback.
    /// Without it, data is silently discarded.
    pub fn write(&mut self, data: &[u8]) -> usize {
        #[cfg(feature = "audio")]
        {
            if self.stream.is_none() {
                self.start_stream();
            }
        }
        if let Ok(mut buf) = self.buffer.lock() {
            // Keep only the newest MAX_BUFFER_BYTES: a guest that writes faster
            // than the device drains (or with no device at all) must not be able
            // to grow this buffer without bound.
            let tail = &data[data.len().saturating_sub(MAX_BUFFER_BYTES)..];
            let overflow = (buf.len() + tail.len())
                .saturating_sub(MAX_BUFFER_BYTES)
                .min(buf.len());
            buf.drain(..overflow);
            buf.extend(tail);
        }
        data.len()
    }

    /// Parse an audioctl command and update configuration.
    ///
    /// Recognized commands: "rate <n>", "chans <n>", and "bits <n>".
    pub fn configure(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.len() >= 2 {
            match parts[0] {
                "rate" => {
                    if let Ok(r) = parts[1].parse() {
                        self.sample_rate = r;
                    }
                }
                "chans" => {
                    if let Ok(c) = parts[1].parse() {
                        self.channels = c;
                    }
                }
                "bits" => {
                    if let Ok(b) = parts[1].parse() {
                        self.bits = b;
                    }
                }
                _ => {}
            }
        }
    }

    /// Return a status string describing the current audio configuration.
    pub fn status(&self) -> String {
        format!(
            "rate {}\nchans {}\nbits {}\n",
            self.sample_rate, self.channels, self.bits
        )
    }

    #[cfg(feature = "audio")]
    fn start_stream(&mut self) {
        let host = cpal::default_host();
        let device = match host.default_output_device() {
            Some(d) => d,
            None => return,
        };
        let config = cpal::StreamConfig {
            channels: self.channels,
            sample_rate: self.sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let buffer = Arc::clone(&self.buffer);
        let stream = device.build_output_stream(
            config,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                let mut buf = buffer.lock().unwrap_or_else(|e| e.into_inner());
                for sample in data.iter_mut() {
                    if buf.len() >= 2 {
                        let lo = buf.pop_front().unwrap_or(0);
                        let hi = buf.pop_front().unwrap_or(0);
                        *sample = i16::from_le_bytes([lo, hi]);
                    } else {
                        *sample = 0; // silence on buffer underrun
                    }
                }
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        );
        if let Ok(s) = stream {
            let _ = s.play();
            self.stream = Some(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let state = AudioState::new();
        assert_eq!(state.sample_rate, 44100);
        assert_eq!(state.channels, 2);
        assert_eq!(state.bits, 16);
    }

    #[test]
    fn configure_rate() {
        let mut state = AudioState::new();
        state.configure("rate 22050");
        assert_eq!(state.sample_rate, 22050);
    }

    #[test]
    fn configure_channels() {
        let mut state = AudioState::new();
        state.configure("chans 1");
        assert_eq!(state.channels, 1);
    }

    #[test]
    fn configure_bits() {
        let mut state = AudioState::new();
        state.configure("bits 8");
        assert_eq!(state.bits, 8);
    }

    #[test]
    fn write_returns_length() {
        let mut state = AudioState::new();
        let data = [0u8; 1024];
        assert_eq!(state.write(&data), 1024);
    }

    #[test]
    fn write_bounds_buffer_growth() {
        let mut state = AudioState::new();
        let chunk = [0u8; 64 * 1024];
        // Nothing drains the buffer without a working output device, so the
        // buffer must not grow without bound.
        for _ in 0..64 {
            assert_eq!(state.write(&chunk), chunk.len());
        }
        let buffered = state.buffer.lock().expect("buffer lock").len();
        assert!(
            buffered <= MAX_BUFFER_BYTES,
            "buffer grew to {buffered} bytes, above the {MAX_BUFFER_BYTES} byte bound"
        );
    }

    #[test]
    fn write_keeps_most_recent_samples() {
        let mut state = AudioState::new();
        state.write(&vec![1u8; MAX_BUFFER_BYTES]);
        state.write(&[7u8, 8u8]);
        let buf = state.buffer.lock().expect("buffer lock");
        assert_eq!(buf.len(), MAX_BUFFER_BYTES);
        assert_eq!(buf.back().copied(), Some(8), "newest samples are kept");
    }

    #[test]
    fn status_string() {
        let state = AudioState::new();
        let s = state.status();
        assert!(s.contains("rate 44100"));
        assert!(s.contains("chans 2"));
        assert!(s.contains("bits 16"));
    }
}
