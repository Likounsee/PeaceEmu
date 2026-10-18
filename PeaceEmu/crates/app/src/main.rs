#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Minimal terminal host for inspecting registered systems and controlling a session.

use std::error::Error;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use emulation_core::content::{ContentFormatProbe, ContentLimits};
use emulation_core::core_io::{InputControl, InputEvent};
use emulation_core::host_runtime::{HostRuntime, HostRuntimeConfig, SessionId};
use emulation_core::save_state_store::SaveStateStore;
use emulation_core::CoreRegistry;
use game_boy::{register_game_boy_color, GameBoyColorCartridgeProbe};
use host_app::controller::ApplicationController;

use host_app::library::ContentLibraryError;

const MAX_CONTENT_BYTES: usize = 32 * 1024 * 1024;

/// Starts the command-line host.
fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        print_usage();
        return Ok(());
    }

    let mut registry = CoreRegistry::new();
    register_game_boy_color(&mut registry)?;
    if args.len() == 1 && args[0] == "--list-systems" {
        list_systems(&registry);
        return Ok(());
    }
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        print_usage();
        return Ok(());
    }
    let options = parse_launch_options(&args)
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;

    let probe = GameBoyColorCartridgeProbe;
    if let Some(selected) = options.selected_system.as_deref() {
        validate_system_selection(&registry, selected, probe.format_id())
            .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    }
    let probes: [&dyn ContentFormatProbe; 1] = [&probe];
    let host = HostRuntime::new(HostRuntimeConfig {
        max_sessions: options.max_sessions,
    })?;
    let mut controller = ApplicationController::new(host);
    let session_id = controller.launch_content_path(
        &options.path,
        ContentLimits::new(MAX_CONTENT_BYTES),
        &probes,
        &registry,
    )?;
    println!("Content loaded into host session {}.", session_id.get());
    println!("Host session limit: {}.", controller.max_sessions());
    println!("This core is an early implementation, not yet a playable emulator.");
    println!("Commands: status, sessions, switch <id>, open <path>, import <path>, library, forget <path>, close [id|all], input <control> <press|release>, release-inputs, inputs, media, run <events>, save-state <path>, load-state <path>, pause, resume, reset, stop, step, quit");
    match controller.import_content(&options.path, MAX_CONTENT_BYTES as u64) {
        Ok(entry) => println!(
            "Library: imported '{}' ({} bytes).",
            entry.title(),
            entry.size_bytes()
        ),
        Err(error) => eprintln!("Could not register launched content in library: {error}"),
    }
    command_loop(&mut controller, &probe, &registry)
}

#[derive(Debug, Eq, PartialEq)]
struct LaunchOptions {
    path: PathBuf,
    selected_system: Option<String>,
    max_sessions: usize,
}

fn parse_launch_options(args: &[std::ffi::OsString]) -> Result<LaunchOptions, String> {
    let mut path = None;
    let mut selected_system = None;
    let mut max_sessions = HostRuntimeConfig::default().max_sessions;
    let mut saw_max_sessions = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].to_string_lossy().as_ref() {
            "--system" => {
                if selected_system.is_some() {
                    return Err("--system may only be specified once".to_owned());
                }
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing system ID after --system".to_owned())?;
                let value = value.to_string_lossy().into_owned();
                if value.is_empty() || value.starts_with('-') {
                    return Err("system ID must be a non-empty value".to_owned());
                }
                selected_system = Some(value);
                index += 2;
            }
            "--max-sessions" => {
                if saw_max_sessions {
                    return Err("--max-sessions may only be specified once".to_owned());
                }
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| "missing count after --max-sessions".to_owned())?;
                let parsed = raw
                    .to_string_lossy()
                    .parse::<usize>()
                    .map_err(|_| "max sessions must be a positive integer".to_owned())?;
                if parsed == 0 {
                    return Err("max sessions must be greater than zero".to_owned());
                }
                max_sessions = parsed;
                saw_max_sessions = true;
                index += 2;
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option: {value}"));
            }
            _ => {
                if path.is_some() {
                    return Err("expected exactly one content path".to_owned());
                }
                path = Some(PathBuf::from(&args[index]));
                index += 1;
            }
        }
    }

    let path = path.ok_or_else(|| "missing content path".to_owned())?;
    Ok(LaunchOptions {
        path,
        selected_system,
        max_sessions,
    })
}

