#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Owned, platform-neutral data contracts for core input and media output.
//!
//! These types define data shape and validation only. They do not provide a
//! renderer, audio backend, scheduler, controller mapper, or console emulation.

use core::fmt;
use std::collections::VecDeque;

/// Host-level control intent before it is mapped to a console's hardware.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputControl {
    /// Up direction.
    Up,
    /// Down direction.
    Down,
    /// Left direction.
    Left,
    /// Right direction.
    Right,
    /// First generic action button.
    Action1,
    /// Second generic action button.
    Action2,
    /// Third generic action button.
    Action3,
    /// Fourth generic action button.
    Action4,
    /// Start or confirm.
    Start,
    /// Select or secondary menu action.
    Select,
}

/// A change in host-level input state.
///
/// Repeated key-down events should be normalized by the host; cores receive
/// state transitions rather than platform-specific keyboard scan codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputEvent {
    /// Semantic control whose state changed.
    pub control: InputControl,
    /// Whether the control is now held.
    pub pressed: bool,
}

/// Packed pixel representation for a complete video frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    /// Four bytes per pixel in red, green, blue, alpha order.
    Rgba8888,
}

/// Errors raised when media payload metadata is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaDataError {
    /// Frame dimensions must both be non-zero.
    EmptyFrame,
    /// Dimensions overflow the host address space when calculating byte size.
    FrameSizeOverflow,
    /// Row stride is smaller than the bytes required by one row.
    InvalidRowStride {
        /// Minimum valid number of bytes in one row.
        minimum: usize,
        /// Declared number of bytes in one row.
        actual: usize,
    },
    /// Pixel payload length does not match the declared dimensions, stride and format.
    InvalidPixelDataLength {
        /// Expected number of bytes.
        expected: usize,
        /// Actual number of bytes.
        actual: usize,
    },
    /// Audio sample rate or channel count is zero.
    InvalidAudioFormat,
    /// Interleaved sample count is not divisible by the channel count.
    IncompleteAudioFrame {
        /// Number of interleaved samples supplied.
        samples: usize,
        /// Number of channels per sample frame.
        channels: u8,
    },
}

impl fmt::Display for MediaDataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFrame => formatter.write_str("video frame dimensions must be non-zero"),
            Self::FrameSizeOverflow => formatter.write_str("video frame byte size overflows usize"),
            Self::InvalidRowStride { minimum, actual } => write!(
                formatter,
                "invalid video row stride: expected at least {minimum} bytes, got {actual}"
            ),
            Self::InvalidPixelDataLength { expected, actual } => write!(
                formatter,
                "invalid video pixel data length: expected {expected} bytes, got {actual}"
            ),
            Self::InvalidAudioFormat => {
                formatter.write_str("audio sample rate and channel count must be non-zero")
            }
            Self::IncompleteAudioFrame { samples, channels } => write!(
                formatter,
                "{samples} interleaved audio samples are not divisible by {channels} channels"
            ),
        }
    }
}

impl std::error::Error for MediaDataError {}

/// An owned, complete video frame in a declared pixel format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoFrame {
    /// Frame width in pixels.
    width: u32,
    height: u32,
    format: PixelFormat,
    row_stride_bytes: usize,
    pixels: Vec<u8>,
}

impl VideoFrame {
    /// Returns the frame width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the frame height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the pixel format of this frame.
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    /// Returns the byte distance between the starts of adjacent rows.
    pub const fn row_stride_bytes(&self) -> usize {
        self.row_stride_bytes
    }

    /// Returns the owned pixel payload, including any padding at the end of each row.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Creates a tightly packed frame after validating dimensions and exact payload length.
    ///
    /// The row stride is inferred from the width and pixel format.
    pub fn new(
        width: u32,
        height: u32,
        format: PixelFormat,
        pixels: Vec<u8>,
    ) -> Result<Self, MediaDataError> {
        if width == 0 || height == 0 {
            return Err(MediaDataError::EmptyFrame);
        }
        let bytes_per_pixel = match format {
            PixelFormat::Rgba8888 => 4usize,
        };
        let row_stride_bytes = (width as usize)
            .checked_mul(bytes_per_pixel)
            .ok_or(MediaDataError::FrameSizeOverflow)?;
        Self::with_stride(width, height, format, row_stride_bytes, pixels)
    }

