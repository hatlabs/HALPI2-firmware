---
title: CI-generated UF2 files brick devices due to wrong load address
date: 2026-04-23
category: build-errors
module: CI Build System
problem_type: build_error
component: tooling
severity: critical
symptoms:
  - Devices flashed via test jig with CI-generated UF2 become unresponsive
  - Firmware placed at 0x10000000 instead of correct address 0x10007000
  - Firmware overwrites BOOT2 and bootloader, requiring manual recovery
  - DFU updates via I2C work correctly (bootloader handles address placement)
  - Locally-built UF2 files work fine (local build uses ELF source)
root_cause: config_error
resolution_type: config_change
tags:
  - picotool
  - uf2-conversion
  - ci-pipeline
  - load-address
  - firmware-flashing
  - rp2040
---

# CI-generated UF2 files brick devices due to wrong load address

## Problem

The CI build action converted firmware from flat `.bin` to `.uf2` using `picotool uf2 convert`. Flat binaries have no address metadata, so picotool defaulted to base address `0x10000000`. The firmware actually loads at `0x10007000`, causing the UF2 to place firmware code over BOOT2 and the bootloader, bricking the device.

## Symptoms

- Devices flashed via test jig with CI-built v3.3.0 UF2 became completely unresponsive
- Required manual downgrade to recover (flash known-good firmware via SWD/BOOTSEL)
- DFU updates of the same firmware version worked fine
- Locally-built UF2 files worked fine
- The bug existed since the CI build action was first created but went unnoticed because firmware was deployed via DFU (I2C), not UF2 flash

## What Didn't Work

The original CI configuration used `.bin` files as picotool input:

```bash
picotool uf2 convert \
  target/thumbv6m-none-eabi/release/halpi2-rs-firmware.bin \
  target/thumbv6m-none-eabi/release/halpi2-rs-firmware.uf2 \
  --family rp2040
```

`arm-none-eabi-objcopy -O binary` strips all address metadata when producing `.bin` files. picotool then defaults to `0x10000000` (per its help: "Load offset (memory address; default 0x10000000 for BIN file)").

## Solution

Changed CI to convert from ELF (which carries section addresses in program headers) and added `-t elf` flag because the build outputs have no file extension:

```bash
picotool uf2 convert -t elf \
  target/thumbv6m-none-eabi/release/halpi2-rs-firmware \
  target/thumbv6m-none-eabi/release/halpi2-rs-firmware.uf2 \
  --family rp2040
```

The `-t elf` flag is required because picotool determines format from file extension. Without a recognized extension (`.elf`, `.bin`, `.uf2`), it fails with "does not have a recognized file type (extension)".

The local build script (`run:149`) already did this correctly using `*.elf` glob patterns:
```bash
picotool uf2 convert "$elf" "artifacts/${base}.uf2"
```

## Why This Works

ELF binaries contain program headers with load addresses for each section. The firmware ELF has:

| Section | Load Address |
|---------|-------------|
| `.vector_table` | `0x10007000` |
| `.text` | `0x100070c0` |
| `.rodata` | `0x1001e124` |

picotool reads these headers and generates UF2 blocks targeting the correct addresses. Verified by parsing UF2 output blocks:

```
Block 0: addr=0x10007000
Block 1: addr=0x10007100
Block 2: addr=0x10007200
```

The RP2040 flash layout separates bootloader (`0x10000100`) from firmware (`0x10007000`). The wrong base address caused firmware to overwrite both BOOT2 (`0x10000000`) and the bootloader.

## Prevention

1. **Always convert from ELF, never from .bin** when address information matters. ELF preserves section metadata; flat binaries do not.
2. **Use `-t elf` when ELF files lack a `.elf` extension.** picotool requires either a recognized extension or explicit `-t` flag.
3. **Keep CI and local build methods aligned.** This bug was a divergence between CI (`.bin` source) and local (`.elf` source) conversion paths.

## Related Issues

- PR [hatlabs/HALPI2-firmware#39](https://github.com/hatlabs/HALPI2-firmware/pull/39): The fix
- The firmware `.bin` files are still generated and included in the deb package for the `postinst` script, which uses them for DFU flashing via `halpi flash`. This is correct — the bootloader handles address placement for DFU.