fn list_systems(registry: &CoreRegistry) {
    for id in registry.system_ids() {
        if let Some(descriptor) = registry.registered_descriptor(id) {
            let capabilities = descriptor.capabilities;
            println!("{} — {}", id.as_str(), descriptor.display_name);
            println!(
                "  content={} input={} video={} audio={} save_states={} persistent_saves={}",
                capabilities.content_loading,
                capabilities.input,
                capabilities.video_output,
                capabilities.audio_output,
                capabilities.save_states,
                capabilities.persistent_saves
            );
        } else {
            println!("{} — metadata unavailable", id.as_str());
        }
    }
}

fn command_loop(
    controller: &mut ApplicationController,
    probe: &dyn ContentFormatProbe,
    registry: &CoreRegistry,
) -> Result<(), Box<dyn Error>> {
    let (host, library, active_state) = controller.parts_mut();
    let mut active = *active_state;
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        print!("host-app> ");
        io::stdout().flush()?;
        let Some(line) = lines.next() else {
            host.close_all_sessions();
            active = None;
            break;
        };
        let line = line?;
        let command = line.trim();
        if command == "library" {
            println!(
                "Content library: {} item(s). Use play <index> to launch one.",
                library.len()
            );
            if library.is_empty() {
                println!("Content library is empty.");
            } else {
                for (index, entry) in library.entries().enumerate() {
                    println!(
                        "{}. {} ({} bytes) — {}",
                        index + 1,
                        entry.title(),
                        entry.size_bytes(),
                        entry.path().display()
                    );
                }
            }
            continue;
        }
        if let Some(path) = command.strip_prefix("import ") {
            match library.import(path.trim(), MAX_CONTENT_BYTES as u64) {
                Ok(entry) => println!(
                    "Imported '{}' ({} bytes).",
                    entry.title(),
                    entry.size_bytes()
                ),
                Err(error) => eprintln!("Could not import content: {error}"),
            }
            continue;
        }
        if let Some(path) = command.strip_prefix("forget ") {
            match library.remove(path.trim()) {
                Ok(entry) => println!("Removed '{}' from the library.", entry.title()),
                Err(error) => eprintln!("Could not remove library entry: {error}"),
            }
            continue;
        }
        if let Some(raw) = command.strip_prefix("play ") {
            let index = match raw.trim().parse::<usize>() {
                Ok(index) if index > 0 => index,
                _ => {
                    eprintln!("Expected play <library-index>, starting at 1.");
                    continue;
                }
            };
            let Some(path) = library_entry_path(library, index) else {
                eprintln!("No library entry at index {index}.");
                continue;
            };
            match host.launch_content_path(
                &path,
                ContentLimits::new(MAX_CONTENT_BYTES),
                &[probe],
                registry,
            ) {
                Ok(id) => {
                    active = Some(id);
                    println!(
                        "Opened library entry {} ('{}') in session {}.",
                        index,
                        path.file_stem()
                            .unwrap_or_else(|| path.as_os_str())
                            .to_string_lossy(),
                        id.get()
                    );
                }
                Err(error) => eprintln!("Could not launch library entry {index}: {error}"),
            }
            continue;
        }
        if command == "sessions" {
            for id in host.session_ids() {
                let session = host.session(id)?;
                let marker = if active == Some(id) { "*" } else { " " };
                println!(
                    "{marker} {} system={} state={:?}",
                    id.get(),
                    session.descriptor().system_id.as_str(),
                    session.state()
                );
            }
            continue;
        }
        if let Some(raw) = command.strip_prefix("switch ") {
            let wanted = raw.trim().parse::<u64>().ok();
            match wanted.and_then(|n| host.session_ids().into_iter().find(|id| id.get() == n)) {
                Some(id) => active = Some(id),
                None => eprintln!("Unknown or invalid session ID: {}", raw.trim()),
            }
            continue;
        }
        if let Some(path) = command.strip_prefix("open ") {
            match host.launch_content_path(
                PathBuf::from(path.trim()),
                ContentLimits::new(MAX_CONTENT_BYTES),
                &[probe],
                registry,
            ) {
                Ok(id) => {
                    active = Some(id);
                    match library.import(path.trim(), MAX_CONTENT_BYTES as u64) {
                        Ok(entry) => println!(
                            "Opened session {} and imported '{}'.",
                            id.get(),
                            entry.title()
                        ),
                        Err(ContentLibraryError::AlreadyImported(_)) => {
                            println!("Opened session {}.", id.get())
                        }
                        Err(error) => {
                            println!("Opened session {}.", id.get());
                            eprintln!("Could not register content in library: {error}");
                        }
                    }
                }
                Err(error) => eprintln!("Could not open content: {error}"),
            }
            continue;
        }
        if let Some(raw) = command.strip_prefix("input ") {
            match apply_input_command(host, active, raw) {
                Ok(message) => println!("{message}"),
                Err(error) => eprintln!("Input failed: {error}"),
            }
            continue;
        }
        if command == "release-inputs" {
            match release_session_inputs(host, active) {
                Ok(()) => println!("All held inputs released."),
                Err(error) => eprintln!("Could not release inputs: {error}"),
            }
            continue;
        }
        if command == "inputs" {
            match inspect_inputs(host, active) {
                Ok(report) => println!("{report}"),
                Err(error) => eprintln!("Input inspection failed: {error}"),
            }
            continue;
        }
        if command == "media" {
            match inspect_media(host, active) {
                Ok(report) => println!("{report}"),
                Err(error) => eprintln!("Media retrieval failed: {error}"),
            }
            continue;
        }
        if let Some(raw) = command.strip_prefix("run ") {
            match run_session_events(host, active, raw) {
                Ok(report) => println!("{report}"),
                Err(error) => eprintln!("Run failed: {error}"),
            }
            continue;
        }
        if let Some(path) = command.strip_prefix("save-state ") {
            match save_state_to_path(host, active, path.trim()) {
                Ok(()) => println!("Save state written."),
                Err(error) => eprintln!("Could not save state: {error}"),
            }
            continue;
        }
        if let Some(path) = command.strip_prefix("load-state ") {
            match load_state_from_path(host, active, path.trim()) {
                Ok(()) => println!("Save state restored."),
                Err(error) => eprintln!("Could not load state: {error}"),
            }
            continue;
        }
        if command == "close all" {
            let closed = close_all_sessions(host, &mut active);
            println!("Closed {closed} session(s).");
            continue;
        }
        if command == "close" || command.starts_with("close ") {
            let target = if let Some(raw) = command.strip_prefix("close ") {
                raw.trim()
                    .parse::<u64>()
                    .ok()
                    .and_then(|n| host.session_ids().into_iter().find(|id| id.get() == n))
            } else {
                active
            };
            match target {
                Some(id) => {
                    host.close_session(id)?;
                    if active == Some(id) {
                        active = host.session_ids().into_iter().next();
                    }
                    println!("Closed session {}.", id.get());
                }
                None => eprintln!("No matching session to close."),
            }
            continue;
        }
        if command == "quit" || command == "exit" {
            host.close_all_sessions();
            active = None;
            break;
        }
        let Some(id) = active else {
            eprintln!("No active session. Use open <content-path> first.");
            continue;
        };
        match command {
            "status" => {
                let session = host.session(id)?;
                let caps = session.descriptor().capabilities;
                println!("session={} state={:?}", id.get(), session.state());
                println!(
                    "capabilities: input={} video={} audio={} save_states={} persistent_saves={}",
                    caps.input,
                    caps.video_output,
                    caps.audio_output,
                    caps.save_states,
                    caps.persistent_saves
                );
            }
            "pause" | "resume" | "reset" | "stop" | "step" => {
                match apply_session_command(host, id, command) {
                    Ok(message) => println!("{message}"),
                    Err(error) => eprintln!("Command failed: {error}"),
                }
            }
            "" => {}
            other => eprintln!(
                "Unknown command: {other}. Use status, sessions, switch <id>, open <path>, import <path>, library, play <index>, forget <path>, close [id|all], input <control> <press|release>, release-inputs, media, run <events>, save-state <path>, load-state <path>, pause, resume, reset, stop, step, or quit."
            ),
        }
    }
    *active_state = active;
    Ok(())
}

