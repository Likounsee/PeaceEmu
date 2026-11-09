//! Best-effort native audio playback for core-provided signed 16-bit PCM.

use emulation_core::core_io::AudioBuffer;
use rodio::{buffer::SamplesBuffer, OutputStream, Sink};

const MAX_QUEUED_BUFFERS: usize = 8;

fn normalize_pcm(samples: &[i16]) -> Vec<f32> {
    samples
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect()
}

fn should_clear_backlog(queued_buffers: usize) -> bool {
    queued_buffers >= MAX_QUEUED_BUFFERS
}

/// Owns the native output stream and bounds latency when emulation outpaces playback.
pub(super) struct AudioOutput {
    _stream: Option<OutputStream>,
    sink: Option<Sink>,
    status: String,
    dropped_buffers: u64,
}

impl AudioOutput {
    /// Opens the default host device when available; failure leaves video/emulation usable.
    pub(super) fn new() -> Self {
        match OutputStream::try_default() {
            Ok((stream, handle)) => match Sink::try_new(&handle) {
                Ok(sink) => Self {
                    _stream: Some(stream),
                    sink: Some(sink),
                    status: "Audio device ready".to_owned(),
                    dropped_buffers: 0,
                },
                Err(error) => Self {
                    _stream: Some(stream),
                    sink: None,
                    status: format!("Audio device unavailable: {error}"),
                    dropped_buffers: 0,
                },
            },
            Err(error) => Self {
                _stream: None,
                sink: None,
                status: format!("No audio output device: {error}"),
                dropped_buffers: 0,
            },
        }
    }

    /// Returns the latest host audio-device status for the UI.
    pub(super) fn status(&self) -> &str {
        &self.status
    }

    /// Enqueues validated PCM for playback; excessive backlog is dropped to cap latency.
    pub(super) fn submit(&mut self, buffer: &AudioBuffer) {
        if buffer.samples().is_empty() {
            return;
        }
        let Some(sink) = self.sink.as_ref() else {
            return;
        };

        if should_clear_backlog(sink.len()) {
            sink.clear();
            self.dropped_buffers = self.dropped_buffers.saturating_add(1);
        }
        self.status = if self.dropped_buffers == 0 {
            "Audio playback active".to_owned()
        } else {
            format!(
                "Audio playback active; cleared backlog {} time(s)",
                self.dropped_buffers
            )
        };

        let samples = normalize_pcm(buffer.samples());
        if let Some(sink) = self.sink.as_ref() {
            sink.append(SamplesBuffer::new(
                u16::from(buffer.channels()),
                buffer.sample_rate(),
                samples,
            ));
        }
    }

    /// Pauses queued playback while the active session is paused.
    pub(super) fn pause(&self) {
        if let Some(sink) = &self.sink {
            sink.pause();
        }
    }

    /// Resumes queued playback after a session resumes.
    pub(super) fn resume(&self) {
        if let Some(sink) = &self.sink {
            sink.play();
        }
    }

    /// Drops queued samples on stop/reset so stale audio cannot leak into a new run.
    pub(super) fn clear(&mut self) {
        if let Some(sink) = &self.sink {
            sink.clear();
            sink.play();
            self.status = "Audio device ready".to_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_pcm, should_clear_backlog, MAX_QUEUED_BUFFERS};

    #[test]
    fn signed_pcm_normalization_maps_extremes_and_zero() {
        let normalized = normalize_pcm(&[i16::MIN, 0, i16::MAX]);
        assert_eq!(normalized[0], -1.0);
        assert_eq!(normalized[1], 0.0);
        assert!(normalized[2] < 1.0);
        assert!(normalized[2] > 0.99);
    }

    #[test]
    fn playback_backlog_limit_is_explicit_and_bounded() {
        assert!(!should_clear_backlog(0));
        assert!(!should_clear_backlog(MAX_QUEUED_BUFFERS - 1));
        assert!(should_clear_backlog(MAX_QUEUED_BUFFERS));
        assert!(should_clear_backlog(MAX_QUEUED_BUFFERS + 1));
    }

    #[test]
    fn missing_audio_device_does_not_panic_or_consume_core_samples() {
        let mut output = super::AudioOutput {
            _stream: None,
            sink: None,
            status: "No audio output device".to_owned(),
            dropped_buffers: 0,
        };
        let buffer = emulation_core::core_io::AudioBuffer::new(48_000, 2, vec![100, -100]).unwrap();

        output.submit(&buffer);

        assert_eq!(output.status(), "No audio output device");
        assert_eq!(output.dropped_buffers, 0);
    }
}
