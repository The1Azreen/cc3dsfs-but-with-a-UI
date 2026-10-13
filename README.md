# cc3dsfs (Rust edition)

A Windows viewer for [Loopy's](https://3dscapture.com/) 3DS and DS capture boards, with a UI styled after the 3DS HOME Menu.

This is a Rust rewrite of [cc3dsfs](https://github.com/Lorenzooone/cc3dsfs) by Lorenzooone. The capture code is a port of the original's Loopy device code. The interface is built with [Slint](https://slint.dev).

| Capturing a New 3DS XL (integer scaling) | Capturing a DS (older board) |
|---|---|
| ![Both 3DS screens captured at 3x integer scale](docs/capture.png) | ![Both DS screens captured from an older Loopy DS board](docs/ds-capture.png) |

| HOME Menu (Esc or right click) |
|---|
| ![The 3DS-styled HOME Menu shown while capturing](docs/home-menu.png) |

## Supported boards

| Board | Driver on Windows |
|---|---|
| New 3DS XL (2020+, FT601) | FTDI D3XX (default) or WinUSB |
| 3DS (older boards) | WinUSB |
| DS (older boards) | WinUSB |
| DS (2024 boards, FT232H) | FTDI D2XX (default) |

- The FTDI DLLs (`FTD3XX.dll`, `ftd2xx.dll`) are copied next to the executable when you build.
- You still need the board's driver installed. If an older board isn't detected, use [Zadig](https://zadig.akeo.ie/) to install the WinUSB driver for it.
- 3D capture needs a New 3DS XL board with 3D support, on a USB 3 port.

## Features

- Connects automatically and reconnects after you unplug the board.
- Live preview: while capturing, the Layout, Video and 3D pages open as a side panel next to the game, so you see each change as you make it.
- Layouts:
  - Top above bottom, bottom above top, side by side (either order), top only or bottom only.
  - Adjustable bottom screen size, screen offset and gap between screens.
- Split mode: the bottom screen opens in its own window, which you can put on a second monitor.
- Fullscreen, integer scaling and optional smoothing.
- The built-in crops from the original (DS, GBA, Virtual Console), plus optional game-specific crops.
- Screen rotation for each screen.
- Audio with volume, mute and output device selection, tuned for low latency.
- 3D capture, shown as the left eye only or side by side.
- 4 profile slots; settings are saved automatically.
- Seven console-colour themes.

## Layout and Video

While capturing, the Layout and Video pages open as a panel next to the game. The game stays live, so each change shows on screen as you make it.

**Layout** sets how the screens are arranged, the bottom screen's size and position, the gap between the screens, split windows and fullscreen.

![Layout page beside the live capture](docs/layout-live.png)

**Video** sets the crop, game-specific crops, integer scaling, smoothing and rotation for each screen.

![Video page beside the live capture](docs/video-live.png)

The 3D page opens the same way. Narrow windows put the panel along the bottom instead.

## Controls

| Input | Action |
|---|---|
| Esc / right click / HOME button | Open or close the HOME Menu |
| F11 / Alt+Enter / double click | Toggle fullscreen |
| M | Mute |

Settings and profiles are stored in `%APPDATA%\cc3dsfs`.

## Building

1. Install [Rust](https://rustup.rs/) (MSVC toolchain).
2. Build:

```
cargo build --release
```

The program, `FTD3XX.dll` and `ftd2xx.dll` end up in `target\release`. Keep the DLLs next to `cc3dsfs.exe` when you move it.

Tests: `cargo test`. Debug builds also have a fake board for trying out layouts without hardware: set `CC3DSFS_DEMO=1` before running.

## Project layout

- `src/capture/`: device discovery and capture.
  - `ftd3.rs`: New 3DS XL boards.
  - `legacy_usb.rs`: older 3DS and DS boards.
  - `ftd2.rs`: 2024 DS boards.
  - `frame.rs`: frame decoding.
- `src/audio.rs`: resampling and low-latency output.
- `src/view.rs`: crop table and rotation.
- `ui/`: the Slint UI. `theme.slint` holds the 3DS-style components; `app.slint` holds the windows and pages.

## Not ported (yet)

Compared to the original cc3dsfs, this version doesn't have:

- Linux, macOS and Android builds.
- Non-Loopy devices: IS Nitro/TWL, Optimize, Partner CTR, Nisetro.
- WinUSB support for the 2024 DS boards.
- Shaders, colour correction, BFI, PAR and the output framerate settings.

## Acknowledgements

This project is based on [cc3dsfs](https://github.com/Lorenzooone/cc3dsfs), created by **[Lorenzooone](https://github.com/Lorenzooone)**. It is a derivative work of cc3dsfs, which is licensed under the MIT License.

- The capture code is a port of cc3dsfs's Loopy device code. That covers the USB protocols for the New 3DS XL, older 3DS/DS and 2024 DS boards, plus frame decoding and audio handling.
- The crop presets and the screen layout behaviour come from cc3dsfs.
- Lorenzooone's original copyright notice and the MIT permission notice are kept, unmodified, in [LICENSE](LICENSE), as the MIT License requires.

Thanks also to **Loopy** ([3dscapture.com](https://3dscapture.com/)), whose capture boards this supports. The board communication code is based on Loopy's sample code, and Loopy provided the DS FPGA bitstreams in `firmware/`.

## Licenses

- MIT, see [LICENSE](LICENSE). The original cc3dsfs is Copyright (c) 2024 Lorenzooone.
- The DS FPGA bitstreams in `firmware/` were provided by Loopy; see [other licenses/bin_files.txt](other%20licenses/bin_files.txt).
- The FTDI driver libraries are covered by [FTDI's licence](ftdi_lib_files/ftd2xx_ftd3xx_license.txt).
- The UI uses Slint under its Royalty-free license.