fn library_entry_path(
    library: &host_app::library::ContentLibrary,
    index: usize,
) -> Option<PathBuf> {
    if index == 0 {
        return None;
    }
    library
        .entries()
        .nth(index - 1)
        .map(|entry| entry.path().to_path_buf())
}

fn close_all_sessions(host: &mut HostRuntime, active: &mut Option<SessionId>) -> usize {
    let closed = host.close_all_sessions();
    *active = None;
    closed
}
const MAX_RUN_EVENT_BUDGET: usize = 10_000;

fn parse_event_budget(raw: &str) -> Result<usize, String> {
    let budget = raw
        .trim()
        .parse::<usize>()
        .map_err(|_| "expected a positive event budget (1..=10000)".to_owned())?;
    if budget == 0 || budget > MAX_RUN_EVENT_BUDGET {
        return Err(format!(
            "event budget must be between 1 and {MAX_RUN_EVENT_BUDGET}"
        ));
    }
    Ok(budget)
}

fn run_session_events(
    host: &mut HostRuntime,
    active: Option<SessionId>,
    raw_budget: &str,
) -> Result<String, String> {
    let budget = parse_event_budget(raw_budget)?;
    let id = active.ok_or_else(|| "no active session".to_owned())?;
    let report = host
        .session_mut(id)
        .map_err(|error| error.to_string())?
        .run_for_events(budget)
        .map_err(|error| error.to_string())?;
    Ok(format!(
        "session={} processed={} last_event={:?} budget_exhausted={}",
        id.get(),
        report.events_processed,
        report.last_event,
        report.budget_exhausted
    ))
}

