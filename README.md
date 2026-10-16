# cc3dsfs Revamped (Rust edition) 

A viewer for [Loopy's](https://3dscapture.com/) 3DS and DS capture boards, with a UI styled after the 3DS HOME Menu. It runs on Windows, and as a kiosk on a Raspberry Pi connected straight to a TV or an upscaler such as the Morph 4K.

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

## Top, bottom and both screens with the LCD grid

These were captured from a New 3DS XL, fullscreen on a 4K monitor, with the **LCD grid** filter at its default 30%. Each image has the whole 3840×2160 frame (scaled down) on the left, and a 1:1 close-up from the yellow box, zoomed 2×, on the right. The grid only shows at full size.

**Top screen** (Game Sack: Top screen preset, 9×). One grid cell per 3DS pixel.

![Top screen at 9x on 4K with the LCD grid](docs/view-top.jpg)

**Bottom screen** (Game Sack: Bottom screen preset, 9×).

![Bottom screen at 9x on 4K with the LCD grid](docs/view-bottom.jpg)

**Both screens** (Game Sack: Both screens preset, 4.5×). Each 3DS pixel is 4.5 screen pixels here, and the grid still follows it on both screens.

![Both screens stacked at 4.5x on 4K with the LCD grid](docs/view-both.jpg)

## Presets and screen filters

### Presets

**HOME Menu → Presets** sets up the screens in one click. Like Layout and Video, the page opens next to the live game.

| Preset | Screens | On a 4K display |
|---|---|---|
| Game Sack: Top screen | Top only, integer scaling, LCD grid | 9×, 3600×2160 |
| Game Sack: Bottom screen | Bottom only, integer scaling, LCD grid | 9×, 2880×2160 |
| Game Sack: Both screens | Top above bottom, filling the height, LCD grid | 4.5× |
| Game Sack: Hinge gap | Same, with a 28-pixel gap for the hinge | ≈4.25× |
| Morph 4K: Top / Bottom / Both / Hinge | The same four layouts, integer scaling, filter off | for a 1080p output |

The **Game Sack** presets recreate the 3DS views in Game Sack's [The Nintendo 3DS](https://www.youtube.com/watch?v=NdBrv1r_jZE). Use them with cc3dsfs fullscreen on a 4K display. Measured at 4K, the screens land where they are in that video:

- top screen at x 120–3719;
- bottom screen at x 480–3359;
- both screens with the top at x 1020–2819;
- hinge gap of about 120 px.

The **Morph 4K** presets are for sending 1080p to a Morph 4K, which then does the 4K scaling and the grid. They match the presets in [morph4k-profile-test](https://github.com/The1Azreen/morph4k-profile-test).

Presets change only screen settings: layout, gap, scaling, filter, crop and rotation. Audio, theme and connection settings stay as they are.

### Screen filters

**Video → Screen filter** draws a filter over each screen:

- **LCD grid**: a soft dark grid between pixels, the same in red, green and blue. The profile was measured from Game Sack's 4K footage of the 3DS. The default 30% strength matches it.
- **Fine grid**: thin, sharp lines between pixels, like the Morph 4K's PSP and Vita masks. Each line is a fifth of a 3DS pixel wide, rounded to whole screen pixels so every line is the same width.
- **Scanlines**: horizontal lines only.

Captured from a New 3DS XL (The Legend of Zelda: Ocarina of Time 3D) at the Game Sack: Top screen preset (9× on a 4K monitor) with the default 30% strength. Each tile is the same 48×32 3DS pixels, shown 1:1:

![No filter, LCD grid, fine grid and scanlines on a New 3DS XL capture at 9x on 4K](docs/screen-filters.png)

The filter follows each screen's own pixels. It stays aligned in every layout and screen position, and with crops, rotation, the bottom screen size slider and split windows. It also works at fractional scales such as the 4.5× both-screens view.

It needs each 3DS pixel to be at least 2 screen pixels. Between 2 and 3, where a soft grid would cause moiré, the LCD grid and scanlines are drawn as thin 1-pixel lines instead. A small bottom screen in a side-by-side layout gets this. Below 2 the filter turns itself off. In practice it needs a large window or fullscreen on a 1440p or 4K display.

### HDR output (Windows)

A screen filter darkens the picture. The Morph 4K and RetroTINK 4K make up for that with HDR injection: they send the game to the TV as HDR, so it can be brighter than normal (SDR) white. cc3dsfs can do the same on Windows.

Turn on **Video → HDR output**.

- While you're capturing with the menu closed, cc3dsfs shows the game in an HDR window, fullscreen on the same monitor.
- The live preview next to the Layout, Video, 3D and Presets pages is in HDR too, so you can tune the filter and brightness while you watch.
- **HDR brightness** sets the white level in nits (default 300). cc3dsfs then raises it further, by however much the screen filter darkens the picture.
- Highlights roll off softly below the monitor's peak brightness instead of clipping.
- **Esc** or a right click in the HDR window opens the HOME Menu. Closing the menu brings the HDR window back.

It needs Windows' **Use HDR** setting on for that monitor. The Video page shows whether it is, and the monitor's peak brightness.

Measured on a 4K HDR monitor, with the Top screen preset, the LCD grid at 30%, and the same frames in both runs:

| | Average | Brightest 1% | Peak |
|---|---|---|---|
| Normal (SDR) window | 43 nits | 126 nits | 171 nits |
| HDR output, 300 nits | 102 nits | 307 nits | 409 nits |

The grid looks the same in both.

Not available on the Raspberry Pi kiosk. Its display output is SDR only, so for HDR on the Pi, use the Morph 4K's own HDR injection.

### Black frame insertion

**Video → Black frame insertion** shows each game frame for one display refresh and black for the next ones. Motion looks sharper, but the picture flickers and gets darker, so it needs a 120 Hz or faster display.

- **Auto** picks the number of black frames from the display's refresh rate: 1 at 120–165 Hz, 2 at 180 Hz, 3 at 240 Hz. At 60 Hz it does nothing.
- **1–3 black frames** force a number. If the display is too slow for it, frames get dropped.
- The refresh rate is measured while the game is shown with the menu closed, and the Video page then shows it. If the output isn't synced to the display, BFI pauses rather than flicker unevenly.
- It turns off while the HOME Menu is open.
- With **HDR output**, the HDR window raises the brightness to make up for the black frames.

It needs the GPU renderer, so it isn't available when the Pi kiosk uses `linuxkms-software`.

### VSync (Windows)

**Video → VSync** (on by default) waits for the display's refresh before showing each frame. Turning it off shows frames as soon as they arrive, for less lag, but the picture can tear. In the HDR window it uses tearing presentation where the GPU and Windows support it. Black frame insertion keeps VSync on, since it needs every refresh. The Pi kiosk always syncs to the display.

### Two boards at once

To show two capture boards at the same time, open cc3dsfs twice. Each window connects to a board that isn't already in use, and its title shows which board it has. **Connection** lists every board found, so you can switch a window to another one.

To tie a window to one board, start it with `--device` and the board's serial (shown on the Connection page), for example in a shortcut:

```
cc3dsfs.exe --device 665F81B0
```

Both windows share the same settings file, so the window closed last keeps its settings.

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

## Raspberry Pi kiosk

On a Raspberry Pi 400 (or Pi 4, Pi 5, Pi 3), cc3dsfs can run as a kiosk: the Pi boots straight into it, fullscreen on HDMI, with no desktop. Plug the Pi's HDMI into a Morph 4K (or a TV) and the capture board into a USB port.

> The kiosk setup is new and has not been tested on a Pi yet. Please report what works.

### What you need

- Raspberry Pi OS **Lite (64-bit)**. Bookworm or newer.
- SSH or a keyboard for the first setup. The Pi 400's built-in keyboard works in the kiosk too.

### Install

Either download `cc3dsfs_raspberry_pi_arm64` from the GitHub Actions build (it includes a ready-made binary), extract it, and run:

```
sudo ./cc3dsfs-pi/pi/install.sh
```

or build on the Pi itself from a clone of this repo. This installs Rust and takes a while on a Pi:

```
git clone https://github.com/The1Azreen/cc3dsfs-but-with-a-UI.git
cd cc3dsfs-but-with-a-UI
sudo ./pi/install.sh
```

Then `sudo reboot`. The installer:

- adds a `cc3dsfs` system user;
- sets up USB permissions for the capture boards;
- installs a systemd service that starts the kiosk on the first console;
- lets the kiosk shut down and restart the Pi.

### Display and the Morph 4K

The kiosk outputs 1920×1080 at 60 Hz by default. To change it, edit `/etc/cc3dsfs/kiosk.conf`, then run `sudo systemctl restart cc3dsfs`.

- `MODE`: the HDMI mode, e.g. `1280x720@60`. `sudo cc3dsfs-kiosk --list-modes` shows what the display offers.
- `OUTPUT`: which HDMI port to use. On a Pi 400, `HDMI-A-1` is the port next to the power socket.
- `BACKEND`: set to `linuxkms-software` if the GPU renderer doesn't work.
- `ROTATION`: rotates the whole picture (0, 90, 180 or 270).

If the upscaler or TV is switched on after the Pi, the kiosk retries until a display appears. If the Pi still doesn't detect it, reinstall with `sudo ./pi/install.sh --force-hdmi`. This keeps HDMI on at the configured mode even when no display is detected.

For sound over HDMI, pick the HDMI output (`vc4hdmi`) in **Audio → Output device**.

### Using it

- **Esc** (or a right click) opens the HOME Menu.
- **HOME Menu → Power** shuts down or restarts the Pi. Shut down before unplugging it.
- **Power → Exit to console** gives you the login prompt back until the next reboot.
- Over SSH:
  - logs: `journalctl -u cc3dsfs -f`
  - stop the kiosk: `sudo systemctl stop cc3dsfs`
  - remove it: `sudo ./pi/install.sh --uninstall`

### Boards on the Pi

New 3DS XL and older 3DS/DS boards work out of the box. They use the same direct USB code as WinUSB on Windows.

2024 DS boards need FTDI's D2XX library for Linux (ARMv8). Install `libftd2xx.so` from [FTDI's D2XX page](https://ftdichip.com/drivers/d2xx-drivers/) into `/usr/local/lib`. The udev rule already detaches the kernel's serial driver from those boards.

## Project layout

- `src/capture/`: device discovery and capture.
  - `ftd3.rs`: New 3DS XL boards.
  - `legacy_usb.rs`: older 3DS and DS boards.
  - `ftd2.rs`: 2024 DS boards.
  - `frame.rs`: frame decoding.
- `src/audio.rs`: resampling and low-latency output.
- `src/view.rs`: crop table and rotation.
- `src/presets.rs`: the Game Sack and Morph 4K presets.
- `src/layout.rs`: screen placement maths, shared with the HDR window.
- `src/hdr.rs`, `src/hdr.hlsl`: the Windows HDR output window (Direct3D 11, scRGB).
- `src/bfi.rs`: black frame insertion timing, shared by the Slint and HDR windows.
- `assets/`: the app icon.
- `ui/`: the Slint UI. `theme.slint` holds the 3DS-style components; `filter.slint` draws the screen filters; `app.slint` holds the windows and pages.
- `pi/`: Raspberry Pi kiosk installer, launcher, systemd service and udev rules.

## Not ported (yet)

Compared to the original cc3dsfs, this version doesn't have:

- macOS and Android builds. Linux is supported through the Raspberry Pi kiosk, but there is no desktop Linux package yet.
- Non-Loopy devices: IS Nitro/TWL, Optimize (with its serial keys and v2 downgrade), Partner CTR, Nisetro, and the USB conflict setting they need.
- WinUSB support for the 2024 DS boards.
- Video: colour correction, pixel aspect ratio (PAR), the fixed output framerate, the Async toggle, slow poll, extra padding for rounded corners, and window scaling in 0.5× steps. The original's shaders are replaced by the screen filters above.
- Menu scaling and the restart-audio option.
- Most keyboard shortcuts (only Esc, F11, Alt+Enter and M are here), the F1–F8 layout keys, gamepad navigation, and holding a button to reset settings.
- The `CC3DSFS_CFG_DIR` setting and `--help`.

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