    /// Creates a frame with an explicit row stride and exact owned payload length.
    ///
    /// A stride may include padding but cannot be smaller than one packed pixel row.
    /// The payload length must equal `row_stride_bytes * height`.
    pub fn with_stride(
        width: u32,
        height: u32,
        format: PixelFormat,
        row_stride_bytes: usize,
        pixels: Vec<u8>,
    ) -> Result<Self, MediaDataError> {
        if width == 0 || height == 0 {
            return Err(MediaDataError::EmptyFrame);
        }
        let bytes_per_pixel = match format {
            PixelFormat::Rgba8888 => 4usize,
        };
        let minimum_stride = (width as usize)
            .checked_mul(bytes_per_pixel)
            .ok_or(MediaDataError::FrameSizeOverflow)?;
        if row_stride_bytes < minimum_stride {
            return Err(MediaDataError::InvalidRowStride {
                minimum: minimum_stride,
                actual: row_stride_bytes,
            });
        }
        let expected = row_stride_bytes
            .checked_mul(height as usize)
            .ok_or(MediaDataError::FrameSizeOverflow)?;
        if pixels.len() != expected {
            return Err(MediaDataError::InvalidPixelDataLength {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            format,
            row_stride_bytes,
            pixels,
        })
    }
}

/// Owned interleaved signed 16-bit PCM audio samples.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioBuffer {
    /// Number of samples per second per channel.
    sample_rate: u32,
    channels: u8,
    samples: Vec<i16>,
}

impl AudioBuffer {
    /// Returns the number of samples per second per channel.
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Returns the number of interleaved channels.
    pub const fn channels(&self) -> u8 {
        self.channels
    }

    /// Returns the interleaved signed 16-bit PCM samples.
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }

    /// Creates an audio buffer with a valid format and complete sample frames.
    pub fn new(sample_rate: u32, channels: u8, samples: Vec<i16>) -> Result<Self, MediaDataError> {
        if sample_rate == 0 || channels == 0 {
            return Err(MediaDataError::InvalidAudioFormat);
        }
        if samples.len() % usize::from(channels) != 0 {
            return Err(MediaDataError::IncompleteAudioFrame {
                samples: samples.len(),
                channels,
            });
        }
        Ok(Self {
            sample_rate,
            channels,
            samples,
        })
    }

    /// Returns the number of complete sample frames in this buffer.
    pub fn frame_count(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
}

/// Errors when constructing a host frame cadence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CadenceError {
    /// A zero rate or a rate beyond nanosecond precision cannot define a frame interval.
    ZeroFrameRate,
    /// A zero catch-up limit would prevent all frames from being emitted.
    ZeroCatchUpLimit,
}

impl fmt::Display for CadenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroFrameRate => formatter.write_str("frame rate must be greater than zero"),
            Self::ZeroCatchUpLimit => {
                formatter.write_str("catch-up limit must be greater than zero")
            }
        }
    }
}

impl std::error::Error for CadenceError {}

/// Result of advancing a deterministic host-side frame cadence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CadenceAdvance {
    /// Frames ready for emulation, capped by the configured catch-up limit.
    pub due_frames: u32,
    /// Whole frame intervals discarded to prevent an unbounded catch-up spiral.
    pub dropped_frames: u64,
}

/// Fixed-rate cadence accumulator independent from wall-clock acquisition.
///
/// The host supplies elapsed time, allowing a frontend to choose its own clock.
/// Fractional frame time is preserved between calls. A long stall emits at most
/// the configured catch-up bound and discards excess whole intervals, so a slow
/// frontend cannot create an ever-growing backlog.
#[derive(Clone, Debug)]
pub struct FrameCadence {
    frame_period: std::time::Duration,
    max_catch_up_frames: u32,
    remainder: std::time::Duration,
}

