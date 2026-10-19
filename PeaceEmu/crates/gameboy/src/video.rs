//! GBA renderer for text/affine background modes 0–2 and bitmap modes 3–5.
//!
//! This module renders the current video-memory snapshot, including tiled text
//! backgrounds, regular OBJ sprites, and bitmap modes. It does not yet advance
//! scanlines, render affine backgrounds, or generate VBlank/HBlank IRQs.

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
    if !forced_blank && mode > 5 {
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
        0..=2 => render_text_backgrounds(bus, display_control, &mut frame),
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

    if mode >= 3 {
        // Bitmap modes reserve VRAM through 0x06013FFF for the framebuffer;
        // OBJ character data therefore begins at 0x06014000 instead.
        for priority in (0..=3).rev() {
            render_regular_objects(bus, display_control, priority, &mut frame);
        }
    }

    Some(frame)
}

/// Composites regular text backgrounds from back to front (priority 3 to 0).
/// Affine backgrounds are not implemented yet.
fn render_text_backgrounds(bus: &MemoryBus, display_control: u16, frame: &mut VideoFrame) {
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
            let mode = display_control & 7;\n            if mode == 0 || bg < 2 {\n                render_text_background(bus, bg, control, frame);\n            } else if bg >= 2 && (mode == 2 || bg == 2) {\n                render_affine_background(bus, bg, control, frame);\n            }
        }
        // OBJ pixels have precedence over BG pixels at the same priority.
        render_regular_objects(bus, display_control, priority, frame);
    }
}

/// Renders an affine 8bpp tiled background (BG2/BG3 in modes 1 and 2).
/// Reference coordinates are signed 28-bit 8.8 fixed-point values; PA/PB/PC/PD
/// are signed 8.8 coefficients. The overflow bit controls wraparound outside
/// the affine background's square map.
fn render_affine_background(bus: &MemoryBus, bg: u32, control: u16, frame: &mut VideoFrame) {
    let register_base = 0x0400_0020 + (bg - 2) * 0x10;
    let pa = i32::from(bus.read16(register_base) as i16);
    let pb = i32::from(bus.read16(register_base + 2) as i16);
    let pc = i32::from(bus.read16(register_base + 4) as i16);
    let pd = i32::from(bus.read16(register_base + 6) as i16);
    let reference_x = sign_extend_28(
        u32::from(bus.read16(register_base + 8))
            | (u32::from(bus.read16(register_base + 10) & 0x0FFF) << 16),
    );
    let reference_y = sign_extend_28(
        u32::from(bus.read16(register_base + 12))
            | (u32::from(bus.read16(register_base + 14) & 0x0FFF) << 16),
    );
    let char_base = u32::from((control >> 2) & 3) * 0x4000;
    let screen_base = u32::from((control >> 8) & 0x1F) * 0x800;
    let size = 128_u32 << ((control >> 14) & 3);
    let wrap = control & (1 << 13) != 0;

    for y in 0..SCREEN_HEIGHT {
        let source_x_base = reference_x + pb * y as i32;
        let source_y_base = reference_y + pd * y as i32;
        for x in 0..SCREEN_WIDTH {
            let source_x = (source_x_base + pa * x as i32) >> 8;
            let source_y = (source_y_base + pc * x as i32) >> 8;
            let (source_x, source_y) = if wrap {
                (
                    source_x.rem_euclid(size as i32) as u32,
                    source_y.rem_euclid(size as i32) as u32,
                )
            } else if source_x < 0
                || source_y < 0
                || source_x >= size as i32
                || source_y >= size as i32
            {
                continue;
            } else {
                (source_x as u32, source_y as u32)
            };

            let tile_x = source_x >> 3;
            let tile_y = source_y >> 3;
            let map_width = size >> 3;
            let map_address = VRAM_BASE + screen_base + tile_y * map_width + tile_x;
            let tile_index = u32::from(bus.read8(map_address));
            let tile_address = VRAM_BASE
                + char_base
                + tile_index * 64
                + (source_y & 7) * 8
                + (source_x & 7);
            let palette_index = u32::from(bus.read8(tile_address));
            if palette_index != 0 {
                frame.pixels[y * SCREEN_WIDTH + x] =
                    rgb555(bus.read16(PALETTE_BASE + palette_index * 2));
            }
        }
    }
}

