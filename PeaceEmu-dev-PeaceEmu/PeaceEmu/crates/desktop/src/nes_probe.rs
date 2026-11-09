//! Lightweight NES content signature probe used when the optional NES module is absent.
#![forbid(unsafe_code)]

use emulation_core::content::{ContentFormatProbe, FormatProbeResult};

/// Read-only probe for iNES/NES 2.0 cartridge headers.
#[derive(Clone, Copy, Debug, Default)]
pub struct NesCartridgeProbe;

impl ContentFormatProbe for NesCartridgeProbe {
    fn format_id(&self) -> &'static str {
        "nes"
    }

    fn probe(&self, prefix: &[u8]) -> FormatProbeResult {
        if prefix.len() < 4 {
            FormatProbeResult::NeedMoreData { minimum_bytes: 4 }
        } else if prefix[..4] == *b"NES\x1A" {
            FormatProbeResult::Match
        } else {
            FormatProbeResult::NoMatch
        }
    }
}