impl FrameCadence {
    /// Creates a cadence for the requested frames per second and catch-up bound.
    ///
    /// Rates above one billion frames per second are rejected because their
    /// nanosecond period would round to zero.
    pub fn new(frames_per_second: u32, max_catch_up_frames: u32) -> Result<Self, CadenceError> {
        if frames_per_second == 0 {
            return Err(CadenceError::ZeroFrameRate);
        }
        if max_catch_up_frames == 0 {
            return Err(CadenceError::ZeroCatchUpLimit);
        }
        let period_nanos = 1_000_000_000u64 / u64::from(frames_per_second);
        if period_nanos == 0 {
            return Err(CadenceError::ZeroFrameRate);
        }
        Ok(Self {
            frame_period: std::time::Duration::from_nanos(period_nanos),
            max_catch_up_frames,
            remainder: std::time::Duration::ZERO,
        })
    }

    /// Returns the target frame period used by this cadence.
    pub const fn frame_period(&self) -> std::time::Duration {
        self.frame_period
    }

    /// Clears accumulated fractional time, for example when pausing or switching content.
    pub fn reset(&mut self) {
        self.remainder = std::time::Duration::ZERO;
    }

    /// Adds elapsed host time and returns due frames plus any dropped backlog.
    pub fn advance(&mut self, elapsed: std::time::Duration) -> CadenceAdvance {
        let total = self.remainder.saturating_add(elapsed);
        let period_nanos = self.frame_period.as_nanos();
        let total_nanos = total.as_nanos();
        let available = total_nanos / period_nanos;
        let remainder_nanos = total_nanos % period_nanos;
        self.remainder = std::time::Duration::from_nanos(remainder_nanos as u64);

        let due_frames = available.min(u128::from(self.max_catch_up_frames)) as u32;
        let dropped_frames = available.saturating_sub(u128::from(due_frames));
        CadenceAdvance {
            due_frames,
            dropped_frames: dropped_frames.min(u128::from(u64::MAX)) as u64,
        }
    }
}

/// Policy applied when a bounded media queue is full.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueOverflowPolicy {
    /// Preserve recent video by removing the oldest queued item.
    DropOldest,
    /// Preserve all queued audio by rejecting the newly submitted item.
    RejectNewest,
}

/// Outcome of submitting an item to a bounded media queue.
#[derive(Debug, Eq, PartialEq)]
pub enum QueuePushResult<T> {
    /// The item was added without discarding queued data.
    Enqueued,
    /// The item was added after removing the oldest queued item.
    EnqueuedDroppingOldest(T),
    /// The queue was full and the submitted item was returned to the caller.
    Rejected(T),
}

/// A bounded FIFO for host-facing media with an explicit overflow policy.
///
/// Capacity is enforced before accepting data. Hosts can use DropOldest for
/// latency-sensitive video and RejectNewest for audio, where the producer
/// must handle back-pressure rather than silently losing already queued audio.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedMediaQueue<T> {
    capacity: usize,
    policy: QueueOverflowPolicy,
    items: VecDeque<T>,
}

impl<T> BoundedMediaQueue<T> {
    /// Creates an empty queue with a non-zero item capacity.
    pub fn new(capacity: usize, policy: QueueOverflowPolicy) -> Result<Self, QueueCapacityError> {
        if capacity == 0 {
            return Err(QueueCapacityError);
        }
        Ok(Self {
            capacity,
            policy,
            items: VecDeque::with_capacity(capacity),
        })
    }

    /// Returns the maximum number of queued items.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the number of currently queued items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns true when no items are queued.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Removes and returns the oldest queued item.
    pub fn pop_front(&mut self) -> Option<T> {
        self.items.pop_front()
    }

    /// Adds an item, applying the configured full-queue policy.
    pub fn push(&mut self, item: T) -> QueuePushResult<T> {
        if self.items.len() < self.capacity {
            self.items.push_back(item);
            return QueuePushResult::Enqueued;
        }

        match self.policy {
            QueueOverflowPolicy::DropOldest => {
                let dropped = self.items.pop_front().expect("full queue is non-empty");
                self.items.push_back(item);
                QueuePushResult::EnqueuedDroppingOldest(dropped)
            }
            QueueOverflowPolicy::RejectNewest => QueuePushResult::Rejected(item),
        }
    }
}

/// Error returned when a bounded queue is configured with zero capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueCapacityError;

impl fmt::Display for QueueCapacityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("media queue capacity must be greater than zero")
    }
}

impl std::error::Error for QueueCapacityError {}

