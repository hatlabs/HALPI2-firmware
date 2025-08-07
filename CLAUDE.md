# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

HALPI2 is a Raspberry Pi Compute Module 5 based boat computer with an RP2040 microcontroller handling power management and peripheral control. The firmware is written in Rust using the Embassy async framework.

## Development Commands

Use the `./run` script for all development tasks:

### Building
- `./run build` - Build the project (debug)
- `./run build --release` - Build release version
- `./run build-binary` - Build release and create `.bin` file
- `./run build-uf2` - Build release and create `.uf2` file
- `./run build-bootloader` - Build the bootloader
- `./run build-all` - Build everything including Debian package

### Flashing and Debugging
- `./run upload` or `./run flash` - Flash firmware to RP2040
- `./run flash-bootloader` - Flash bootloader only
- `./run flash-all` - Flash both bootloader and firmware
- `./run monitor` or `./run attach` - Attach debugger/monitor
- `./run flash-and-monitor` - Flash then monitor

### Packaging
- `./run build-debian` - Build Debian package
- `./run debtools-build` - Build Debian package using Docker
- `./run convert-artifacts` - Convert ELF files to UF2/BIN formats

### Utilities
- `./run clean` - Clean build artifacts
- `./run get-version` - Get current firmware version

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