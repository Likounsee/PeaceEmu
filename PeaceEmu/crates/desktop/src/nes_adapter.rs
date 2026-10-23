//! Adapter that exposes the independently developed NES module through PeaceEmu's core contract.
//!
//! All launching, frame scheduling, rendering, input and persistence remain owned by PeaceEmu.

use emulation_core::content::FormatProbeResult;
use emulation_core::core_io::{AudioBuffer, InputControl, InputEvent, PixelFormat, VideoFrame};
use emulation_core::{
    CoreApiVersion, CoreCapabilities, CoreDescriptor, CoreError, CoreEvent, EmulationCore, SystemId,
};
use peaceemu_nes::{
    session::{NesConfig, NesSession},
    CORE_VERSION, SYSTEM_ID,
};

const AUDIO_RATE: u32 = 44_100;
const FRAME_WIDTH: u32 = 256;
const FRAME_HEIGHT: u32 = 240;

/// Detects the standard iNES header without relying on a filename extension.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NesCartridgeProbe;

impl emulation_core::content::ContentFormatProbe for NesCartridgeProbe {
    fn format_id(&self) -> &'static str {
        SYSTEM_ID
    }

    fn probe(&self, prefix: &[u8]) -> FormatProbeResult {
        if prefix.len() < 4 {
            return FormatProbeResult::NeedMoreData { minimum_bytes: 4 };
        }
        if &prefix[..4] != b"NES\x1a" {
            return FormatProbeResult::NoMatch;
        }
        if prefix.len() < 16 {
            return FormatProbeResult::NeedMoreData { minimum_bytes: 16 };
        }
        FormatProbeResult::Match
    }
}

/// Converts NES palette indices to RGBA pixels for PeaceEmu's generic renderer.
fn palette_rgba(index: u8) -> [u8; 4] {
    const PALETTE: [[u8; 3]; 64] = [
        [84, 84, 84],
        [0, 30, 116],
        [8, 16, 144],
        [48, 0, 136],
        [68, 0, 100],
        [92, 0, 48],
        [84, 4, 0],
        [60, 24, 0],
        [32, 42, 0],
        [8, 58, 0],
        [0, 64, 0],
        [0, 60, 0],
        [0, 50, 60],
        [0, 0, 0],
        [0, 0, 0],
        [0, 0, 0],
        [152, 150, 152],
        [8, 76, 196],
        [48, 50, 236],
        [92, 30, 228],
        [136, 20, 176],
        [160, 20, 100],
        [152, 34, 32],
        [120, 60, 0],
        [84, 90, 0],
        [40, 114, 0],
        [8, 124, 0],
        [0, 118, 40],
        [0, 102, 120],
        [0, 0, 0],
        [0, 0, 0],
        [0, 0, 0],
        [236, 238, 236],
        [76, 154, 236],
        [120, 124, 236],
        [176, 98, 236],
        [228, 84, 236],
        [236, 88, 180],
        [236, 106, 100],
        [212, 136, 32],
        [160, 170, 0],
        [116, 196, 0],
        [76, 208, 32],
        [56, 204, 108],
        [56, 180, 204],
        [60, 60, 60],
        [0, 0, 0],
        [0, 0, 0],
        [236, 238, 236],
        [168, 204, 236],
        [188, 188, 236],
        [212, 178, 236],
        [236, 174, 236],
        [236, 174, 212],
        [236, 180, 176],
        [228, 196, 144],
        [204, 210, 120],
        [180, 222, 120],
        [168, 226, 144],
        [152, 226, 180],
        [160, 214, 228],
        [160, 162, 160],
        [0, 0, 0],
        [0, 0, 0],
    ];
    let [r, g, b] = PALETTE[usize::from(index & 0x3f)];
    [r, g, b, 255]
}

fn core_error(_error: impl std::fmt::Display) -> CoreError {
    CoreError::Execution("NES module failed to execute the current game")
}

