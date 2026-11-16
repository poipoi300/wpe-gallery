# WPE Gallery

A small native Rust / [GPUI](https://www.gpui.rs/) desktop gallery for installed
Wallpaper Engine Workshop wallpapers. On Linux, apply a wallpaper to a display
through [linux-wallpaperengine](https://github.com/Almamu/linux-wallpaperengine).

WPE Gallery is independent of Wallpaper Engine and Valve. It does not include
wallpapers or renderer assets. You need a licensed Steam installation of
Wallpaper Engine (app 431960), its assets, and downloaded Workshop items.

## What works

- Discover Steam libraries from libraryfolders.vdf, including additional drives.
- Virtualized gallery, local titles, types, tags, descriptions and property counts.
- JPEG/PNG/WebP thumbnails; selected GIF animation.
- Wayland output names and native resolutions, with niri JSON fallback.
- Persistent independent output assignments; one renderer process for all outputs.
- Atomic configuration writes and asynchronous systemd user-service restart.

The layout follows the familiar display strip, wallpaper grid and right-hand
details pane. The gallery is deliberately local-only in v0.1: no Workshop
subscription management, search, filters, playlists, or property editor yet.
See the [issue tracker](https://github.com/poipoi300/wpe-gallery/issues).

## Build and run

Install a current stable Rust toolchain. On Arch Linux, build prerequisites are:

```sh
sudo pacman -S --needed base-devel rustup clang cmake pkgconf   wayland libxkbcommon libx11 libxcb libxrandr libxi libxcursor   fontconfig freetype2 vulkan-icd-loader
rustup default stable
git clone https://github.com/poipoi300/wpe-gallery.git
cd wpe-gallery
cargo build --release --locked
./target/release/wpe-gallery
```

A Vulkan-capable graphics driver is required by GPUI on Linux. Install
linux-wallpaperengine separately; a package providing that name is sufficient.

Left/Right select the previous/next wallpaper and bring its tile into view.
Tab cycles the target output, Enter applies, Ctrl+Q quits. Click a display,
tile, or Apply button for the same actions.

Diagnostics:

```sh
wpe-gallery --scan                    # JSON library, metadata, warnings, outputs
wpe-gallery --select 1234567890       # initially select an installed Workshop ID
```

Linux is the tested platform. Platform-independent library and UI code use GPUI's
macOS/Windows backends, but those builds are not yet validated. Apply is a trait
with a Linux implementation; other platforms report that Apply is unsupported.
Non-Wayland output enumeration is not implemented yet.

## Linux renderer integration

The app expects a user unit named `linux-wallpaperengine.service`.
If you already manage that unit, adapt its launcher to the config below.
Otherwise the optional integration files provide a working starting point:

```sh
install -Dm755 integration/wallpaperengine-start ~/.local/bin/wallpaperengine-start
install -Dm644 integration/linux-wallpaperengine.service   ~/.config/systemd/user/linux-wallpaperengine.service
mkdir -p ~/.config/linux-wallpaperengine
printf 'WPE_FPS=60\nWPE_BG=YOUR_WORKSHOP_ID\n' >   ~/.config/linux-wallpaperengine/wallpaper.env
systemctl --user daemon-reload
systemctl --user enable --now linux-wallpaperengine.service
```

The sample unit is tied to `niri.service`; on other desktop environments adjust
its PartOf/After/WantedBy to your graphical session. Export Wayland session
variables to your systemd user manager using your desktop's session setup.
The launcher requires Python 3 and the renderer at /usr/bin/linux-wallpaperengine.

Apply writes `$XDG_CONFIG_HOME/linux-wallpaperengine/assignments.json`
(default `~/.config/linux-wallpaperengine/assignments.json`):

```json
{
  "version": 1,
  "outputs": {
    "DP-1": {"wallpaper_id": "1234567890", "properties": {}},
    "HDMI-A-1": {"wallpaper_id": "2345678901", "properties": {}}
  }
}
```

Only the chosen output changes; existing assignments for other outputs survive.
The file is written to a temporary file in the same directory, synced and renamed.
Malformed or future-version configurations produce an error rather than being
overwritten. Run one app instance when editing assignments.

The launcher sorts output names and constructs one argument array:

```text
linux-wallpaperengine --silent --fps WPE_FPS   --fullscreen-pause-only-active --scaling fill   --screen-root DP-1 --bg 1234567890 --scaling fill   --screen-root HDMI-A-1 --bg 2345678901 --scaling fill
```

It preserves WPE_FPS (30 by default) and the existing fullscreen pause rule.
Before assignments.json exists, WPE_BG and WPE_OUTPUT from wallpaper.env retain
their single-display behavior; unset WPE_OUTPUT falls back to the first enabled
niri output. Remove assignments.json to return to that mode.
Disconnected-output assignments are retained; edit the file to remove stale names
and reopen the app after display changes. Hotplug handling is future work.

The properties map is reserved. Upstream currently treats --set-property globally;
the launcher rejects nonempty maps instead of misapplying one output's values to
all wallpapers. Service-start success does not guarantee a wallpaper will render:
inspect `journalctl --user -u linux-wallpaperengine.service` for renderer failures.

## Preview resource budget

Only virtualized visible rows request thumbnails. One thumbnail worker and one
selected-preview worker cap concurrent decodes at two. Changing selection cancels
the previous request; cancelled GIF work stops between frames. Static grid tiles
do not animate. Only the selected details image uses GPUI's animation scheduling.

Thumbnails fit 256×144, selected previews 480×270, GIFs use at most 48 frames at
at most about 15 fps, and files over 32 MiB are rejected. GIF source dimensions
are capped at 16 megapixels; image decoder allocation limits also apply.
The thumbnail LRU scales with viewport capacity plus in-flight headroom.
There are no library polling timers or decode loops while idle.
Long GIFs loop a truncated preview, not their entire original duration.

## Packaging and development

`assets/` includes the desktop entry and SVG icon. Install the binary into
/usr/bin, the desktop entry into /usr/share/applications, and the icon into
/usr/share/icons/hicolor/scalable/apps.

```sh
cargo test --locked
cargo fmt --check
cargo clippy --locked -- -D warnings
python3 tests/launcher.py
```

The vendored xattr 0.2.3 is a transitive GPUI dependency patched solely to use
Linux ENODATA instead of libc's removed ENOATTR alias. Its upstream MIT/Apache
licenses are retained. Cargo.lock pins the dependency graph.

Feature comparison references:
[Windows browser and playlist changes](https://store.steampowered.com/news/posts/?appgroupname=Wallpaper+Engine&appids=431960&enddate=1623502818&feed=steam_community_announcements),
[application rules and display profiles](https://help.wallpaperengine.io/en/functionality/wallpaperperapp.html),
[Wallpaper Engine editor](https://docs.wallpaperengine.io/en/scene/overview.html).
Renderer compatibility belongs in linux-wallpaperengine; gallery issues tagged
`upstream` distinguish that work.

MIT licensed; see [LICENSE](LICENSE).
