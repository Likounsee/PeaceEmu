//! GBA renderer for text background mode 0 and bitmap modes 3, 4, and 5.
//!
//! This module renders the current video-memory snapshot. It does not yet
//! advance scanlines, composite multiple backgrounds/objects, model affine
//! backgrounds, or generate VBlank/HBlank IRQs.

use crate::MemoryBus;

/// Width of the GBA display in pixels.
pub const SCREEN_WIDTH: usize = 240;
/// Height of the GBA display in pixels.
pub const SCREEN_HEIGHT: usize = 160;
const VRAM_BASE: u32 = 0x0600_0000;
const PALETTE_BASE: u32 = 0x0500_0000;
const MODE_5_WIDTH: usize = 160;
const MODE_5_HEIGHT: usize = 128;
const BG0CNT: u32 = 0x0400_0008;
const BG0HOFS: u32 = 0x0400_0010;
const BG0VOFS: u32 = 0x0400_0012;

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
/// Mode 0 composites BG0–BG3 as tiled text backgrounds; mode 3 uses direct 15-bit
/// color, mode 4 uses 8-bit palette indices, and mode 5 uses direct color in
/// a centered 160×128 viewport. Modes 1, 2, and 6+ return None unless forced
/// blank is active. Forced blank returns an opaque white frame.
pub fn render_bitmap_frame(bus: &MemoryBus) -> Option<VideoFrame> {
    let display_control = bus.read16(0x0400_0000);
    let mode = display_control & 7;
    let forced_blank = display_control & (1 << 7) != 0;
    if !forced_blank && (mode == 1 || mode == 2 || mode > 5) {
        return None;
    }

    let mut frame = VideoFrame {
        width: SCREEN_WIDTH,
        height: SCREEN_HEIGHT,
        pixels: vec![0xFF00_0000; SCREEN_WIDTH * SCREEN_HEIGHT],
    };

    if forced_blank {
        frame.pixels.fill(0xFFFF_FFFF);
        return Some(frame);
    }

    match mode {
        0 => render_text_bg0(bus, display_control, &mut frame),
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
                    let address = VRAM_BASE + page_offset + ((y * MODE_5_WIDTH + x) * 2) as u32;
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

/// Composites regular text backgrounds from back to front (priority 3 to 0).
/// Sprites and affine backgrounds are not implemented yet.
fn render_text_bg0(bus: &MemoryBus, display_control: u16, frame: &mut VideoFrame) {
    frame.pixels.fill(rgb555(bus.read16(PALETTE_BASE)));
    for priority in (0..=3).rev() {
        // On equal priorities BG0 wins over BG1, then BG2, then BG3.
        // Draw the lower-numbered background last so it remains in front.
        for bg in (0..4).rev() {
            let control_address = BG0CNT + bg * 2;
            let control = bus.read16(control_address);
            if display_control & (1 << (8 + bg)) == 0 || control & 3 != priority as u16 {
                continue;
            }
            render_text_background(bus, bg, control, frame);
        }
    }
}

fn render_text_background(bus: &MemoryBus, bg: u32, control: u16, frame: &mut VideoFrame) {
    let hofs_address = BG0HOFS + bg * 4;
    let vofs_address = BG0VOFS + bg * 4;
    let char_base = u32::from((control >> 2) & 0x3) * 0x4000;
    let screen_base = u32::from((control >> 8) & 0x1F) * 0x800;
    let color_8bpp = control & (1 << 7) != 0;
    let size = (control >> 14) & 3;
    let (map_width, map_height) = match size {
        0 => (256_u32, 256_u32),
        1 => (512, 256),
        2 => (256, 512),
        _ => (512, 512),
    };
    let scroll_x = u32::from(bus.read16(hofs_address) & 0x01FF);
    let scroll_y = u32::from(bus.read16(vofs_address) & 0x01FF);
    let map_blocks_per_row = map_width / 256;

    for y in 0..SCREEN_HEIGHT {
        let world_y = (y as u32 + scroll_y) % map_height;
        let tile_y = world_y / 8;
        let in_tile_y = world_y & 7;
        for x in 0..SCREEN_WIDTH {
            let world_x = (x as u32 + scroll_x) % map_width;
            let tile_x = world_x / 8;
            let in_tile_x = world_x & 7;
            let block_x = tile_x / 32;
            let block_y = tile_y / 32;
            let block = block_y * map_blocks_per_row + block_x;
            let map_offset = screen_base + block * 0x800 + ((tile_y % 32) * 32 + tile_x % 32) * 2;
            let entry = bus.read16(VRAM_BASE + map_offset);
            let tile_index = u32::from(entry & 0x03FF);
            let pixel_x = if entry & (1 << 10) != 0 {
                7 - in_tile_x
            } else {
                in_tile_x
            };
            let pixel_y = if entry & (1 << 11) != 0 {
                7 - in_tile_y
            } else {
                in_tile_y
            };
            // Tile-map indices are expressed in 32-byte units. In 8bpp
            // mode each tile occupies 64 bytes, so bit 0 of the tile index
            // is ignored (the tile starts on an even 32-byte slot).
            let tile_offset = if color_8bpp {
                (tile_index & !1) * 32
            } else {
                tile_index * 32
            };
            let tile_address = VRAM_BASE + char_base + tile_offset;
            let (palette_index, transparent) = if color_8bpp {
                let index = u32::from(bus.read8(tile_address + pixel_y * 8 + pixel_x));
                (index, index == 0)
            } else {
                let packed = bus.read8(tile_address + pixel_y * 4 + pixel_x / 2);
                let nibble = if pixel_x & 1 == 0 {
                    packed & 0x0F
                } else {
                    packed >> 4
                };
                let index = u32::from(nibble);
                (index + u32::from((entry >> 12) & 0xF) * 16, index == 0)
            };
            // Color index zero is transparent even when a 4bpp palette bank
            // makes its absolute palette index nonzero.
            if !transparent {
                frame.pixels[y * SCREEN_WIDTH + x] =
                    rgb555(bus.read16(PALETTE_BASE + palette_index * 2));
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GbaCartridge, MemoryBus};

    fn bus() -> MemoryBus {
        let mut rom = vec![0; 0xC0];
        rom[0xBD] = rom[0xA0..=0xBC]
            .iter()
            .fold(0x19_u8, |sum, byte| sum.wrapping_add(*byte))
            .wrapping_neg();
        MemoryBus::new(GbaCartridge::parse(&rom).expect("valid test ROM"))
    }

    #[test]
    fn mode0_renders_4bpp_bg0_tile_and_palette_bank() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 << 8); // mode 0, BG0 enabled
                                          // Tile 1: pixels 0 and 1 are 1 and 2.
        bus.write8(VRAM_BASE + 32, 0x21);
        // Select tile 1 by changing the map entry.
        bus.write16(VRAM_BASE, (1 << 12) | 1);
        bus.write16(PALETTE_BASE + 34, 0x001F); // palette 17: red
        bus.write16(PALETTE_BASE + 36, 0x03E0); // palette 18: green
        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(1, 0), Some(0xFF00_FF00));
    }

    #[test]
    fn mode0_renders_8bpp_tile_pixels_and_zero_transparency() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 << 8); // mode 0, BG0 enabled
        bus.write16(BG0CNT, 1 << 7); // 8bpp tiles
                                     // In 8bpp mode tile index 1 aliases tile index 0; tile 2
                                     // starts at byte offset 64 in character data.
        bus.write16(VRAM_BASE, 2);
        bus.write8(VRAM_BASE + 64, 3);
        bus.write8(VRAM_BASE + 65, 4);
        bus.write16(PALETTE_BASE, 0x7C00); // blue backdrop for transparent index zero
        bus.write16(PALETTE_BASE + 6, 0x001F); // palette 3: red
        bus.write16(PALETTE_BASE + 8, 0x03E0); // palette 4: green
        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(1, 0), Some(0xFF00_FF00));
        assert_eq!(frame.pixel(2, 0), Some(0xFF00_00FF));
    }

    #[test]
    fn mode0_8bpp_odd_tile_index_uses_even_tile_slot() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 << 8); // mode 0, BG0 enabled
        bus.write16(BG0CNT, 1 << 7); // 8bpp tiles
        bus.write16(VRAM_BASE, 3); // odd tile index aliases tile 2
        bus.write8(VRAM_BASE + 64, 5); // tile 2, first pixel
        bus.write8(VRAM_BASE + 96, 6); // tile 3 slot must not be used
        bus.write16(PALETTE_BASE + 10, 0x001F); // palette 5: red
        bus.write16(PALETTE_BASE + 12, 0x03E0); // palette 6: green
        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
    }

    #[test]
    fn mode0_zero_pixel_stays_transparent_in_nonzero_palette_bank() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 << 8); // mode 0, BG0 enabled
        bus.write16(PALETTE_BASE, 0x7C00); // blue backdrop
                                           // Tile 1 begins with zero (transparent); next pixel uses palette entry 17.
        bus.write16(VRAM_BASE, (1 << 12) | 1);
        bus.write8(VRAM_BASE + 32, 0x10);
        bus.write16(PALETTE_BASE + 34, 0x001F); // palette 17: red
        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFF00_00FF));
        assert_eq!(frame.pixel(1, 0), Some(0xFFFF_0000));
    }

    #[test]
    fn mode0_composites_backgrounds_by_priority() {
        let mut bus = bus();
        // Enable BG0 and BG1. BG0 has priority 0; BG1 has priority 1.
        bus.write16(0x0400_0000, (1 << 8) | (1 << 9));
        bus.write16(BG0CNT, 0);
        bus.write16(BG0CNT + 2, 1 | (1 << 8));
        // BG0 tile 1: first pixel is palette index 1 (red).
        bus.write16(VRAM_BASE, 1);
        bus.write8(VRAM_BASE + 32, 1);
        bus.write16(PALETTE_BASE + 2, 0x001F);
        // BG1 screen block 1, tile 2: first pixel is palette index 2 (green).
        bus.write16(VRAM_BASE + 0x800, 2);
        bus.write8(VRAM_BASE + 64, 2);
        bus.write16(PALETTE_BASE + 4, 0x03E0);

        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
    }

    #[test]
    fn mode0_bg0_wins_ties_against_bg1() {
        let mut bus = bus();
        bus.write16(0x0400_0000, (1 << 8) | (1 << 9));
        bus.write16(BG0CNT, 0); // BG0 priority 0
        bus.write16(BG0CNT + 2, 0 | (1 << 8)); // BG1 priority 0, screen block 1
        bus.write16(VRAM_BASE, 1);
        bus.write8(VRAM_BASE + 32, 1);
        bus.write16(VRAM_BASE + 0x800, 2);
        bus.write8(VRAM_BASE + 64, 2);
        bus.write16(PALETTE_BASE + 2, 0x001F); // BG0 red
        bus.write16(PALETTE_BASE + 4, 0x03E0); // BG1 green

        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
    }

    #[test]
    fn mode0_disabled_bg_shows_palette_backdrop() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 0);
        bus.write16(PALETTE_BASE, 0x7C00); // blue backdrop
        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFF00_00FF));
    }
}
