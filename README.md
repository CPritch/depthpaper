# shiftpaper

Parallax wallpaper daemon for Wayland. Uses monocular depth estimation to generate a depth map from any image, then shifts the wallpaper layers based on cursor position. The effect is subtle but satisfying.

Experimental. Built and tested on a single machine (Arch, Hyprland, RTX 5060). Expect rough edges.

![Example Tiger Wallpaper](header.gif)

## How it works

Two binaries:

- `shiftpaper` (CLI) takes a source image, runs Depth Anything V2/V3 inference via ONNX Runtime, and writes a color + 16-bit depth PNG pair to a cache directory.
- `shiftpaperd` (daemon) loads the pre-baked pair and renders a parallax-displaced wallpaper on wlr-layer-shell surfaces using wgpu/Vulkan.

The daemon has no ML dependencies. All inference happens in the CLI.

## Dependencies

- Wayland compositor with wlr-layer-shell support (see [Supported compositors](#supported-compositors))
- Vulkan-capable GPU
- ONNX Runtime (`onnxruntime` on Arch; pick a CUDA or ROCm variant if you want GPU inference)

Building from source additionally requires a Rust toolchain.

The depth model is downloaded automatically by `shiftpaper fetch-model`.

## Install

### Arch Linux (AUR)

```
paru -S shiftpaper        # or yay, pikaur, etc.
```

### From source

```
cargo install --path crates/cli
cargo install --path crates/daemon
```

If you want to use the systemd user service with a source build, install to somewhere like `/usr/local` so the binaries are in systemd's lookup path:

```
cargo install --root /usr/local --path crates/cli
cargo install --root /usr/local --path crates/daemon
```

## Quick start

```
shiftpaper fetch-model
shiftpaper set ~/Pictures/wallpaper.jpg
```

`fetch-model` downloads Depth Anything V2 Small (~97 MB) from HuggingFace to `~/.local/share/shiftpaper/models/` and saves the path to config. After that, `set` needs no `--model` flag.

If you already have a model, skip fetch-model and pass it directly:

```
shiftpaper set ~/Pictures/wallpaper.jpg --model ~/path/to/model.onnx
```

The model path is saved to config so `--model` is only needed once.

## Running the daemon

Directly:

```
shiftpaperd
```

As a systemd user service. The AUR package ships the unit file at `/usr/lib/systemd/user/`, so:

```
systemctl --user enable --now shiftpaperd
```

For source installs, copy the unit file first:

```
cp shiftpaperd.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now shiftpaperd
```

Change wallpapers without restarting:

```
shiftpaper set ~/Pictures/new_wallpaper.jpg
systemctl --user reload shiftpaperd
```

## CLI referenceland

| Command | Description |
|---------|-------------|
| `shiftpaper fetch-model` | Download the default depth model from HuggingFace |
| `shiftpaper set <image>` | Bake an image and set it as the active wallpaper |
| `shiftpaper bake <image>` | Bake an image without changing the active wallpaper |
| `shiftpaper mode [pointer\|hyprland]` | Show or set the cursor tracking mode |

## Config

Lives at `~/.config/shiftpaper/config.toml`. Mostly managed by the CLI but you can edit it by hand.

```toml
[daemon]
parallax_intensity = 0.025
tracking_mode = "pointer"       # "pointer" (Wayland-native) or "hyprland" (IPC)
idle_timeout_secs = 300
battery_threshold = 20          # percent; 0 to disable

[inference]
model_path = "~/.local/share/shiftpaper/models/depth_anything_v2_small.onnx"

[wallpaper]
color = "~/.cache/shiftpaper/wallpapers/abc123.color.png"
# depth is inferred from color path; override with:
# depth = "/path/to/custom.depth16.png"
```

### Tracking modes

**pointer** (default): Uses Wayland pointer events. Works on any wlr-layer-shell compositor. Parallax only active when the cursor is over visible desktop. Stops rendering entirely when the cursor is over a window, which is good for battery.

**hyprland**: Polls the Hyprland IPC socket for global cursor position. Parallax stays active even when windows cover the desktop. Hyprland-only. Note that this lets the daemon observe cursor position over arbitrary windows.

## Supported compositors

Requires [wlr-layer-shell](https://wayland.app/protocols/wlr-layer-shell-unstable-v1). Tested compositors:

- Hyprland
- Sway
- River
- Wayfire
- niri
- labwc

GNOME and KDE Plasma do not implement wlr-layer-shell and are not supported.

## License

MIT