#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Generic, bounded content envelopes for untrusted game images and firmware.
//!
//! This layer validates only common size and emptiness constraints. Concrete
//! cores remain responsible for validating their own formats and compatibility.

use core::fmt;
use std::io::{ErrorKind, Read};
use std::path::Path;

/// Caller-selected limits applied before content reaches an emulation core.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContentLimits {
    max_bytes: usize,
}

impl ContentLimits {
    /// Creates limits with an explicit maximum payload size in bytes.
    ///
    /// The maximum is required from the caller because valid content sizes vary
    /// widely between systems. A limit of zero rejects every non-empty payload.
    pub const fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }

    /// Returns the configured maximum payload size in bytes.
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
}

/// Common validation failure for a content envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentError {
    /// Empty payloads are never accepted as launchable content.
    Empty,
    /// Reading from a content source failed.
    ReadFailed(ErrorKind),
    /// Payload exceeds the explicit caller-provided size limit.
    TooLarge {
        /// Configured maximum payload size in bytes.
        max_bytes: usize,
        /// Actual payload size in bytes.
        actual_bytes: usize,
    },
}

impl fmt::Display for ContentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("content payload cannot be empty"),
            Self::ReadFailed(kind) => write!(formatter, "failed to read content source: {kind}"),
            Self::TooLarge {
                max_bytes,
                actual_bytes,
            } => write!(
                formatter,
                "content payload is too large: maximum {max_bytes} bytes, got {actual_bytes}"
            ),
        }
    }
}

impl std::error::Error for ContentError {}

/// Outcome of probing bytes for a possible content format.
///
/// A match is only a detection hint; it does not certify structural validity
/// or mean that a core accepts the content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatProbeResult {
    /// The prefix matches this probe's identifying characteristics.
    Match,
    /// The prefix definitively does not match this format.
    NoMatch,
    /// The prefix is too short to decide; the value is the minimum total
    /// prefix length needed for this probe to decide.
    NeedMoreData {
        /// Minimum total prefix length required to decide.
        minimum_bytes: usize,
    },
}

/// A format-specific, read-only probe for untrusted content.
///
/// Implementations should inspect only the supplied bytes, must not rely on a
/// filename extension, and should return `NeedMoreData` rather than treating
/// a truncated identifying header as a match. Probes are not validators.
pub trait ContentFormatProbe {
    /// Returns a stable identifier for the format this probe recognizes.
    fn format_id(&self) -> &'static str;

    /// Examines a prefix and reports whether it may be this format.
    fn probe(&self, prefix: &[u8]) -> FormatProbeResult;
}

/// Results of running format probes against the same content prefix.
///
/// Matches and undecided probes are kept in input order. The caller must still
/// perform format-specific structural validation and ask a core to accept the
/// content; a detected format alone is never sufficient for launch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContentDetection {
    /// Format IDs whose probes reported a match.
    pub matches: Vec<&'static str>,
    /// Format IDs that need more bytes, paired with their minimum total prefix length.
    pub needs_more_data: Vec<(&'static str, usize)>,
}

/// Runs format probes without consulting a path, extension, or other metadata.
///
/// This function does not load or validate content and does not privilege one
/// matching format over another.
pub fn detect_content_formats(
    prefix: &[u8],
    probes: &[&dyn ContentFormatProbe],
) -> ContentDetection {
    let mut detection = ContentDetection::default();
    for probe in probes {
        match probe.probe(prefix) {
            FormatProbeResult::Match => detection.matches.push(probe.format_id()),
            FormatProbeResult::NoMatch => {}
            FormatProbeResult::NeedMoreData { minimum_bytes } => {
                // A buggy or untrusted probe must not claim that the current
                // prefix is already long enough while still requesting data.
                // Normalize such answers to at least one additional byte.
                let minimum_bytes = minimum_bytes.max(prefix.len().saturating_add(1));
                detection
                    .needs_more_data
                    .push((probe.format_id(), minimum_bytes));
            }
        }
    }
    detection
}

