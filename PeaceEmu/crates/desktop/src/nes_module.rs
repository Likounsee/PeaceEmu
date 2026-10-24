//! NES module adapter for PeaceEmu's shared emulation-core contract.
//!
//! This adapter has no window, event loop, filesystem access, or audio device.
//! The PeaceEmu host owns library launch, presentation, input, and persistence.

use emulation_core::content::{ContentFormatProbe, FormatProbeResult};
use emulation_core::core_io::{AudioBuffer, InputControl, InputEvent, PixelFormat, VideoFrame};
use emulation_core::{
    CoreCapabilities, CoreDescriptor, CoreError, CoreEvent, CoreRegistry, EmulationCore,
    RegistryError, SystemId, CURRENT_CORE_API_VERSION,
};
use peaceemu_nes::session::{NesConfig, NesSession, NES_FRAME_HEIGHT, NES_FRAME_WIDTH};

/// Read-only probe for iNES/NES 2.0 cartridge headers.
#[derive(Clone, Copy, Debug, Default)]
pub struct NesCartridgeProbe;

impl ContentFormatProbe for NesCartridgeProbe {
    fn format_id(&self) -> &'static str {
        "nes"
    }

    fn probe(&self, prefix: &[u8]) -> FormatProbeResult {
        if prefix.len() < 4 {
            return FormatProbeResult::NeedMoreData { minimum_bytes: 4 };
        }
        if prefix[..4] == *b"NES\x1A" {
            FormatProbeResult::Match
        } else {
            FormatProbeResult::NoMatch
        }
    }
}

/// PeaceEmu-owned adapter around the independent Rust NES core.
#[derive(Default)]
pub struct NesCoreAdapter {
    session: Option<NesSession>,
    image: Vec<u8>,
    pending_video: Option<VideoFrame>,
    pending_audio: Option<AudioBuffer>,
    controller_1: u8,
    save_dirty: bool,
}

impl NesCoreAdapter {
    /// Constructs an unloaded NES core instance for registration with PeaceEmu.
    pub fn new() -> Self {
        Self::default()
    }

    fn session(&self) -> Result<&NesSession, CoreError> {
        self.session.as_ref().ok_or(CoreError::NotInitialized)
    }

    fn session_mut(&mut self) -> Result<&mut NesSession, CoreError> {
        self.session.as_mut().ok_or(CoreError::NotInitialized)
    }
}

fn create_nes_core() -> Box<dyn EmulationCore> {
    Box::new(NesCoreAdapter::new())
}

/// Registers NES with the host using its truthful runtime capabilities.
pub fn register_nes(registry: &mut CoreRegistry) -> Result<(), RegistryError> {
    let descriptor = NesCoreAdapter::new().descriptor();
    registry.register_with_descriptor(descriptor, create_nes_core)
}