fn parse_input_control(raw: &str) -> Option<InputControl> {
    match raw {
        "up" => Some(InputControl::Up),
        "down" => Some(InputControl::Down),
        "left" => Some(InputControl::Left),
        "right" => Some(InputControl::Right),
        "action1" => Some(InputControl::Action1),
        "action2" => Some(InputControl::Action2),
        "action3" => Some(InputControl::Action3),
        "action4" => Some(InputControl::Action4),
        "start" => Some(InputControl::Start),
        "select" => Some(InputControl::Select),
        _ => None,
    }
}

fn apply_input_command(
    host: &mut HostRuntime,
    active: Option<SessionId>,
    raw: &str,
) -> Result<String, String> {
    let mut parts = raw.split_whitespace();
    let control_name = parts
        .next()
        .ok_or_else(|| "expected input <control> <press|release>".to_owned())?;
    let action = parts
        .next()
        .ok_or_else(|| "expected input <control> <press|release>".to_owned())?;
    if parts.next().is_some() {
        return Err("expected exactly two arguments: <control> <press|release>".to_owned());
    }
    let control = parse_input_control(control_name)
        .ok_or_else(|| format!("unknown control '{control_name}'"))?;
    let pressed = match action {
        "press" => true,
        "release" => false,
        _ => {
            return Err(format!(
                "unknown input action '{action}'; use press or release"
            ))
        }
    };
    let id = active.ok_or_else(|| "no active session".to_owned())?;
    host.session_mut(id)
        .map_err(|error| error.to_string())?
        .send_input(InputEvent { control, pressed })
        .map_err(|error| error.to_string())?;
    Ok(format!("{} {}.", control_name, action))
}

fn release_session_inputs(host: &mut HostRuntime, active: Option<SessionId>) -> Result<(), String> {
    let id = active.ok_or_else(|| "no active session".to_owned())?;
    host.session_mut(id)
        .map_err(|error| error.to_string())?
        .release_all_inputs()
        .map_err(|error| error.to_string())
}

fn inspect_inputs(host: &mut HostRuntime, active: Option<SessionId>) -> Result<String, String> {
    let id = active.ok_or_else(|| "no active session".to_owned())?;
    let session = host.session(id).map_err(|error| error.to_string())?;
    if !session.descriptor().capabilities.input {
        return Ok("input: unsupported by this core".to_owned());
    }
    let pressed = session.pressed_inputs();
    if pressed.is_empty() {
        return Ok("held inputs: none".to_owned());
    }
    let names = pressed
        .iter()
        .map(|control| format!("{control:?}").to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!("held inputs: {names}"))
}

