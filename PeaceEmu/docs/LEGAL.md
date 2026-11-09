# Legal and third-party components

PeaceEmu does not bundle commercial ROMs, BIOS images or firmware. Users are responsible for supplying content they are legally entitled to use where required.

## Project license

The repository includes `LICENSE-MIT` and `LICENSE-APACHE`. The workspace metadata declares `MIT OR Apache-2.0`; contributors should ensure new project files are compatible with that dual-license policy. Third-party code may have different terms and must retain its required notices.

## Third-party inventory

Before adding a dependency or external core, record its upstream source, exact version or revision, license, redistribution obligations, platform support, maintenance status and known security concerns. Confirm that static linking, dynamic linking, packaging and source distribution comply with the applicable license. Preserve required notices and source offers.

The Rust workspace uses third-party Rust dependencies for the desktop UI, media handling, serialization, cryptographic verification and networking; their exact versions are recorded in Cargo manifests and the lockfile where applicable. Review licenses, maintenance, security advisories and redistribution obligations before adding or updating dependencies. No C/C++ core is currently integrated. GitHub Actions dependencies are build tooling and are not linked into the distributed application.

Do not implement circumvention of console protections or redistribute copyrighted firmware without authorization. License presence does not grant rights to game content, BIOS images, trademarks or console firmware.