/// Canonical host-side input state shared by all emulation cores.
///
/// It deduplicates repeated transitions and can synthesize releases when the
/// host loses focus, preventing a key from remaining stuck in the emulated
/// machine after its physical key-up event was missed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InputState {
    pressed: [bool; 10],
}

impl InputState {
    /// Creates an input state with every control released.
    pub const fn new() -> Self {
        Self {
            pressed: [false; 10],
        }
    }

    /// Returns whether a semantic control is currently held.
    pub const fn is_pressed(&self, control: InputControl) -> bool {
        self.pressed[control.index()]
    }

    /// Applies a transition and returns whether it changed the held state.
    ///
    /// Repeated key-down or key-up events are ignored and return false.
    pub fn apply(&mut self, event: InputEvent) -> bool {
        let state = &mut self.pressed[event.control.index()];
        if *state == event.pressed {
            return false;
        }
        *state = event.pressed;
        true
    }

    /// Releases all held controls in a stable order and returns their events.
    ///
    /// Hosts should call this when focus is lost or a controller disconnects.
    pub fn release_all(&mut self) -> Vec<InputEvent> {
        let mut released = Vec::new();
        for control in InputControl::ALL {
            if self.apply(InputEvent {
                control,
                pressed: false,
            }) {
                released.push(InputEvent {
                    control,
                    pressed: false,
                });
            }
        }
        released
    }

    /// Returns the number of controls currently held.
    pub fn pressed_count(&self) -> usize {
        self.pressed.iter().filter(|pressed| **pressed).count()
    }
}

impl InputControl {
    /// All semantic controls in the canonical release order.
    pub const ALL: [Self; 10] = [
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::Action1,
        Self::Action2,
        Self::Action3,
        Self::Action4,
        Self::Start,
        Self::Select,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Up => 0,
            Self::Down => 1,
            Self::Left => 2,
            Self::Right => 3,
            Self::Action1 => 4,
            Self::Action2 => 5,
            Self::Action3 => 6,
            Self::Action4 => 7,
            Self::Start => 8,
            Self::Select => 9,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_media_queue_drops_oldest_for_video_without_growing() {
        let mut queue = BoundedMediaQueue::new(2, QueueOverflowPolicy::DropOldest).unwrap();
        assert_eq!(queue.push(10), QueuePushResult::Enqueued);
        assert_eq!(queue.push(20), QueuePushResult::Enqueued);
        assert_eq!(queue.push(30), QueuePushResult::EnqueuedDroppingOldest(10));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.pop_front(), Some(20));
        assert_eq!(queue.pop_front(), Some(30));
        assert!(queue.is_empty());
    }

