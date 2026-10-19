//! Initial renderer for GBA bitmap display modes 3, 4, and 5.
//!
//! This module renders the current video-memory snapshot. It does not yet
//! advance scanlines, model affine backgrounds, or generate VBlank/HBlank IRQs.

use crate::MemoryBus;

/// Width of the GBA display in pixels.
pub const SCREEN_WIDTH: usize = 240;
/// Height of the GBA display in pixels.
pub const SCREEN_HEIGHT: usize = 160;
const VRAM_BASE: u32 = 0x0600_0000;
const PALETTE_BASE: u32 = 0x0500_0000;
const MODE_5_WIDTH: usize = 160;
const MODE_5_HEIGHT: usize = 128;

/// An opaque 240×160 frame in packed ARGB format (0xFFRRGGBB).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoFrame {
    /// Frame width in pixels.
    pub width: usize,
    /// Frame height in pixels.
    pub height: usize,
    /// Row-major opaque pixels, each packed as 0xFFRRGGBB.
    pub pixels: Vec<u32>,
}

impl VideoFrame {
    /// Returns the pixel at (x, y), or None when outside the frame.
    pub fn pixel(&self, x: usize, y: usize) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.pixels.get(y * self.width + x).copied()
    }
}

/// Renders the current GBA bitmap display state.
///
/// Mode 3 uses direct 15-bit color, mode 4 uses 8-bit palette indices, and
/// mode 5 uses direct color in a centered 160×128 viewport. Unsupported tile
/// modes return None so callers can distinguish them from a valid black frame.
/// Forced blank returns an opaque white frame, as specified by the display
/// control register.
pub fn render_bitmap_frame(bus: &MemoryBus) -> Option<VideoFrame> {
    let display_control = bus.read16(0x0400_0000);
    let mode = display_control & 7;
    if mode > 5 || (mode < 3 && display_control & (1 << 7) == 0) {
        return None;
    }

    let mut frame = VideoFrame {
        width: SCREEN_WIDTH,
        height: SCREEN_HEIGHT,
        pixels: vec![0xFF00_0000; SCREEN_WIDTH * SCREEN_HEIGHT],
    };

    if display_control & (1 << 7) != 0 {
        frame.pixels.fill(0xFFFF_FFFF);
        return Some(frame);
    }

    match mode {
        3 => {
            for y in 0..SCREEN_HEIGHT {
                for x in 0..SCREEN_WIDTH {
                    let address = VRAM_BASE + ((y * SCREEN_WIDTH + x) * 2) as u32;
                    frame.pixels[y * SCREEN_WIDTH + x] = rgb555(bus.read16(address));
                }
            }
        }
        4 => {
            let page_offset = if display_control & (1 << 4) != 0 {
                0xA000
            } else {
                0
            };
            for y in 0..SCREEN_HEIGHT {
                for x in 0..SCREEN_WIDTH {
                    let address = VRAM_BASE + page_offset + (y * SCREEN_WIDTH + x) as u32;
                    let index = u32::from(bus.read8(address));
                    let color = bus.read16(PALETTE_BASE + index * 2);
                    frame.pixels[y * SCREEN_WIDTH + x] = rgb555(color);
                }
            }
        }
        5 => {
            let page_offset = if display_control & (1 << 4) != 0 {
                0xA000
            } else {
                0
            };
            for y in 0..MODE_5_HEIGHT {
                for x in 0..MODE_5_WIDTH {
                    let address =
                        VRAM_BASE + page_offset + ((y * MODE_5_WIDTH + x) * 2) as u32;
                    let target = (y + (SCREEN_HEIGHT - MODE_5_HEIGHT) / 2) * SCREEN_WIDTH
                        + x
                        + (SCREEN_WIDTH - MODE_5_WIDTH) / 2;
                    frame.pixels[target] = rgb555(bus.read16(address));
                }
            }
        }
        _ => return None,
    }

    Some(frame)
}

fn rgb555(color: u16) -> u32 {
    let red = u32::from(color & 0x1F);
    let green = u32::from((color >> 5) & 0x1F);
    let blue = u32::from((color >> 10) & 0x1F);
    let red = (red << 3) | (red >> 2);
    let green = (green << 3) | (green >> 2);
    let blue = (blue << 3) | (blue >> 2);
    0xFF00_0000 | (red << 16) | (green << 8) | blue
}