fn inspect_media(host: &mut HostRuntime, active: Option<SessionId>) -> Result<String, String> {
    let id = active.ok_or_else(|| "no active session".to_owned())?;
    let session = host.session_mut(id).map_err(|error| error.to_string())?;
    let capabilities = session.descriptor().capabilities;
    let mut report = Vec::with_capacity(2);

    if capabilities.video_output {
        match session
            .take_video_frame()
            .map_err(|error| error.to_string())?
        {
            Some(frame) => report.push(format!(
                "video: {}x{} {:?}, stride={} bytes, payload={} bytes",
                frame.width(),
                frame.height(),
                frame.format(),
                frame.row_stride_bytes(),
                frame.pixels().len()
            )),
            None => report.push("video: no frame ready".to_owned()),
        }
    } else {
        report.push("video: unsupported".to_owned());
    }

    if capabilities.audio_output {
        match session
            .take_audio_buffer()
            .map_err(|error| error.to_string())?
        {
            Some(buffer) => report.push(format!(
                "audio: {} Hz, {} channel(s), {} sample frame(s)",
                buffer.sample_rate(),
                buffer.channels(),
                buffer.frame_count()
            )),
            None => report.push("audio: no buffer ready".to_owned()),
        }
    } else {
        report.push("audio: unsupported".to_owned());
    }

    Ok(report.join("; "))
}

fn save_state_to_path(
    host: &mut HostRuntime,
    active: Option<SessionId>,
    path: &str,
) -> Result<(), String> {
    let id = active.ok_or_else(|| "no active session".to_owned())?;
    let session = host.session_mut(id).map_err(|error| error.to_string())?;
    if !session.descriptor().capabilities.save_states {
        return Err("active core does not support emulator save states".to_owned());
    }
    let envelope = session
        .save_state_envelope()
        .map_err(|error| error.to_string())?;
    SaveStateStore::new(path)
        .save(&envelope)
        .map_err(|error| error.to_string())
}

fn load_state_from_path(
    host: &mut HostRuntime,
    active: Option<SessionId>,
    path: &str,
) -> Result<(), String> {
    let id = active.ok_or_else(|| "no active session".to_owned())?;
    let session = host.session(id).map_err(|error| error.to_string())?;
    if !session.descriptor().capabilities.save_states {
        return Err("active core does not support emulator save states".to_owned());
    }
    let envelope = SaveStateStore::new(path)
        .load()
        .map_err(|error| error.to_string())?;
    host.session_mut(id)
        .map_err(|error| error.to_string())?
        .load_state_envelope(&envelope)
        .map_err(|error| error.to_string())
}

fn apply_session_command(
    host: &mut HostRuntime,
    id: SessionId,
    command: &str,
) -> Result<String, String> {
    let session = host.session_mut(id).map_err(|error| error.to_string())?;
    match command {
        "pause" => {
            session.pause().map_err(|error| error.to_string())?;
            Ok("Paused.".to_owned())
        }
        "resume" => {
            session.resume().map_err(|error| error.to_string())?;
            Ok("Resumed.".to_owned())
        }
        "reset" => {
            session.reset().map_err(|error| error.to_string())?;
            Ok("Reset requested.".to_owned())
        }
        "stop" => {
            session.stop().map_err(|error| error.to_string())?;
            Ok("Stopped.".to_owned())
        }
        "step" => {
            let event = session
                .run_until_event()
                .map_err(|error| error.to_string())?;
            Ok(format!("Core event: {event:?}"))
        }
        _ => Err(format!("Unsupported session command: {command}")),
    }
}

fn print_usage() {
    println!("host-app — minimal terminal host");
    println!("Usage:");
    println!("  host-app --list-systems");
    println!("  host-app <content-path>");
    println!("  host-app --system <system-id> <content-path>");
    println!("  host-app [--system <system-id>] [--max-sessions <count>] <content-path>");
    println!("  host-app --help");
    println!("REPL library: import <path>, library, play <index>, forget <path>. Importing adds metadata; launch still performs format validation.");
    println!("Interactive input: input <up|down|left|right|action1|action2|action3|action4|start|select> <press|release>");
    println!("                  release-inputs releases all held controls.");
    println!("                  media polls one video frame/audio buffer and reports metadata.");
    println!(
        "                  run <events> runs a bounded number of core event calls (1..=10000)."
    );
}