    #[test]
    fn bounded_media_queue_rejects_new_audio_without_losing_queued_samples() {
        let mut queue = BoundedMediaQueue::new(1, QueueOverflowPolicy::RejectNewest).unwrap();
        assert_eq!(queue.push("first"), QueuePushResult::Enqueued);
        assert_eq!(queue.push("second"), QueuePushResult::Rejected("second"));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.pop_front(), Some("first"));
    }

    #[test]
    fn bounded_media_queue_rejects_zero_capacity() {
        assert_eq!(
            BoundedMediaQueue::<u8>::new(0, QueueOverflowPolicy::DropOldest).unwrap_err(),
            QueueCapacityError
        );
    }

    #[test]
    fn video_frame_requires_nonzero_dimensions_and_exact_rgba_payload() {
        assert_eq!(
            VideoFrame::new(0, 1, PixelFormat::Rgba8888, vec![]),
            Err(MediaDataError::EmptyFrame)
        );
        assert_eq!(
            VideoFrame::new(2, 1, PixelFormat::Rgba8888, vec![0; 7]),
            Err(MediaDataError::InvalidPixelDataLength {
                expected: 8,
                actual: 7
            })
        );
        let frame = VideoFrame::new(2, 1, PixelFormat::Rgba8888, vec![0; 8]).unwrap();
        assert_eq!(frame.width(), 2);
        assert_eq!(frame.height(), 1);
        assert_eq!(frame.pixels().len(), 8);
    }

    #[test]
    fn video_frame_supports_explicit_padded_row_stride() {
        let frame = VideoFrame::with_stride(1, 2, PixelFormat::Rgba8888, 8, vec![1; 16]).unwrap();
        assert_eq!(frame.row_stride_bytes(), 8);
        assert_eq!(frame.pixels().len(), 16);
    }

    #[test]
    fn video_frame_rejects_stride_smaller_than_a_packed_row() {
        assert_eq!(
            VideoFrame::with_stride(2, 1, PixelFormat::Rgba8888, 7, vec![0; 7]),
            Err(MediaDataError::InvalidRowStride {
                minimum: 8,
                actual: 7,
            })
        );
    }

    #[test]
    fn cadence_preserves_fractional_time_between_advances() {
        let mut cadence = FrameCadence::new(60, 3).unwrap();
        assert_eq!(
            cadence.advance(std::time::Duration::from_millis(10)),
            CadenceAdvance::default()
        );
        assert_eq!(
            cadence.advance(std::time::Duration::from_millis(7)),
            CadenceAdvance {
                due_frames: 1,
                dropped_frames: 0
            }
        );
    }

    #[test]
    fn cadence_bounds_catch_up_after_a_long_host_stall() {
        let mut cadence = FrameCadence::new(60, 2).unwrap();
        assert_eq!(
            cadence.advance(std::time::Duration::from_secs(1)),
            CadenceAdvance {
                due_frames: 2,
                dropped_frames: 58
            }
        );
        assert_eq!(
            cadence.advance(std::time::Duration::from_millis(16)),
            CadenceAdvance::default()
        );
    }

    #[test]
    fn cadence_emits_frames_only_at_exact_period_boundaries() {
        let mut cadence = FrameCadence::new(50, 4).unwrap();
        let period = cadence.frame_period();

        assert_eq!(
            cadence
                .advance(period - std::time::Duration::from_nanos(1))
                .due_frames,
            0
        );
        assert_eq!(
            cadence
                .advance(std::time::Duration::from_nanos(1))
                .due_frames,
            1
        );
        assert_eq!(cadence.advance(period * 2).due_frames, 2);
    }

    #[test]
    fn cadence_reset_discards_fractional_time_before_resume() {
        let mut cadence = FrameCadence::new(60, 4).unwrap();
        let half_period = cadence.frame_period() / 2;

        assert_eq!(cadence.advance(half_period).due_frames, 0);
        cadence.reset();
        assert_eq!(cadence.advance(half_period).due_frames, 0);
        assert_eq!(
            cadence.advance(half_period).due_frames,
            1,
            "only time accumulated after reset should contribute"
        );
    }

    #[test]
    fn input_state_releases_all_controls_in_canonical_order() {
        let mut state = InputState::new();
        for control in InputControl::ALL.into_iter().rev() {
            assert!(state.apply(InputEvent {
                control,
                pressed: true
            }));
        }

        assert_eq!(state.pressed_count(), InputControl::ALL.len());
        let released = state.release_all();
        assert_eq!(
            released
                .iter()
                .map(|event| event.control)
                .collect::<Vec<_>>(),
            InputControl::ALL
        );
        assert!(released.iter().all(|event| !event.pressed));
        assert_eq!(state.pressed_count(), 0);
        assert!(state.release_all().is_empty());
    }

    #[test]
    fn cadence_drops_excess_whole_frames_but_preserves_fractional_remainder() {
        let mut cadence = FrameCadence::new(10, 2).unwrap();

        assert_eq!(
            cadence.advance(std::time::Duration::from_millis(550)),
            CadenceAdvance {
                due_frames: 2,
                dropped_frames: 3,
            }
        );
        assert_eq!(
            cadence
                .advance(std::time::Duration::from_millis(49))
                .due_frames,
            0
        );
        assert_eq!(
            cadence
                .advance(std::time::Duration::from_millis(1))
                .due_frames,
            1
        );
    }

    #[test]
    fn cadence_caps_reported_dropped_frames_for_extreme_elapsed_time() {
        let mut cadence = FrameCadence::new(60, 2).unwrap();

        let result = cadence.advance(std::time::Duration::MAX);
        assert_eq!(result.due_frames, 2);
        assert_eq!(result.dropped_frames, u64::MAX);
        assert!(cadence.remainder < cadence.frame_period());
    }

    #[test]
    fn cadence_accepts_exact_catch_up_limit_without_reporting_drops() {
        let mut cadence = FrameCadence::new(10, 3).unwrap();

        assert_eq!(
            cadence.advance(std::time::Duration::from_millis(300)),
            CadenceAdvance {
                due_frames: 3,
                dropped_frames: 0,
            }
        );
    }

    #[test]
    fn cadence_rejects_unrepresentable_rates_and_zero_catch_up_limit() {
        assert_eq!(
            FrameCadence::new(0, 1).unwrap_err(),
            CadenceError::ZeroFrameRate
        );
        assert_eq!(
            FrameCadence::new(1_000_000_001, 1).unwrap_err(),
            CadenceError::ZeroFrameRate,
            "rates above nanosecond precision must not create a zero-length period"
        );
        let mut one_nanosecond = FrameCadence::new(1_000_000_000, 1).unwrap();
        assert_eq!(
            one_nanosecond.frame_period(),
            std::time::Duration::from_nanos(1)
        );
        assert_eq!(
            one_nanosecond
                .advance(std::time::Duration::from_nanos(1))
                .due_frames,
            1
        );
        assert_eq!(
            FrameCadence::new(60, 0).unwrap_err(),
            CadenceError::ZeroCatchUpLimit
        );
    }

    #[test]
    fn video_frame_rejects_stride_height_byte_size_overflow() {
        assert_eq!(
            VideoFrame::with_stride(1, 2, PixelFormat::Rgba8888, usize::MAX, Vec::new(),),
            Err(MediaDataError::FrameSizeOverflow)
        );
    }

    #[test]
    fn video_frame_rejects_dimensions_whose_byte_size_overflows() {
        assert_eq!(
            VideoFrame::new(u32::MAX, u32::MAX, PixelFormat::Rgba8888, Vec::new(),),
            Err(MediaDataError::FrameSizeOverflow)
        );
    }

    #[test]
    fn audio_buffer_requires_valid_format_and_complete_interleaved_frames() {
        assert_eq!(
            AudioBuffer::new(0, 2, vec![]),
            Err(MediaDataError::InvalidAudioFormat)
        );
        assert_eq!(
            AudioBuffer::new(48_000, 2, vec![1]),
            Err(MediaDataError::IncompleteAudioFrame {
                samples: 1,
                channels: 2
            })
        );
        let audio = AudioBuffer::new(48_000, 2, vec![1, -1, 2, -2]).unwrap();
        assert_eq!(audio.frame_count(), 2);
    }

    #[test]
    fn empty_audio_buffer_is_a_valid_zero_frame_buffer() {
        let audio = AudioBuffer::new(48_000, 2, Vec::new()).unwrap();
        assert_eq!(audio.frame_count(), 0);
        assert!(audio.samples().is_empty());
    }

    #[test]
    fn input_state_deduplicates_repeated_transitions() {
        let mut state = InputState::new();
        let down = InputEvent {
            control: InputControl::Action1,
            pressed: true,
        };
        let up = InputEvent {
            control: InputControl::Action1,
            pressed: false,
        };

        assert!(state.apply(down));
        assert!(!state.apply(down));
        assert!(state.is_pressed(InputControl::Action1));
        assert_eq!(state.pressed_count(), 1);
        assert!(state.apply(up));
        assert!(!state.apply(up));
        assert_eq!(state.pressed_count(), 0);
    }

    #[test]
    fn input_state_releases_all_held_controls_after_focus_loss() {
        let mut state = InputState::new();
        for control in [InputControl::Up, InputControl::Action2, InputControl::Start] {
            assert!(state.apply(InputEvent {
                control,
                pressed: true,
            }));
        }

        let released = state.release_all();
        assert_eq!(released.len(), 3);
        assert!(released.iter().all(|event| !event.pressed));
        assert_eq!(released[0].control, InputControl::Up);
        assert_eq!(released[1].control, InputControl::Action2);
        assert_eq!(released[2].control, InputControl::Start);
        assert_eq!(state.pressed_count(), 0);
        assert!(state.release_all().is_empty());
    }

    #[test]
    fn input_events_are_platform_neutral_state_transitions() {
        assert_eq!(
            InputEvent {
                control: InputControl::Action1,
                pressed: true
            },
            InputEvent {
                control: InputControl::Action1,
                pressed: true
            }
        );
    }
}
