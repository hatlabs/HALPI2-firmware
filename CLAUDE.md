# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

HALPI2 is a Raspberry Pi Compute Module 5 based boat computer with an RP2040 microcontroller handling power management and peripheral control. The firmware is written in Rust using the Embassy async framework.

## Development Commands

Use the `./run` script for all development tasks. Commands are organized by functional area:

### Core Development
- `./run build [--release]` - Build firmware (debug by default)
- `./run build:bootloader [--release]` - Build bootloader
- `./run clean` - Clean all build artifacts
- `./run check` - Run cargo check and clippy

### Hardware Interaction
- `./run flash [firmware|bootloader|all]` - Flash to device (default: firmware)
- `./run monitor` - Attach debugger/monitor
- `./run flash:monitor` - Flash then monitor (common workflow)

### Release/Artifacts
- `./run release:build` - Build all release artifacts (elf, bin, uf2)
- `./run release:artifacts` - Convert existing ELF to bin/uf2 formats
- `./run release:version` - Get current firmware version

### Package Management
- `./run package:deb` - Build Debian package (native)
- `./run package:deb:docker` - Build Debian package using Docker
- `./run package:docker:build` - Build Docker tools image

### Testing/CI
- `./run test:prepare` - Copy artifacts to test directory
- `./run ci:build` - Full CI build pipeline
- `./run ci:check` - CI verification checks

### Development Utilities
- `./run dev:env` - Show/check development environment
- `./run dev:clean:all` - Deep clean (cargo + artifacts + packages)
- `./run dev:version:bump <version>` - Bump to specific version (e.g. 3.2.0)
- `./run dev:version:dry-run <version>` - Preview version change without applying
- `./run dev:version:show` - Show current version

### Common Workflows
```bash
# Development cycle
./run build && ./run flash:monitor

# Create release
./run release:build

# Full CI pipeline
./run ci:build

# Version management
./run dev:version:show                    # Check current version
./run dev:version:dry-run 3.2.0           # Preview version change
./run dev:version:bump 3.2.0              # Bump to new version
```

## Architecture Overview

### Core Structure
- **Workspace**: Contains `firmware/` (main application) and `bootloader/` packages
- **Embassy Framework**: Async runtime with tasks for different subsystems
- **State Machine**: Hierarchical power management using `statig` crate
- **I2C Communication**: Secondary device (0x6d) for CM5 communication

### Key Components

#### State Machine (`firmware/src/tasks/state_machine.rs`)
Manages power states with transitions based on:
- VIN power availability (>9.0V threshold)
- Supercap voltage levels (8.0V power-on, 5.5V power-off)
- Compute Module status (3.3V rail monitoring)
- Watchdog timeouts and user interactions

States include: PowerOff, OffCharging, SystemStartup, OperationalSolo/CoOp, BlackoutSolo/CoOp, EnteringStandby, Standby, HostUnresponsive, PoweredDownBlackout/Manual.

#### Task System
- **Config Manager**: Persistent configuration storage in flash
- **I2C Secondary**: Handles CM5 communication (command/response)
- **GPIO Input**: Monitors analog inputs (VIN, VSCAP, IIN) and digital inputs
- **LED Blinker**: Controls 5-LED RGB bar indicating system state and voltage
- **Power Button**: Handles physical button presses
- **Watchdog Feeder**: System watchdog management
- **Flash Writer**: Firmware update handling

#### I2C Command Interface
Commands include power control (0x10), watchdog (0x12), voltage thresholds (0x13-0x14), analog readings (0x20-0x23), shutdown commands (0x30-0x31), and DFU operations (0x40-0x45).

### Hardware Details
- **Target**: RP2040 microcontroller (thumbv6m-none-eabi)
- **GPIO**: 30 pins controlling power rails, LEDs, USB ports, I2C buses
- **Analog**: VIN voltage, supercap voltage, input current monitoring
- **RGB LEDs**: 5-LED bar graph showing voltage levels and system state

### Configuration
- Flash-based persistent storage using `sequential-storage`
- Voltage correction scales, thresholds, timeouts configurable
- LED brightness, auto-restart behavior configurable

## File Structure Notes

### Main Modules
- `config.rs` - Hardware constants and default values
- `config_resources.rs` - Resource assignment using `assign-resources`
- `flash_layout.rs` - Flash memory partitioning
- `led_patterns.rs` - LED behavior patterns for different states
- `tasks/mod.rs` - Task spawning and coordination

### Build System
- Uses Embassy's async task system
- Target-specific memory layout in `memory.x`
- Custom build script for resource generation
- Debian packaging with `debian/` directory

## Development Notes

- No traditional tests - this is embedded firmware for specific hardware
- State machine is the core architectural component
- All I/O is async using Embassy
- Flash storage uses sequential-storage for wear leveling
- Bootloader supports DFU updates over I2C
- GPIO inputs are debounced and filtered
- LED patterns provide visual system status feedback

## Version Management

**IMPORTANT**: Never manually edit version numbers in Cargo.toml files. Always use the `./run dev:version:bump` command to update versions.

The version bump script updates:
- `firmware/Cargo.toml` - Firmware package version
- `debian/changelog` - Debian package changelog with timestamp
- Git tags (when creating releases)

Example:
```bash
./run dev:version:show                 # Check current version
./run dev:version:dry-run 3.2.1-a1     # Preview changes
./run dev:version:bump 3.2.1-a1        # Apply version bump
```