fn validate_system_selection(
    registry: &CoreRegistry,
    selected: &str,
    probe_format: &str,
) -> Result<(), String> {
    if !registry.system_ids().any(|id| id.as_str() == selected) {
        return Err(format!("unknown system ID: {selected}"));
    }
    if selected != probe_format {
        return Err(format!(
            "system {selected} has no content probe registered by this frontend"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use host_app::library::ContentLibrary;

    struct NoSaveStateCore;

    impl emulation_core::EmulationCore for NoSaveStateCore {
        fn descriptor(&self) -> emulation_core::CoreDescriptor {
            emulation_core::CoreDescriptor {
                api_version: emulation_core::CURRENT_CORE_API_VERSION,
                system_id: emulation_core::SystemId::new("test-core").unwrap(),
                display_name: "Test core",
                core_version: "0.1.0",
                capabilities: emulation_core::CoreCapabilities::default(),
            }
        }

        fn load_content(&mut self, _: &[u8]) -> Result<(), emulation_core::CoreError> {
            Err(emulation_core::CoreError::Unsupported("content loading"))
        }

        fn reset(&mut self) -> Result<(), emulation_core::CoreError> {
            Ok(())
        }

        fn run_until_event(
            &mut self,
        ) -> Result<emulation_core::CoreEvent, emulation_core::CoreError> {
            Ok(emulation_core::CoreEvent::Yielded)
        }
    }

    fn registry() -> CoreRegistry {
        let mut registry = CoreRegistry::new();
        register_game_boy_color(&mut registry).unwrap();
        registry
    }

    fn args(values: &[&str]) -> Vec<std::ffi::OsString> {
        values
            .iter()
            .map(|value| std::ffi::OsString::from(*value))
            .collect()
    }

    #[test]
    fn launch_options_accept_session_limit_and_system_selection_in_either_order() {
        let first = parse_launch_options(&args(&[
            "--max-sessions",
            "8",
            "--system",
            "game-boy",
            "game.rom",
        ]))
        .unwrap();
        assert_eq!(
            first,
            LaunchOptions {
                path: PathBuf::from("game.rom"),
                selected_system: Some("game-boy".to_owned()),
                max_sessions: 8,
            }
        );

        let second = parse_launch_options(&args(&[
            "game.rom",
            "--system",
            "game-boy",
            "--max-sessions",
            "2",
        ]))
        .unwrap();
        assert_eq!(second.max_sessions, 2);
        assert_eq!(second.path, PathBuf::from("game.rom"));
    }

    #[test]
    fn launch_options_reject_invalid_or_ambiguous_arguments() {
        for values in [
            vec!["--max-sessions", "0", "game.rom"],
            vec!["--max-sessions", "many", "game.rom"],
            vec!["--max-sessions"],
            vec!["--max-sessions", "2", "--max-sessions", "3", "game.rom"],
            vec!["--system", "game-boy", "--system", "game-boy", "game.rom"],
            vec!["--system"],
            vec!["--unknown", "game.rom"],
            vec!["game-a.rom", "game-b.rom"],
            vec![],
        ] {
            assert!(
                parse_launch_options(&args(&values)).is_err(),
                "arguments should be rejected: {:?}",
                values
            );
        }
    }

    #[test]
    fn close_all_sessions_clears_active_selection_and_releases_every_session() {
        let mut host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();
        let first = host
            .create_session(Box::new(game_boy::GameBoyColorCore::new()))
            .unwrap();
        host.create_session(Box::new(game_boy::GameBoyColorCore::new()))
            .unwrap();
        let mut active = Some(first);

        assert_eq!(close_all_sessions(&mut host, &mut active), 2);
        assert_eq!(active, None);
        assert!(host.session_ids().is_empty());
        assert_eq!(close_all_sessions(&mut host, &mut active), 0);
    }
    #[test]
    fn library_entry_selection_is_one_based_and_out_of_range_is_rejected() {
        let directory =
            std::env::temp_dir().join(format!("host-app-library-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let first = directory.join("first.gb");
        let second = directory.join("second.gb");
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();

        let mut library = ContentLibrary::new();
        library.import(&second, 64).unwrap();
        library.import(&first, 64).unwrap();

        let first_path = library_entry_path(&library, 1).unwrap();
        let second_path = library_entry_path(&library, 2).unwrap();
        assert_eq!(first_path.file_name().unwrap(), "first.gb");
        assert_eq!(second_path.file_name().unwrap(), "second.gb");
        assert!(library_entry_path(&library, 0).is_none());
        assert!(library_entry_path(&library, 3).is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn explicit_system_selection_accepts_a_registered_matching_probe() {
        let registry = registry();
        assert!(validate_system_selection(&registry, "game-boy", "game-boy").is_ok());
    }

    #[test]
    fn explicit_system_selection_rejects_unknown_systems_and_probe_mismatches() {
        let registry = registry();
        assert!(validate_system_selection(&registry, "unknown-system", "unknown-system").is_err());
        assert!(validate_system_selection(&registry, "game-boy", "another-format").is_err());
    }

    #[test]
    fn input_control_parser_accepts_every_semantic_control_and_rejects_unknown_names() {
        let controls = [
            ("up", InputControl::Up),
            ("down", InputControl::Down),
            ("left", InputControl::Left),
            ("right", InputControl::Right),
            ("action1", InputControl::Action1),
            ("action2", InputControl::Action2),
            ("action3", InputControl::Action3),
            ("action4", InputControl::Action4),
            ("start", InputControl::Start),
            ("select", InputControl::Select),
        ];
        for (name, expected) in controls {
            assert_eq!(parse_input_control(name), Some(expected));
        }
        assert_eq!(parse_input_control("spacebar"), None);
    }

    #[test]
    fn input_inspection_reports_unsupported_capability_and_missing_session() {
        let mut host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();
        assert_eq!(
            inspect_inputs(&mut host, None).unwrap_err(),
            "no active session"
        );

        let id = host
            .create_session(Box::new(game_boy::GameBoyColorCore::new()))
            .unwrap();
        assert_eq!(
            inspect_inputs(&mut host, Some(id)).unwrap(),
            "input: unsupported by this core"
        );
    }

    #[test]
    fn media_inspection_reports_unsupported_outputs_without_polling_the_core() {
        let mut host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();
        let id = host
            .create_session(Box::new(game_boy::GameBoyColorCore::new()))
            .unwrap();

        assert_eq!(
            inspect_media(&mut host, Some(id)).unwrap(),
            "video: unsupported; audio: unsupported"
        );
    }

    #[test]
    fn event_budget_is_bounded_and_rejects_invalid_values() {
        assert_eq!(parse_event_budget("1").unwrap(), 1);
        assert_eq!(parse_event_budget("10000").unwrap(), 10_000);
        assert!(parse_event_budget("0").is_err());
        assert!(parse_event_budget("10001").is_err());
        assert!(parse_event_budget("many").is_err());
    }

    #[test]
    fn media_inspection_requires_an_active_session() {
        let mut host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();
        assert_eq!(
            inspect_media(&mut host, None).unwrap_err(),
            "no active session"
        );
    }

    #[test]
    fn save_state_commands_report_unsupported_capability_before_file_access() {
        let mut host = HostRuntime::new(HostRuntimeConfig::default()).unwrap();
        let id = host.create_session(Box::new(NoSaveStateCore)).unwrap();
        let path = std::env::temp_dir().join(format!(
            "host-app-unsupported-state-{}.bin",
            std::process::id()
        ));
        let path_text = path.to_string_lossy();

        let save_error = save_state_to_path(&mut host, Some(id), &path_text).unwrap_err();
        let load_error = load_state_from_path(&mut host, Some(id), &path_text).unwrap_err();

        assert!(save_error.contains("does not support emulator save states"));
        assert!(load_error.contains("does not support emulator save states"));
        assert!(!path.exists());
    }
}