impl EmulationCore for NesCoreAdapter {
    fn descriptor(&self) -> CoreDescriptor {
        CoreDescriptor {
            api_version: CURRENT_CORE_API_VERSION,
            system_id: SystemId::new("nes").expect("static NES system ID is valid"),
            display_name: "Nintendo Entertainment System",
            core_version: peaceemu_nes::CORE_VERSION,
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
        // Build first, then replace the active session so a rejected ROM cannot
        // destroy the previously loaded game.
        let session = NesSession::load(content, NesConfig::default())
            .map_err(|_| CoreError::InvalidContent("invalid or unsupported NES cartridge"))?;
        self.image.clear();
        self.image.extend_from_slice(content);
        self.session = Some(session);
        self.pending_video = None;
        self.pending_audio = None;
        self.controller_1 = 0;
        self.save_dirty = false;
        Ok(())
    }

    fn reset(&mut self) -> Result<(), CoreError> {
        let previous = self.session()?;
        let saved_ram = if previous.has_battery_backed_ram() {
            Some(
                previous
                    .save_battery_ram()
                    .map_err(|_| CoreError::Execution("could not preserve NES battery RAM"))?,
            )
        } else {
            None
        };
        let mut replacement = NesSession::load(&self.image, previous.config())
            .map_err(|_| CoreError::Execution("could not reset NES session"))?;
        if let Some(data) = saved_ram.as_deref() {
            replacement
                .load_battery_ram(data)
                .map_err(|_| CoreError::Execution("could not restore NES battery RAM"))?;
        }
        self.session = Some(replacement);
        self.pending_video = None;
        self.pending_audio = None;
        self.controller_1 = 0;
        self.save_dirty = false;
        Ok(())
    }

    fn run_until_event(&mut self) -> Result<CoreEvent, CoreError> {
        let output = self
            .session_mut()?
            .advance_frame()
            .map_err(|_| CoreError::Execution("NES frame execution failed"))?;

        let mut rgba = Vec::with_capacity(NES_FRAME_WIDTH * NES_FRAME_HEIGHT * 4);
        for index in output.palette_indices {
            let [red, green, blue] = NES_PALETTE[usize::from(index & 0x3f)];
            rgba.extend_from_slice(&[red, green, blue, 255]);
        }
        self.pending_video = Some(
            VideoFrame::new(
                output.width as u32,
                output.height as u32,
                PixelFormat::Rgba8888,
                rgba,
            )
            .map_err(|_| CoreError::Execution("could not construct NES video frame"))?,
        );

        self.pending_audio = if output.audio_samples.is_empty() {
            None
        } else {
            let samples = output
                .audio_samples
                .into_iter()
                .map(|sample| {
                    let centered = (sample.clamp(0.0, 1.0) * 2.0) - 1.0;
                    (centered * f32::from(i16::MAX)) as i16
                })
                .collect();
            Some(
                AudioBuffer::new(self.session()?.config().audio_sample_rate, 1, samples)
                    .map_err(|_| CoreError::Execution("could not construct NES audio buffer"))?,
            )
        };
        if self.session()?.has_battery_backed_ram() {
            self.save_dirty = true;
        }
        Ok(CoreEvent::FrameReady)
    }

    fn save_game_data(&self) -> Result<Vec<u8>, CoreError> {
        let session = self.session()?;
        if !session.has_battery_backed_ram() {
            return Err(CoreError::Unsupported(
                "this NES cartridge has no battery-backed RAM",
            ));
        }
        session
            .save_battery_ram()
            .map_err(|_| CoreError::Execution("could not read NES battery RAM"))
    }

    fn load_game_data(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let session = self.session_mut()?;
        if !session.has_battery_backed_ram() {
            return Err(CoreError::Unsupported(
                "this NES cartridge has no battery-backed RAM",
            ));
        }
        session
            .load_battery_ram(data)
            .map_err(|_| CoreError::InvalidState("NES battery save has an invalid size"))?;
        self.save_dirty = false;
        Ok(())
    }

    fn persistent_save_dirty(&self) -> bool {
        self.save_dirty
    }

    fn clear_persistent_save_dirty(&mut self) {
        self.save_dirty = false;
    }

    fn send_input(&mut self, event: InputEvent) -> Result<(), CoreError> {
        let bit = match event.control {
            InputControl::Action1 => 0,
            InputControl::Action2 => 1,
            InputControl::Select | InputControl::Action3 => 2,
            InputControl::Start | InputControl::Action4 => 3,
            InputControl::Up => 4,
            InputControl::Down => 5,
            InputControl::Left => 6,
            InputControl::Right => 7,
        };
        if event.pressed {
            self.controller_1 |= 1 << bit;
        } else {
            self.controller_1 &= !(1 << bit);
        }
        let buttons = self.controller_1;
        self.session_mut()?.set_controller_state(0, buttons);
        Ok(())
    }

    fn take_video_frame(&mut self) -> Result<Option<VideoFrame>, CoreError> {
        Ok(self.pending_video.take())
    }

    fn take_audio_buffer(&mut self) -> Result<Option<AudioBuffer>, CoreError> {
        Ok(self.pending_audio.take())
    }
}

/// A practical NTSC-like 64-color palette used to convert NES indices for the host renderer.
const NES_PALETTE: [[u8; 3]; 64] = [
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

#[cfg(test)]
mod tests {
    use super::*;

    fn looping_rom() -> Vec<u8> {
        let mut rom = vec![0; 16 + 16 * 1024];
        rom[..4].copy_from_slice(b"NES\x1A");
        rom[4] = 1;
        rom[16] = 0x4c;
        rom[17] = 0x00;
        rom[18] = 0x80;
        rom[16 + 0x3ffc] = 0x00;
        rom[16 + 0x3ffd] = 0x80;
        rom
    }

    #[test]
    fn probe_requires_complete_magic_and_recognizes_ines() {
        let probe = NesCartridgeProbe;
        assert_eq!(
            probe.probe(b"NES"),
            FormatProbeResult::NeedMoreData { minimum_bytes: 4 }
        );
        assert_eq!(probe.probe(b"NES\x1A"), FormatProbeResult::Match);
        assert_eq!(probe.probe(b"NOPE"), FormatProbeResult::NoMatch);
    }

    #[test]
    fn adapter_loads_a_rom_and_emits_host_owned_video_and_audio() {
        let mut core = NesCoreAdapter::new();
        assert_eq!(core.descriptor().system_id.as_str(), "nes");
        core.load_content(&looping_rom()).unwrap();
        assert_eq!(core.run_until_event().unwrap(), CoreEvent::FrameReady);
        let frame = core.take_video_frame().unwrap().unwrap();
        assert_eq!((frame.width(), frame.height()), (256, 240));
        assert_eq!(frame.pixels().len(), 256 * 240 * 4);
        assert!(core.take_audio_buffer().unwrap().is_some());
    }

    #[test]
    fn library_launch_pipeline_selects_and_starts_the_registered_nes_core() {
        let mut registry = CoreRegistry::new();
        register_nes(&mut registry).unwrap();
        let probe = NesCartridgeProbe;
        let probes: [&dyn ContentFormatProbe; 1] = [&probe];
        let session = emulation_core::runtime::launch_content(
            &looping_rom(),
            emulation_core::content::ContentLimits::new(32 * 1024 * 1024),
            &probes,
            &registry,
        )
        .unwrap();

        assert_eq!(session.descriptor().system_id.as_str(), "nes");
        assert_eq!(
            session.state(),
            emulation_core::runtime::SessionState::Running
        );
    }

    #[test]
    fn adapter_maps_host_input_to_nes_controller_bits() {
        let mut core = NesCoreAdapter::new();
        core.load_content(&looping_rom()).unwrap();
        core.send_input(InputEvent {
            control: InputControl::Action1,
            pressed: true,
        })
        .unwrap();
        assert_eq!(core.controller_1, 1);
        core.send_input(InputEvent {
            control: InputControl::Action1,
            pressed: false,
        })
        .unwrap();
        assert_eq!(core.controller_1, 0);
    }
}
