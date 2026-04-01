//! Audio output for /dev/audio.
//!
//! Behind the `audio` feature flag, PCM data written to /dev/audio
//! plays through the system audio device via cpal.
//! Without the feature, writes are silently discarded.

#[cfg(feature = "audio")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

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
            buf.extend(data);
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
            sample_rate: cpal::SampleRate(self.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };
        let buffer = Arc::clone(&self.buffer);
        let stream = device.build_output_stream(
            &config,
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
    fn status_string() {
        let state = AudioState::new();
        let s = state.status();
        assert!(s.contains("rate 44100"));
        assert!(s.contains("chans 2"));
        assert!(s.contains("bits 16"));
    }
}