/// Owned content that has passed common size checks.
///
/// This type does not certify a file format, authenticity, safety of a core,
/// or legal permission to use the bytes. It prevents accidental bypass of the
/// caller's common size policy when using the validated session-loading API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentImage {
    bytes: Vec<u8>,
}

impl ContentImage {
    /// Validates and takes ownership of a non-empty bounded payload.
    pub fn new(bytes: Vec<u8>, limits: ContentLimits) -> Result<Self, ContentError> {
        if bytes.is_empty() {
            return Err(ContentError::Empty);
        }
        if bytes.len() > limits.max_bytes {
            return Err(ContentError::TooLarge {
                max_bytes: limits.max_bytes,
                actual_bytes: bytes.len(),
            });
        }
        Ok(Self { bytes })
    }

    /// Copies a borrowed payload only after checking the common size limits.
    ///
    /// This avoids allocating a second copy when a caller supplies a slice that
    /// already exceeds the configured maximum.
    pub fn from_slice(bytes: &[u8], limits: ContentLimits) -> Result<Self, ContentError> {
        if bytes.is_empty() {
            return Err(ContentError::Empty);
        }
        if bytes.len() > limits.max_bytes {
            return Err(ContentError::TooLarge {
                max_bytes: limits.max_bytes,
                actual_bytes: bytes.len(),
            });
        }
        Ok(Self {
            bytes: bytes.to_vec(),
        })
    }

    /// Opens a filesystem path and reads it with the same strict size bound.
    ///
    /// File metadata is not trusted for the size decision: the stream reader
    /// enforces the limit even if the file changes while being read.
    pub fn read_path(path: impl AsRef<Path>, limits: ContentLimits) -> Result<Self, ContentError> {
        let file =
            std::fs::File::open(path).map_err(|error| ContentError::ReadFailed(error.kind()))?;

        // Reject obviously oversized files before reading their contents. The
        // bounded stream read remains necessary because metadata can be stale
        // or the file can grow after this check.
        let size = file
            .metadata()
            .map_err(|error| ContentError::ReadFailed(error.kind()))?
            .len();
        if size > limits.max_bytes as u64 {
            return Err(ContentError::TooLarge {
                max_bytes: limits.max_bytes,
                actual_bytes: usize::try_from(size).unwrap_or(usize::MAX),
            });
        }

        Self::read_from(file, limits)
    }

    /// Reads at most the configured limit plus one byte from a stream.
    ///
    /// The extra byte detects oversized inputs without reading an unbounded
    /// source into memory. For oversized input, `actual_bytes` reports the
    /// number of bytes observed (at least the configured limit plus one), not
    /// necessarily the source's total length. A stream error is returned as
    /// `ReadFailed`; no partially read payload is exposed.
    pub fn read_from<R: Read>(reader: R, limits: ContentLimits) -> Result<Self, ContentError> {
        let read_limit = limits.max_bytes.saturating_add(1) as u64;
        let mut bytes = Vec::new();
        reader
            .take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(|error| ContentError::ReadFailed(error.kind()))?;
        if bytes.len() > limits.max_bytes {
            return Err(ContentError::TooLarge {
                max_bytes: limits.max_bytes,
                actual_bytes: bytes.len(),
            });
        }
        Self::new(bytes, limits)
    }

