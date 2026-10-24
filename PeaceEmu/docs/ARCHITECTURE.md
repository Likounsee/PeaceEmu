# Architecture

## Current state

The repository contains a Rust workspace with a shared core contract and a native eframe/egui desktop application. Console-specific implementations are developed and versioned in separate module repositories. The desktop provides library management, session controls, semantic keyboard input, preferences and a bounded video-frame presentation path. Audio playback is not yet integrated, and the Game Boy core is incomplete and not a playable emulator. Contracts and successful CI checks are not proof of console compatibility.

## Intended layers

1. **Application / frontend** — library browsing, launch orchestration, user settings, diagnostics, and host-side interface themes. It must not own console-specific hardware logic.
2. **Shared services** — configuration, content metadata, input mapping, save storage, audio/video host integration, presentation settings and logging, introduced when implemented functionality needs them.
3. **Core contract** — stable metadata, capability declarations, lifecycle errors and observable execution events. Graphics capabilities must be declared rather than inferred.
4. **Platform cores** — independent CPU, memory/bus, interrupt/timer, peripheral, graphics and audio implementations as required by each target.
5. **Presentation pipeline** — host-side scaling, aspect-ratio handling, filtering, color correction and optional overlays or redesigned menus. It must not be confused with the emulated GPU/video hardware or modify core timing.
6. **Native adapters** — optional C ABI boundaries for licensed C/C++ cores, with explicit ownership, buffer lifetime and error mapping.

## Dependency direction

The application may depend on shared services and core contracts. Concrete cores may depend on the minimal contract but must not depend on a graphical frontend. Avoid cycles between services and cores.

## Core contract decisions

- Each core exposes a stable lowercase system ID and an implementation version.
- Capabilities are explicit booleans and default to false; the host must not infer support from a system name.
- Execution returns an observable event rather than forcing every platform into a frame-per-call abstraction.
- Save-state methods default to an explicit unsupported error.
- External bytes are validated by the concrete core before machine state is changed.
- Platform-specific extensions remain possible when the common interface is insufficient.

## Console software interfaces and enhanced presentation

A console with a system interface may expose a dashboard, home menu, settings UI, media shell or other built-in software. This must be evaluated per system: some consoles have no general-purpose system UI, and faithful behavior may depend on original firmware or system software.

- Keep three modes conceptually distinct: execution of original system software when legally supplied by the user, independent faithful reimplementation of documented behavior, and optional modernized interface themes.
- Never imply a recreated interface is the original firmware. Track the source and confidence of behavioral details.
- Keep presentation enhancements in the host renderer when possible. A system-specific shader or reconstruction belongs behind an explicit capability and setting, with a faithful baseline available.
- Enhancement options may include integer scaling, aspect-ratio correction, filtering, color correction, and higher-resolution redrawn menus. 3D upscaling, texture replacement, widescreen patches or geometry fixes are system/game-specific and must not be promised universally.
- Test authentic and enhanced modes separately. Enhancements must be reversible and must not be required for correctness.
- Do not bundle copyrighted firmware, proprietary UI assets or console software without permission. Support user-provided lawful files where feasible, and independently recreate artwork/behavior when appropriate.

## Next architectural gates

Before selecting a first console, inspect reliable public specifications and legally usable test programs. Choose based on manageable scope and testability. Add real bus/CPU tests before creating a UI that could imply the system is already emulated.

## Console implementations are modules

Every console implementation must live in its own independently testable repository and expose an adapter for the shared core contract, not code embedded directly in the frontend. The host discovers modules through stable system IDs and registers factories in `CoreRegistry`; a selected module instance then implements `EmulationCore`. Console-specific CPU, bus, mapper, peripherals and hardware timing stay inside that module. Shared services and presentation remain outside it and communicate through explicit contracts.

The host repository defines generic module contracts and integration boundaries only. A console-specific repository owns its CPU, bus, peripherals, timing, test fixtures, and module-specific development. No console is considered supported merely because its repository exists or its module can be compiled.

Future consoles should use the same integration shape without forcing their hardware into Game Boy-specific abstractions. Optional console-specific system interfaces and graphics enhancements should be exposed through host presentation contracts or explicit module capabilities, not hardwired into the shared core.


## CPU execution and graphics translation strategies

PeaceEmu uses one application, one session lifecycle and one host/core integration contract. CPU execution strategy is selected inside the console-module boundary; it must not create a separate frontend or a second host runtime.

### CPU strategy is explicit, not inferred from a console label

The shared planning contract lives in `emulation_core::execution_strategy`. It describes CPU architectures and can prefer one of three strategies:

- **Native x86-64 with compatibility layer**: permitted only when both guest and host are x86-64 and host policy explicitly allows it. Native instructions are only one part of the work; a console module still needs ABI/calling-convention handling, virtual-memory mapping, executable/module loading, syscall/kernel and system-service compatibility, privilege boundaries and exception handling. A matching ISA does not mean an unmodified game can run.
- **Interpreter**: portable reference path for guest instruction semantics. It is the baseline for architectures such as ARM, PowerPC, MIPS or a proprietary CPU and is valuable for debugging and differential tests.
- **Dynamic recompiler (JIT)**: optional optimization that translates guest instructions to host code. It is selected only as a preference; a module must actually implement it, validate generated-code boundaries and retain a safe interpreter fallback where feasible.

The planner returns a *preferred strategy*, not proof of support. Modules must declare/validate their implemented strategies and reject unsupported choices rather than silently pretending to run. The current core capability structure does not yet advertise a CPU strategy; wiring strategy support into a versioned module/core descriptor is a follow-up contract task, because adding required fields to public descriptors is a compatibility-sensitive change.

### Keep CPU, system compatibility and GPU translation independent

Each future console module should separate these concerns behind internal interfaces:

1. **CPU execution** — native-compatible execution, interpreter or JIT; CPU state and instruction semantics.
2. **System compatibility** — executable format, ABI, system calls, kernel-facing behavior, libraries/services, virtual address space and synchronization.
3. **Machine model** — bus/memory map, interrupts, exceptions, timers, DMA and required peripherals. Cycle accuracy is required only when tests and software behavior demonstrate it is necessary.
4. **Graphics translation** — guest command processor, resource/state model and shader translation, independently mapped to a host backend such as Vulkan, Direct3D or Metal. Do not equate guest GPU commands with host API calls or claim universal shader compatibility.
5. **Host I/O adapters** — shared input, audio, video, storage, save-state and diagnostics contracts.

These interfaces belong to independently testable console crates and reusable shared services where semantics genuinely match. Console-specific behavior must not leak into egui or the application controller. Existing `EmulationCore`, `CoreRegistry`, `ApplicationController`, `CoreEvent`, input/media payloads and session lifecycle remain the integration seam; they are useful foundations, but do not yet constitute a complete native-x64 compatibility environment or a complete JIT/GPU translation framework.

### Host compatibility and honest support claims

A module must consider host ISA, operating-system APIs, memory-protection/JIT permissions, graphics backend, required firmware, legal user-provided assets and hardware features. Strategy planning cannot bypass any of those checks. Capability declarations must represent tested behavior, not desired future work. Test interpreter correctness first; compare JIT output against it; test native compatibility separately from system services and graphics translation. A console is not supported until reproducible software tests and end-to-end results justify the claim.
