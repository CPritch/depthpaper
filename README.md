# depthpaper

Parallax wallpaper daemon for Wayland. Uses monocular depth estimation to generate a depth map from any image, then shifts the wallpaper layers based on cursor position. The effect is subtle but satisfying.

Experimental. Built and tested on a single machine (Arch, Hyprland, RTX 5060). Expect rough edges.

## How it works

Two binaries:

- `depthpaper` (CLI) takes a source image, runs Depth Anything V2/V3 inference via ONNX Runtime, and writes a color + 16-bit depth PNG pair to a cache directory.
- `depthpaperd` (daemon) loads the pre-baked pair and renders a parallax-displaced wallpaper on wlr-layer-shell surfaces using wgpu/Vulkan.

The daemon has no ML dependencies. All inference happens in the CLI.

## Dependencies

- Wayland compositor with wlr-layer-shell support
- Vulkan-capable GPU
- A Depth Anything ONNX model (v2-small recommended)
- Rust toolchain

## Install

```
cargo install --path crates/cli
cargo install --path crates/daemon
```

## Quick start

```
depthpaper set ~/Pictures/wallpaper.jpg --model ~/path/to/depth_anything_v2_small.onnx
```

This bakes the image (first run only), writes the config, and tells you to reload the daemon. The model path is saved to config so you only need `--model` once.

## Running the daemon

Directly:

```
depthpaperd
```

As a systemd user service:

```
cp depthpaperd.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now depthpaperd
```

Change wallpapers without restarting:

```
depthpaper set ~/Pictures/new_wallpaper.jpg
systemctl --user reload depthpaperd
```

## Config

Lives at `~/.config/depthpaper/config.toml`. Mostly managed by the CLI but you can edit it by hand.

```toml
[daemon]
parallax_intensity = 0.025
tracking_mode = "pointer"       # "pointer" (Wayland-native) or "hyprland" (IPC)
idle_timeout_secs = 300
battery_threshold = 20          # percent; 0 to disable

[inference]
model_path = "~/.local/share/depthpaper/models/depth_anything_v2_small.onnx"

[wallpaper]
color = "~/.cache/depthpaper/wallpapers/abc123.color.png"
# depth is inferred from color path; override with:
# depth = "/path/to/custom.depth16.png"
```

### Tracking modes

**pointer** (default): Uses Wayland pointer events. Works on any wlr-layer-shell compositor. Parallax only active when the cursor is over visible desktop. Stops rendering entirely when the cursor is over a window, which is good for battery.

**hyprland**: Polls the Hyprland IPC socket for global cursor position. Parallax stays active even when windows cover the desktop. Hyprland-only. Note that this lets the daemon observe cursor position over arbitrary windows.

## License

MIT