/// NES implementation of PeaceEmu's shared core interface.
pub(crate) struct NesCoreAdapter {
    rom: Vec<u8>,
    session: Option<NesSession>,
    video: Option<VideoFrame>,
    audio: Option<AudioBuffer>,
    buttons: u8,
    save_dirty: bool,
}

impl NesCoreAdapter {
    pub(crate) fn new() -> Self {
        Self {
            rom: Vec::new(),
            session: None,
            video: None,
            audio: None,
            buttons: 0,
            save_dirty: false,
        }
    }

    fn session(&self) -> Result<&NesSession, CoreError> {
        self.session.as_ref().ok_or(CoreError::NotInitialized)
    }

    fn session_mut(&mut self) -> Result<&mut NesSession, CoreError> {
        self.session.as_mut().ok_or(CoreError::NotInitialized)
    }
}

impl EmulationCore for NesCoreAdapter {
    fn descriptor(&self) -> CoreDescriptor {
        CoreDescriptor {
            api_version: CoreApiVersion { major: 1, minor: 0 },
            system_id: SystemId::new(SYSTEM_ID).expect("NES system ID is a valid constant"),
            display_name: "Nintendo Entertainment System",
            core_version: CORE_VERSION,
            capabilities: CoreCapabilities {
                content_loading: true,
                persistent_saves: true,
                save_states: false,
                video_output: true,
                audio_output: true,
                input: true,
            },
        }
    }

    fn load_content(&mut self, content: &[u8]) -> Result<(), CoreError> {
        let session = NesSession::load(
            content,
            NesConfig {
                audio_sample_rate: AUDIO_RATE,
            },
        )
        .map_err(|_| CoreError::InvalidContent("invalid or unsupported iNES cartridge"))?;
        self.rom.clear();
        self.rom.extend_from_slice(content);
        self.session = Some(session);
        self.video = None;
        self.audio = None;
        self.buttons = 0;
        self.save_dirty = false;
        Ok(())
    }

    fn reset(&mut self) -> Result<(), CoreError> {
        if self.rom.is_empty() {
            return Err(CoreError::NotInitialized);
        }
        let saved_ram = self.session()?.save_battery_ram().map_err(core_error)?;
        let mut session = NesSession::load(
            &self.rom,
            NesConfig {
                audio_sample_rate: AUDIO_RATE,
            },
        )
        .map_err(|_| CoreError::InvalidContent("invalid or unsupported iNES cartridge"))?;
        if !saved_ram.is_empty() {
            session.load_battery_ram(&saved_ram).map_err(core_error)?;
        }
        self.session = Some(session);
        self.video = None;
        self.audio = None;
        self.buttons = 0;
        Ok(())
    }

    fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
        let output = self.session_mut()?.advance_frame().map_err(core_error)?;
        let mut rgba = Vec::with_capacity(output.palette_indices.len() * 4);
        for index in output.palette_indices {
            rgba.extend_from_slice(&palette_rgba(index));
        }
        self.video = Some(
            VideoFrame::new(FRAME_WIDTH, FRAME_HEIGHT, PixelFormat::Rgba8888, rgba)
                .map_err(|_| CoreError::Execution("NES video frame has an invalid shape"))?,
        );
        self.audio = if output.audio_samples.is_empty() {
            None
        } else {
            let samples = output
                .audio_samples
                .into_iter()
                .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .collect();
            Some(
                AudioBuffer::new(AUDIO_RATE, 1, samples)
                    .map_err(|_| CoreError::Execution("NES audio buffer has an invalid format"))?,
            )
        };
        if self.session()?.has_battery_backed_ram() {
            self.save_dirty = true;
        }
        Ok(CoreEvent::FrameReady)
    }

    fn save_game_data(&self) -> Result<Vec<u8>, CoreError> {
        self.session()?.save_battery_ram().map_err(core_error)
    }

    fn load_game_data(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let session = self.session_mut()?;
        if !session.has_battery_backed_ram() {
            return if data.is_empty() {
                Ok(())
            } else {
                Err(CoreError::InvalidState(
                    "cartridge has no battery-backed RAM",
                ))
            };
        }
        session.load_battery_ram(data).map_err(core_error)
    }

    fn persistent_save_dirty(&self) -> bool {
        self.save_dirty
    }

    fn clear_persistent_save_dirty(&mut self) {
        self.save_dirty = false;
    }

    fn send_input(&mut self, event: InputEvent) -> Result<(), CoreError> {
        let bit = match event.control {
            InputControl::Action1 => Some(0),
            InputControl::Action2 => Some(1),
            InputControl::Select => Some(2),
            InputControl::Start => Some(3),
            InputControl::Up => Some(4),
            InputControl::Down => Some(5),
            InputControl::Left => Some(6),
            InputControl::Right => Some(7),
            InputControl::Action3 | InputControl::Action4 => None,
        };
        if let Some(bit) = bit {
            if event.pressed {
                self.buttons |= 1 << bit;
            } else {
                self.buttons &= !(1 << bit);
            }
            self.session_mut()?.set_controller_state(0, self.buttons);
        } else {
            let _ = self.session()?;
        }
        Ok(())
    }

    fn take_video_frame(&mut self) -> Result<Option<VideoFrame>, CoreError> {
        Ok(self.video.take())
    }

    fn take_audio_buffer(&mut self) -> Result<Option<AudioBuffer>, CoreError> {
        Ok(self.audio.take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emulation_core::content::ContentFormatProbe;

    #[test]
    fn nes_probe_requires_a_complete_ines_header() {
        let probe = NesCartridgeProbe;
        assert_eq!(
            probe.probe(b"NES"),
            FormatProbeResult::NeedMoreData { minimum_bytes: 4 }
        );
        assert_eq!(probe.probe(b"NOPE"), FormatProbeResult::NoMatch);
        let mut header = [0; 16];
        header[..4].copy_from_slice(b"NES\x1a");
        assert_eq!(probe.probe(&header), FormatProbeResult::Match);
    }

    #[test]
    fn registered_nes_module_launches_ines_content_through_host_pipeline() {
        use emulation_core::content::ContentLimits;
        use emulation_core::host_runtime::{HostRuntime, HostRuntimeConfig};
        use emulation_core::{CoreRegistry, SystemId};

        let mut rom = vec![0; 16 + 16 * 1024];
        rom[..4].copy_from_slice(b"NES\x1a");
        rom[4] = 1;
        rom[16] = 0x4c;
        rom[17] = 0;
        rom[18] = 0x80;
        rom[16 + 0x3ffc] = 0;
        rom[16 + 0x3ffd] = 0x80;

        let mut registry = CoreRegistry::new();
        registry
            .register(SystemId::new(SYSTEM_ID).unwrap(), || {
                Box::new(NesCoreAdapter::new())
            })
            .unwrap();
        let probe = NesCartridgeProbe;
        let probes: [&dyn ContentFormatProbe; 1] = [&probe];
        let mut host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();

        let session_id = host
            .launch_content(&rom, ContentLimits::new(32 * 1024), &probes, &registry)
            .unwrap();

        assert_eq!(host.session_count(), 1);
        assert_eq!(host.session_ids(), vec![session_id]);
    }

    #[test]
    fn adapter_exposes_a_frame_to_the_host_contract() {
        let mut rom = vec![0; 16 + 16 * 1024];
        rom[..4].copy_from_slice(b"NES\x1a");
        rom[4] = 1;
        rom[16] = 0x4c;
        rom[17] = 0;
        rom[18] = 0x80;
        rom[16 + 0x3ffc] = 0;
        rom[16 + 0x3ffd] = 0x80;
        let mut core = NesCoreAdapter::new();
        core.load_content(&rom).unwrap();
        assert_eq!(core.run_until_event().unwrap(), CoreEvent::FrameReady);
        let frame = core.take_video_frame().unwrap().unwrap();
        assert_eq!((frame.width(), frame.height()), (256, 240));
        assert_eq!(frame.pixels().len(), 256 * 240 * 4);
    }
}