fn sign_extend_28(value: u32) -> i32 {
    ((value << 4) as i32) >> 4
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
    let mosaic_enabled = control & (1 << 6) != 0;
    let mosaic = bus.read16(0x0400_004C);
    let mosaic_width = u32::from((mosaic & 0xF) + 1);
    let mosaic_height = u32::from(((mosaic >> 4) & 0xF) + 1);
    let map_blocks_per_row = map_width / 256;

    for y in 0..SCREEN_HEIGHT {
        let screen_y = if mosaic_enabled {
            (y as u32 / mosaic_height) * mosaic_height
        } else {
            y as u32
        };
        let world_y = (screen_y + scroll_y) % map_height;
        let tile_y = world_y / 8;
        let in_tile_y = world_y & 7;
        for x in 0..SCREEN_WIDTH {
            let screen_x = if mosaic_enabled {
                (x as u32 / mosaic_width) * mosaic_width
            } else {
                x as u32
            };
            let world_x = (screen_x + scroll_x) % map_width;
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

/// Draws regular (non-affine) OBJ sprites for the requested priority.
/// OBJ is composited after backgrounds of the same priority; lower OAM indices
/// win ties, so entries are visited from 127 down to 0.
fn render_regular_objects(
    bus: &MemoryBus,
    display_control: u16,
    priority: u16,
    frame: &mut VideoFrame,
) {
    if display_control & (1 << 12) == 0 {
        return;
    }
    const OAM_BASE: u32 = 0x0700_0000;
    let mode = display_control & 7;
    let obj_vram_base: u32 = if mode >= 3 { 0x0601_4000 } else { 0x0601_0000 };
    let one_dimensional = display_control & (1 << 6) != 0;

    for index in (0..128_u32).rev() {
        let base = OAM_BASE + index * 8;
        let attr0 = bus.read16(base);
        let attr1 = bus.read16(base + 2);
        let attr2 = bus.read16(base + 4);

        // Affine sprites need matrix transforms and are deliberately skipped
        // until that part of the PPU is implemented. Disabled regular sprites
        // and prohibited shape 3 are likewise not drawable.
        if attr0 & (1 << 8) != 0
            || attr0 & (1 << 9) != 0
            || attr0 & (3 << 10) != 0
            || (attr0 >> 14) & 3 == 3
        {
            continue;
        }
        if u16::from((attr2 >> 10) & 3) != priority {
            continue;
        }

        let shape = (attr0 >> 14) & 3;
        let size = (attr1 >> 14) & 3;
        let (width, height) = match (shape, size) {
            (0, 0) => (8_u32, 8_u32),
            (0, 1) => (16, 16),
            (0, 2) => (32, 32),
            (0, 3) => (64, 64),
            (1, 0) => (16, 8),
            (1, 1) => (32, 8),
            (1, 2) => (32, 16),
            (1, 3) => (64, 32),
            (2, 0) => (8, 16),
            (2, 1) => (8, 32),
            (2, 2) => (16, 32),
            (2, 3) => (32, 64),
            _ => continue,
        };

        let raw_x = u32::from(attr1 & 0x01FF);
        let origin_x = if raw_x >= 256 {
            raw_x as i32 - 512
        } else {
            raw_x as i32
        };
        let raw_y = u32::from(attr0 & 0x00FF);
        let origin_y = if raw_y >= 256 - height {
            raw_y as i32 - 256
        } else {
            raw_y as i32
        };
        let color_8bpp = attr0 & (1 << 13) != 0;
        let tile_index = u32::from(attr2 & 0x03FF) & if color_8bpp { !1 } else { !0 };
        let palette_bank = u32::from((attr2 >> 12) & 0xF);
        let hflip = attr1 & (1 << 12) != 0;
        let vflip = attr1 & (1 << 13) != 0;
        let row_stride = if one_dimensional {
            (width / 8) * if color_8bpp { 2 } else { 1 }
        } else {
            32
        };

        for dy in 0..height {
            let screen_y = origin_y + dy as i32;
            if !(0..SCREEN_HEIGHT as i32).contains(&screen_y) {
                continue;
            }
            let source_y = if vflip { height - 1 - dy } else { dy };
            for dx in 0..width {
                let screen_x = origin_x + dx as i32;
                if !(0..SCREEN_WIDTH as i32).contains(&screen_x) {
                    continue;
                }
                let source_x = if hflip { width - 1 - dx } else { dx };
                let tile_x = source_x / 8;
                let tile_y = source_y / 8;
                let slot =
                    tile_index + tile_y * row_stride + tile_x * if color_8bpp { 2 } else { 1 };
                let address =
                    obj_vram_base + slot * 32 + (source_y & 7) * if color_8bpp { 8 } else { 4 };
                let palette_index = if color_8bpp {
                    u32::from(bus.read8(address + (source_x & 7)))
                } else {
                    let packed = bus.read8(address + (source_x & 7) / 2);
                    let nibble = if source_x & 1 == 0 {
                        packed & 0x0F
                    } else {
                        packed >> 4
                    };
                    if nibble == 0 {
                        continue;
                    }
                    palette_bank * 16 + u32::from(nibble)
                };
                if palette_index == 0 {
                    continue;
                }
                let color = bus.read16(PALETTE_BASE + 0x200 + palette_index * 2);
                frame.pixels[screen_y as usize * SCREEN_WIDTH + screen_x as usize] = rgb555(color);
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
    #[test]
    fn mode1_renders_affine_bg2_with_identity_transform() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 | (1 << 10)); // mode 1, BG2 enabled
        bus.write16(0x0400_000C, 0); // affine map at screen block 0
        bus.write8(VRAM_BASE, 1); // map entry selects tile 1
        bus.write8(VRAM_BASE + 64, 1); // tile 1 first pixel
        bus.write16(PALETTE_BASE + 2, 0x001F); // palette 1: red
        bus.write16(0x0400_0020, 0x0100); // PA = 1.0
        bus.write16(0x0400_0022, 0); // PB = 0
        bus.write16(0x0400_0024, 0); // PC = 0
        bus.write16(0x0400_0026, 0x0100); // PD = 1.0

        let frame = render_bitmap_frame(&bus).expect("mode 1 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(1, 0), Some(0xFF00_0000));
    }

    #[test]
    fn affine_background_wraps_coordinates_when_overflow_enabled() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 2 | (1 << 10)); // mode 2, BG2 enabled
        bus.write16(0x0400_000C, 1 << 13); // wrap enabled, 128x128 map
        bus.write16(0x0400_0020, 0x0100);
        bus.write16(0x0400_0026, 0x0100);
        // Start at x=-1 pixel (signed 28-bit 8.8 fixed point).
        bus.write16(0x0400_0028, 0xFF00);
        bus.write16(0x0400_002A, 0x0FFF);
        // The wrapped coordinate is the final pixel of the first map row.
        bus.write8(VRAM_BASE + 15, 1);
        bus.write8(VRAM_BASE + 64 + 7, 1);
        bus.write16(PALETTE_BASE + 2, 0x001F);

        let frame = render_bitmap_frame(&bus).expect("mode 2 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
    }

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
        // VRAM byte writes replicate across the containing halfword; use a
        // halfword write to preserve distinct adjacent 8bpp palette indices.
        bus.write16(VRAM_BASE + 64, 0x0403);
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
    fn mode0_bg_mosaic_repeats_pixels_and_rows() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 << 8); // BG0 enabled
        bus.write16(BG0CNT, 1 << 6); // BG0 mosaic enabled
        bus.write16(0x0400_004C, 1 | (1 << 4)); // 2x2 mosaic

        bus.write16(VRAM_BASE, 1);
        bus.write16(VRAM_BASE + 32, 0x4321);
        bus.write16(PALETTE_BASE + 2, 0x001F);
        bus.write16(PALETTE_BASE + 4, 0x03E0);
        bus.write16(PALETTE_BASE + 6, 0x7C00);
        bus.write16(PALETTE_BASE + 8, 0x7FFF);

        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(1, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(2, 0), Some(0xFF00_00FF));
        assert_eq!(frame.pixel(0, 1), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(1, 1), Some(0xFFFF_0000));
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

    #[test]
    fn mode0_renders_regular_4bpp_obj_over_background() {
        let mut bus = bus();
        bus.write16(0x0400_0000, (1 << 8) | (1 << 12)); // BG0 and OBJ enabled
        bus.write16(PALETTE_BASE, 0x7C00); // blue backdrop
        bus.write16(PALETTE_BASE + 2, 0x03E0); // BG palette index 1: green
        bus.write16(PALETTE_BASE + 0x202, 0x001F); // OBJ palette index 1: red
        bus.write16(VRAM_BASE, 1);
        bus.write8(VRAM_BASE + 32, 1); // BG tile 1: green

        bus.write16(0x0601_0000, 0x0001);
        // OAM entry 0: regular 8x8 sprite at (0,0), tile 0, priority 0.
        bus.write16(0x0700_0000, 0);
        bus.write16(0x0700_0002, 0);
        bus.write16(0x0700_0004, 0);

        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(1, 0), Some(0xFF00_FF00));
    }

    #[test]
    fn mode0_obj_transparency_preserves_background() {
        let mut bus = bus();
        bus.write16(0x0400_0000, (1 << 8) | (1 << 12));
        bus.write16(PALETTE_BASE, 0x7C00); // backdrop blue
        bus.write16(PALETTE_BASE + 0x202, 0x001F); // OBJ index 1 red

        bus.write16(0x0700_0000, 0);
        bus.write16(0x0700_0002, 0);
        bus.write16(0x0700_0004, 0);
        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFF00_00FF));
    }

    #[test]
    fn bitmap_mode4_renders_obj_with_obj_palette_and_transparency() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 4 | (1 << 12)); // mode 4, OBJ enabled
        bus.write16(VRAM_BASE, 0); // framebuffer index 0: backdrop color
        bus.write16(PALETTE_BASE, 0x7C00); // background palette index 0: blue
        bus.write16(PALETTE_BASE + 0x202, 0x001F); // OBJ palette index 1: red
        bus.write16(0x0601_4000, 0x0001); // OBJ tile pixel index 1
        bus.write16(0x0601_4002, 0x0000); // next pixels transparent
        bus.write16(0x0700_0000, 0);
        bus.write16(0x0700_0002, 0);
        bus.write16(0x0700_0004, 0);

        let frame = render_bitmap_frame(&bus).expect("mode 4 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(1, 0), Some(0xFF00_00FF));
    }

    #[test]
    fn bitmap_mode5_obj_uses_bitmap_mode_character_base() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 5 | (1 << 12)); // mode 5, OBJ enabled
        bus.write16(VRAM_BASE, 0x03E0); // underlying framebuffer pixel: green
        bus.write16(PALETTE_BASE + 0x202, 0x001F); // OBJ palette index 1: red
        bus.write16(0x0601_4000, 0x0001);
        bus.write16(0x0700_0000, 0);
        bus.write16(0x0700_0002, 0);
        bus.write16(0x0700_0004, 0);

        let frame = render_bitmap_frame(&bus).expect("mode 5 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(1, 0), Some(0xFF00_0000));
    }

    #[test]
    fn bitmap_mode3_renders_obj_from_bitmap_obj_vram_region() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 3 | (1 << 12)); // mode 3 and OBJ enabled
        bus.write16(VRAM_BASE, 0x03E0); // framebuffer pixel (0,0): green

        bus.write16(0x0601_4000, 0x0001);
        bus.write16(PALETTE_BASE + 0x202, 0x001F); // OBJ palette index 1: red
        bus.write16(0x0700_0000, 0);
        bus.write16(0x0700_0002, 0);
        bus.write16(0x0700_0004, 0);

        let frame = render_bitmap_frame(&bus).expect("mode 3 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(1, 0), Some(rgb555(bus.read16(VRAM_BASE + 2))));
    }

    #[test]
    fn bitmap_mode4_uses_selected_frame_page_and_palette() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 4 | (1 << 4)); // mode 4, page 1
        bus.write16(VRAM_BASE + 0xA000, 3);
        bus.write16(PALETTE_BASE + 6, 0x001F); // palette index 3: red

        let frame = render_bitmap_frame(&bus).expect("mode 4 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(1, 0), Some(0xFF00_0000));
    }

    #[test]
    fn bitmap_mode5_is_centered_and_uses_selected_page() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 5 | (1 << 4)); // mode 5, page 1
        bus.write16(VRAM_BASE + 0xA000, 0x001F); // first visible pixel: red

        let frame = render_bitmap_frame(&bus).expect("mode 5 should render");
        assert_eq!(frame.pixel(39, 16), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(40, 16), Some(0xFF00_0000));
        assert_eq!(frame.pixel(200, 16), Some(0xFF00_0000));
    }

    #[test]
    fn lower_oam_index_wins_obj_priority_ties() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 << 12); // OBJ enabled
        bus.write16(PALETTE_BASE + 0x202, 0x001F); // index 1 red
        bus.write16(PALETTE_BASE + 0x204, 0x03E0); // index 2 green
        bus.write16(0x0601_0000, 0x0001); // tile 0: red pixel
        bus.write16(0x0601_0020, 0x0002); // tile 1: green pixel
        // Entry 0 has the lower OAM index and must win.
        bus.write16(0x0700_0000, 0);
        bus.write16(0x0700_0002, 0);
        bus.write16(0x0700_0004, 0);
        bus.write16(0x0700_0008, 0);
        bus.write16(0x0700_000A, 0);
        bus.write16(0x0700_000C, 1);

        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
    }

    #[test]
    fn non_normal_obj_modes_are_not_drawn_as_opaque_sprites() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 << 12); // OBJ enabled
        bus.write16(PALETTE_BASE, 0x7C00); // blue backdrop
        bus.write16(PALETTE_BASE + 0x202, 0x001F); // OBJ index 1 red
        bus.write16(0x0601_0000, 0x0001);
        bus.write16(0x0700_0000, 1 << 10); // semi-transparent OBJ mode
        bus.write16(0x0700_0002, 0);
        bus.write16(0x0700_0004, 0);

        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFF00_00FF));
    }

    #[test]
    fn mode0_renders_8bpp_obj_pixels_and_zero_transparency() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 << 12); // OBJ enabled
        bus.write16(PALETTE_BASE + 0x202, 0x001F); // OBJ palette index 1: red
        bus.write16(PALETTE_BASE + 0x204, 0x03E0); // OBJ palette index 2: green
        bus.write16(0x0601_0000, 0x0201); // 8bpp pixels 1, 2
        bus.write16(0x0601_0002, 0x0000); // transparent pixels
        bus.write16(0x0700_0000, 1 << 13); // 8bpp, regular 8x8
        bus.write16(0x0700_0002, 0);
        bus.write16(0x0700_0004, 0);

        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(1, 0), Some(0xFF00_FF00));
        assert_eq!(frame.pixel(2, 0), Some(0xFF00_0000));
    }

    #[test]
    fn obj_rectangular_size_renders_expected_extent() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 << 12); // OBJ enabled
        bus.write16(PALETTE_BASE + 0x202, 0x001F);
        // Fill the first 16x8 sprite row with palette index 1.
        for offset in (0..64).step_by(2) {
            bus.write16(0x0601_0000 + offset, 0x1111);
        }
        bus.write16(0x0700_0000, 1 << 14); // horizontal shape, size 0 = 16x8
        bus.write16(0x0700_0002, 0);
        bus.write16(0x0700_0004, 0);

        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(15, 0), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(16, 0), Some(0xFF00_0000));
        assert_eq!(frame.pixel(15, 7), Some(0xFFFF_0000));
        assert_eq!(frame.pixel(0, 8), Some(0xFF00_0000));
    }

    #[test]
    fn mode0_obj_vflip_reverses_4bpp_rows() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 << 12); // OBJ enabled
        bus.write16(PALETTE_BASE + 0x202, 0x001F); // index 1 red
        bus.write16(PALETTE_BASE + 0x204, 0x03E0); // index 2 green

        bus.write16(0x0601_0000, 0x0001);
        bus.write16(0x0601_001C, 0x0002);
        bus.write16(0x0700_0000, 0);
        bus.write16(0x0700_0002, 1 << 13); // vertical flip
        bus.write16(0x0700_0004, 0);

        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFF00_FF00));
        assert_eq!(frame.pixel(0, 7), Some(0xFFFF_0000));
    }

    #[test]
    fn mode0_obj_hflip_reverses_4bpp_pixels() {
        let mut bus = bus();
        bus.write16(0x0400_0000, 1 << 12); // OBJ enabled
        bus.write16(PALETTE_BASE + 0x202, 0x001F); // index 1 red
        bus.write16(PALETTE_BASE + 0x204, 0x03E0); // index 2 green

        bus.write16(0x0601_0000, 0x0021);
        bus.write16(0x0601_0002, 0x0000);
        bus.write16(0x0601_0004, 0x0000);
        bus.write16(0x0601_0006, 0x0000);
        bus.write16(0x0601_0008, 0x0000);
        bus.write16(0x0601_000A, 0x0000);
        bus.write16(0x0601_000C, 0x0000);
        bus.write16(0x0601_000E, 0x0000);
        bus.write16(0x0700_0000, 0);
        bus.write16(0x0700_0002, 1 << 12); // horizontal flip
        bus.write16(0x0700_0004, 0);

        let frame = render_bitmap_frame(&bus).expect("mode 0 should render");
        assert_eq!(frame.pixel(0, 0), Some(0xFF00_0000));
        assert_eq!(frame.pixel(6, 0), Some(0xFF00_FF00));
        assert_eq!(frame.pixel(7, 0), Some(0xFFFF_0000));
    }
}