    /// Returns the payload bytes for format-specific core validation.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the validated payload size in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether the payload is empty. Valid instances are never empty.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MagicProbe {
        id: &'static str,
        magic: &'static [u8],
    }

    impl ContentFormatProbe for MagicProbe {
        fn format_id(&self) -> &'static str {
            self.id
        }

        fn probe(&self, prefix: &[u8]) -> FormatProbeResult {
            let compared = prefix.len().min(self.magic.len());
            if prefix[..compared] != self.magic[..compared] {
                return FormatProbeResult::NoMatch;
            }
            if prefix.len() < self.magic.len() {
                return FormatProbeResult::NeedMoreData {
                    minimum_bytes: self.magic.len(),
                };
            }
            FormatProbeResult::Match
        }
    }

    #[test]
    fn format_detection_uses_bytes_without_claiming_validation() {
        let magic = MagicProbe {
            id: "test-format",
            magic: b"ARCH",
        };
        let probes: [&dyn ContentFormatProbe; 1] = [&magic];
        assert_eq!(
            detect_content_formats(b"ARCH payload", &probes),
            ContentDetection {
                matches: vec!["test-format"],
                needs_more_data: vec![],
            }
        );
        assert_eq!(
            detect_content_formats(b"NOPE", &probes),
            ContentDetection::default()
        );
    }

    #[test]
    fn format_detection_reports_truncated_signatures_as_incomplete() {
        let magic = MagicProbe {
            id: "test-format",
            magic: b"ARCH",
        };
        let probes: [&dyn ContentFormatProbe; 1] = [&magic];
        assert_eq!(
            detect_content_formats(b"AR", &probes),
            ContentDetection {
                matches: vec![],
                needs_more_data: vec![("test-format", 4)],
            }
        );
    }

    #[test]
    fn format_detection_keeps_ambiguous_candidates_in_probe_order() {
        let first = MagicProbe {
            id: "first",
            magic: b"A",
        };
        let second = MagicProbe {
            id: "second",
            magic: b"A",
        };
        let probes: [&dyn ContentFormatProbe; 2] = [&first, &second];
        assert_eq!(
            detect_content_formats(b"A", &probes).matches,
            vec!["first", "second"]
        );
    }

    #[test]
    fn format_detection_does_not_match_arbitrary_bytes() {
        let magic = MagicProbe {
            id: "test-format",
            magic: b"ARCH",
        };
        let probes: [&dyn ContentFormatProbe; 1] = [&magic];
        for bytes in [&b""[..], &b"xxxx"[..], &b"ARX"[..]] {
            let detection = detect_content_formats(bytes, &probes);
            if bytes.is_empty() {
                assert_eq!(detection.needs_more_data, vec![("test-format", 4)]);
            } else {
                assert_eq!(detection, ContentDetection::default());
            }
        }
    }

    #[test]
    fn format_detection_normalizes_invalid_incomplete_probe_lengths() {
        struct InvalidIncompleteProbe {
            minimum_bytes: usize,
        }

        impl ContentFormatProbe for InvalidIncompleteProbe {
            fn format_id(&self) -> &'static str {
                "invalid-incomplete"
            }

            fn probe(&self, _prefix: &[u8]) -> FormatProbeResult {
                FormatProbeResult::NeedMoreData {
                    minimum_bytes: self.minimum_bytes,
                }
            }
        }

        for reported_minimum in [0, 1, 3] {
            let probe = InvalidIncompleteProbe {
                minimum_bytes: reported_minimum,
            };
            let probes: [&dyn ContentFormatProbe; 1] = [&probe];
            assert_eq!(
                detect_content_formats(b"ABCD", &probes).needs_more_data,
                vec![("invalid-incomplete", 5)],
                "an incomplete probe must request bytes beyond the supplied prefix"
            );
        }
    }

    #[test]
    fn borrowed_content_is_checked_before_it_is_copied() {
        assert_eq!(
            ContentImage::from_slice(b"", ContentLimits::new(3)),
            Err(ContentError::Empty)
        );
        assert_eq!(
            ContentImage::from_slice(b"four", ContentLimits::new(3)),
            Err(ContentError::TooLarge {
                max_bytes: 3,
                actual_bytes: 4,
            })
        );
        assert_eq!(
            ContentImage::from_slice(b"abc", ContentLimits::new(3))
                .unwrap()
                .as_bytes(),
            b"abc"
        );
    }

    #[test]
    fn reads_a_file_with_the_same_size_limit() {
        let path =
            std::env::temp_dir().join(format!("host-app-content-test-{}.bin", std::process::id()));
        std::fs::write(&path, b"file bytes").unwrap();
        let result = ContentImage::read_path(&path, ContentLimits::new(10));
        let _ = std::fs::remove_file(&path);
        assert_eq!(result.unwrap().as_bytes(), b"file bytes");
    }

    #[test]
    fn rejects_a_file_that_exceeds_the_limit_by_one_byte() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "host-app-content-oversized-{}-{unique}.bin",
            std::process::id()
        ));
        std::fs::write(&path, b"abcd").unwrap();
        let result = ContentImage::read_path(&path, ContentLimits::new(3));
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            result,
            Err(ContentError::TooLarge {
                max_bytes: 3,
                actual_bytes: 4,
            })
        );
    }

    #[test]
    fn reports_file_open_errors() {
        let path = std::env::temp_dir().join(format!(
            "host-app-content-missing-{}.bin",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        assert!(matches!(
            ContentImage::read_path(path, ContentLimits::new(10)),
            Err(ContentError::ReadFailed(_))
        ));
    }

    #[test]
    fn reads_a_stream_within_the_limit() {
        let image = ContentImage::read_from(&b"abc"[..], ContentLimits::new(3)).unwrap();
        assert_eq!(image.as_bytes(), b"abc");
    }

    #[test]
    fn bounded_reader_stops_after_detecting_oversized_input() {
        let mut source: &[u8] = b"abcdef";
        let error = ContentImage::read_from(&mut source, ContentLimits::new(3)).unwrap_err();
        assert_eq!(
            error,
            ContentError::TooLarge {
                max_bytes: 3,
                actual_bytes: 4,
            }
        );
        assert_eq!(
            source, b"ef",
            "the reader must not consume beyond limit + 1"
        );
    }

    #[test]
    fn stream_read_errors_do_not_expose_partially_read_content() {
        struct FailsAfterPrefix {
            remaining: &'static [u8],
        }

        impl Read for FailsAfterPrefix {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if !self.remaining.is_empty() {
                    let count = buffer.len().min(self.remaining.len());
                    buffer[..count].copy_from_slice(&self.remaining[..count]);
                    self.remaining = &self.remaining[count..];
                    return Ok(count);
                }
                Err(std::io::Error::other("injected stream failure"))
            }
        }

        let reader = FailsAfterPrefix {
            remaining: b"partial",
        };
        assert_eq!(
            ContentImage::read_from(reader, ContentLimits::new(32)),
            Err(ContentError::ReadFailed(std::io::ErrorKind::Other))
        );
    }

    #[test]
    fn zero_limit_stream_reads_only_one_byte_to_detect_oversize() {
        let mut source: &[u8] = b"not-empty";
        assert_eq!(
            ContentImage::read_from(&mut source, ContentLimits::new(0)),
            Err(ContentError::TooLarge {
                max_bytes: 0,
                actual_bytes: 1,
            })
        );
        assert_eq!(
            source, b"ot-empty",
            "a zero-byte limit must consume at most one detection byte"
        );
    }

    #[test]
    fn rejects_empty_streams() {
        assert_eq!(
            ContentImage::read_from(&b""[..], ContentLimits::new(3)),
            Err(ContentError::Empty)
        );
    }

    #[test]
    fn accepts_payload_at_exact_size_limit() {
        let image = ContentImage::new(vec![1, 2, 3], ContentLimits::new(3)).unwrap();
        assert_eq!(image.as_bytes(), &[1, 2, 3]);
        assert_eq!(image.len(), 3);
        assert!(!image.is_empty());
    }

    #[test]
    fn rejects_empty_payloads() {
        assert_eq!(
            ContentImage::new(Vec::new(), ContentLimits::new(10)),
            Err(ContentError::Empty)
        );
    }

    #[test]
    fn rejects_payloads_over_the_explicit_limit() {
        assert_eq!(
            ContentImage::new(vec![1, 2, 3], ContentLimits::new(2)),
            Err(ContentError::TooLarge {
                max_bytes: 2,
                actual_bytes: 3,
            })
        );
    }

    #[test]
    fn zero_limit_rejects_non_empty_payloads() {
        assert!(matches!(
            ContentImage::new(vec![1], ContentLimits::new(0)),
            Err(ContentError::TooLarge {
                max_bytes: 0,
                actual_bytes: 1
            })
        ));
    }
}
