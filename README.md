# WPE Gallery

A native Rust desktop gallery built with [GPUI Kit](https://github.com/longbridge/gpui-kit) for installed
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
- Atomic configuration writes; seamless renderer swap through a systemd reload.

The library opens directly on the grid. Drag the divider to resize the inspector.
Its display menu sets the Apply target. The pearl outline marks the wallpaper
being inspected, and the **On** badge marks the target display’s saved assignment.
The gear controls thumbnail size, artwork edge lighting, Safe only and review
reloads. Gallery appearance and sidebar width are saved in
`$XDG_CONFIG_HOME/wpe-gallery/preferences.json` (normally
`~/.config/wpe-gallery/preferences.json` on Linux).

The gallery is deliberately local-only in v0.2: no Workshop
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

Left/Right select the previous/next wallpaper, Up/Down the one in the row
above/below; the selected tile is kept in view.
Tab cycles the target output, Enter applies, Ctrl+Q quits. The display menu, wallpaper tiles and Apply button provide the same actions by pointer.

Diagnostics:

```sh
wpe-gallery --scan                    # JSON library, metadata, warnings, outputs
wpe-gallery --select 1234567890       # initially select an installed Workshop ID
WPE_GALLERY_LOG=info wpe-gallery      # GPUI/renderer log (default: warn) on stderr
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
The launcher requires the dedicated /usr/bin/wpe-gallery-launch helper (built alongside the GUI) and the renderer at /usr/bin/linux-wallpaperengine. Install both release binaries before enabling the unit. Before upgrading from 0.1, a missing helper fails closed and does not open a GUI.

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

Only reviewed-safe, hash-matching items can be applied. Only the chosen output changes; existing assignments for other outputs survive.
The file is written to a temporary file in the same directory, synced and renamed.
Malformed or future-version configurations produce an error rather than being
overwritten. Run one app instance when editing assignments.

The launcher verifies every selected item and passes its absolute verified directory, avoiding a different Steam library being resolved by the renderer. It sorts output names and constructs one argument array:

```text
linux-wallpaperengine --silent --fps WPE_FPS   --fullscreen-pause-only-active --scaling fill   --screen-root DP-1 --bg /verified/SteamLibrary/steamapps/workshop/content/431960/1234567890 --scaling fill   --screen-root HDMI-A-1 --bg /verified/SteamLibrary/steamapps/workshop/content/431960/2345678901 --scaling fill
```

It preserves WPE_FPS (30 by default) and the existing fullscreen pause rule.
Before assignments.json exists, WPE_BG and WPE_OUTPUT from wallpaper.env retain
their single-display behavior, subject to the same whitelist checks; unset WPE_OUTPUT uses the first active Wayland output (niri fallback). Remove assignments.json to return to that mode.
Disconnected-output assignments are retained; edit the file to remove stale names
and reopen the app after display changes. Hotplug handling is future work.

The properties map is reserved. Upstream currently treats --set-property globally;
the launcher rejects nonempty maps instead of misapplying one output's values to
all wallpapers. Service-start success does not guarantee a wallpaper will render:
inspect `journalctl --user -u linux-wallpaperengine.service` for renderer failures.

### Seamless swap

`wpe-gallery-launch` stays the unit's main process and supervises the renderer.
Apply runs `systemctl --user reload-or-restart linux-wallpaperengine.service`;
with the sample unit's `ExecReload`, SIGHUP makes the launcher re-verify every
assignment, start a second renderer, and kill the old one only after the new
one has presented frames. niri and wlroots stack the newer layer surface on top,
so every output goes straight from the old wallpaper to the new one, with no bare
background in between. Readiness is the renderer's own `--screenshot` file in a
private temporary directory; after 15 s without it (for example, paused behind a
fullscreen window) the old renderer is replaced anyway. A replacement that exits
before presenting leaves the current renderer running and is logged; a reload in
which no assignment verifies stops the renderer, as a restart would. Units without
`ExecReload` are restarted. The reload needs wpe-gallery 0.2.5 or newer; with
an older exec-style launcher the renderer exits on SIGHUP and the unit restarts
it, with the old gap.
`wallpaper.env` is read when the unit starts, so restart the unit after
editing it. Video wallpapers draw black until their first
decoded frame (about 0.6 s), which the swap cannot hide because the new surface
is on top.

## Preview resource budget

Only virtualized visible rows request thumbnails. One thumbnail worker and one
selected-preview worker cap concurrent decodes at two. Changing selection cancels
the previous request; cancelled GIF work stops between frames. Static grid tiles
do not animate. Only the selected details image uses GPUI's animation scheduling.

Grid thumbnails use the tile width in display pixels, rounded up in 128-pixel steps
to a maximum of 1024×576. The grid center-crops before downsampling and preserves
native detail in smaller sources. Selected previews fit 480×270, GIFs use at most 48 frames at
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
cargo clippy --locked --all-targets -- -D warnings
python3 tests/hash_compatibility.py
```

### Vendored patches

The vendored xattr 0.2.3 is a transitive GPUI dependency patched solely to use
Linux ENODATA instead of libc's removed ENOATTR alias. Its upstream MIT/Apache
licenses are retained. Cargo.lock pins the dependency graph.

The vendored blade-graphics 0.7.1 (MIT, kvark/blade; GPUI 0.2.2's Linux
renderer) is patched in `src/vulkan/surface.rs` and `src/vulkan/mod.rs`. When
`vkAcquireNextImageKHR` returns `VK_ERROR_OUT_OF_DATE_KHR`, upstream returned an
unpresentable frame and never rebuilt the swapchain, so every later frame was
silently dropped until the window changed size. NVIDIA's Wayland driver does this
right after the first frame on niri (niri-wm/niri#2335, zed-industries/zed#50734):
the window kept showing `Verifying…` while layout and hit-testing had moved on.
The patched surface rebuilds its swapchain with its last configuration and
acquires again. The crate-level lint allow only silences upstream warnings that
Cargo no longer caps for a path dependency.

Feature comparison references:
[Windows browser and playlist changes](https://store.steampowered.com/news/posts/?appgroupname=Wallpaper+Engine&appids=431960&enddate=1623502818&feed=steam_community_announcements),
[application rules and display profiles](https://help.wallpaperengine.io/en/functionality/wallpaperperapp.html),
[Wallpaper Engine editor](https://docs.wallpaperengine.io/en/scene/overview.html).
Renderer compatibility belongs in linux-wallpaperengine; gallery issues tagged
`upstream` distinguish that work.

MIT licensed; see [LICENSE](LICENSE).

## Safety reviews and content verification

The whitelist lives at `$XDG_CONFIG_HOME/wpe-gallery/whitelist.json`
(default `~/.config/wpe-gallery/whitelist.json`). It is private user-maintained
configuration. WPE Gallery ships no approved IDs and never approves an item itself.
The format is one JSON object keyed by Workshop ID:

```json
{
  "1234567890": {
    "verdict": "safe",
    "reviewed": "2026-11-16",
    "reviewer": "Your reviewer name",
    "notes": "What was inspected and any limitations",
    "content_hash": "0000000000000000000000000000000000000000000000000000000000000000"
  },
  "2345678901": {
    "verdict": "blocked",
    "reviewed": "2026-11-16",
    "reviewer": "Your reviewer name",
    "notes": "Reason this item must not run",
    "content_hash": "0000000000000000000000000000000000000000000000000000000000000000"
  }
}
```

Zero hashes above are placeholders, not reviews. Use `blocked` and explain the
reason in `notes` for blocklist entries. A safe entry must have a lowercase
64-character SHA-256 content hash. Required fields are verdict, reviewed
(YYYY-MM-DD), reviewer (nonempty), notes and content_hash. Missing/malformed
whitelists authorize nothing. Entries may optionally include `type` (for example
`scene`, `video` or `web`) and other producer metadata. Unknown per-entry fields
are ignored; they do not authorize an item or replace any required field or its
validation.

Safe-only filtering starts on. Tiles and details show Safe, Unreviewed or Blocked.
A safe verdict becomes Safe only after its complete local tree matches its review.
Changed content is Unreviewed and requires a new review. Toggle Safe only off to
inspect other items; this does not permit applying them. Reload reviews after
editing the file or after Steam updates. Verification runs off the UI thread;
Apply always rereads the whitelist and rehashes, independently of cached badges.

To keep startup fast, the gallery's badge pass caches hashes in
`$XDG_CACHE_HOME/wpe-gallery/hashes.v1` (default `~/.cache/wpe-gallery/hashes.v1`,
mode 0600). Each record holds the Workshop ID, canonical item path, a digest of
the tree's lstat fingerprint (relative path, type, size, mtime, device, inode and
ctime of every entry including the root) and the resulting content hash. An item
is reread only when its fingerprint changes. Userland can restore mtime but not
ctime, so an ordinary edit is rehashed even if its size and mtime are restored.
The cache is used only on filesystems where the kernel owns ctime (ext4, btrfs,
xfs, f2fs, tmpfs, bcachefs) and only for trees whose newest ctime predates
hashing by two seconds. ctime is still not proof of unchanged bytes (writes
through a shared mmap can skip it), so the cache only affects badges: Apply, the
launcher and Recheck always hash the whole tree. Deleting the file is always
safe; a corrupt or unwritable cache only costs a rehash.

To compute a hash without approving or executing anything:

```sh
wpe-gallery --content-hash /path/to/steamapps/workshop/content/431960/1234567890
wpe-gallery --renderer-args  # verify and print the proposed CLI; no renderer/unit starts
```

The final hash contract is `wpe-gallery-content-v1`: ASCII domain name plus NUL;
all files and directories below the item root, sorted by UTF-8 relative POSIX
path bytes; ASCII D/F tag, big-endian u64 path byte length, path bytes; files add
big-endian u64 file byte length and raw bytes. Hidden files, previews, project.json,
original scene.pkg bytes and empty directories all count. Root path, timestamps,
permissions and unpacked package entries do not. Symlinks, special files,
non-UTF-8 names and detected file/directory changes fail verification. Duplicate
installed copies of one ID are refused instead of choosing a potentially
different copy. The [triage scanner](tools/triage/README.md) uses the same format.

The dedicated non-GUI launcher shares this verifier. It validates requested
assignments, then tries the corresponding last-safe assignment for each rejected
output and logs the rejection/fallback to stderr (the service journal). Last-safe
assignments are stored in `linux-wallpaperengine/last-safe-assignments.json`
after a verified launch plan and are fully rechecked against the current whitelist
and disk on every use. If neither request nor fallback verifies, that output is
omitted. If none verifies, no renderer starts. Legacy WPE_BG also requires a review.
A missing helper when upgrading from 0.1 fails closed.

A review and matching hash are an authorization record, not a malware detector
or sandbox. Hashes cover the reviewed local files, not downloaded network content,
renderer assets, dependencies or files changed after verification. Close Steam
or pause updates while reviewing/applying when a stable snapshot is needed.
Renderer/media-parser vulnerabilities and active tampering after verification
are outside this mechanism.

## Static wallpaper triage

The stdlib-only [triage tool](tools/triage/README.md) inventories Workshop files
and packed SceneScript without running wallpapers. Its documentation specifies
the `wpe-gallery-content-v1` SHA-256 contract for whitelist integration. Private scan
results and review decisions are kept outside this repository.


Pointer regression (requires an agent-owned headless sway, Pillow, clang and Wayland
development tools; installs nothing):

```sh
python3 tests/gui_pointer.py /private/path/headless-env.json
```

The environment JSON must specify the test compositor's WAYLAND_DISPLAY and
SWAYSOCK. The test refuses non-HEADLESS outputs, uses synthetic wallpapers and a
mock systemctl, and stops its app and virtual pointer afterwards. The caller owns
compositor teardown. Do not point it at a live desktop.


Initial tiled-window repaint regression (requires Pillow in addition to the GUI
test tools):

```sh
python3 tests/gui_startup.py /private/path/nested-niri-env.json
```

Start headless sway in a private XDG_RUNTIME_DIR (mode 0700), unset inherited
WAYLAND_DISPLAY, DISPLAY and NIRI_SOCKET, then start nested niri inside that sway.
Use a fractional-scale output and a normal initial column, for example
`output "winit" { scale 1.25; }` and
`layout { default-column-width { fixed 1331; }; }`.
Pass the nested compositor's environment JSON. The test builds
`tests/vulkan/out_of_date_layer.c` (needs `cc`) and loads it as an explicit
Vulkan layer for the app only; it makes the first configured swapchain report
out-of-date after two frames, reproducing the NVIDIA/niri failure on any GPU.
If zenity is installed it takes focus so the gallery is idle and unfocused
when verification completes. The test verifies the app's
runtime and sockets before screenshots, waits for an asynchronous hash check to
finish without resizing, asserts painted thumbnails and logged swapchain
recovery, then exercises column resize and fullscreen/return. It also captures a malformed-whitelist diagnostic.
Stop both test compositors afterwards. No test should use the live runtime.

During verification the gallery shows `Verifying N wallpapers…` with progress.
A whitelist read/parse failure shows the full path and diagnostic in the gallery;
it does not display the completed-but-empty safe-library message. Reload reviews
after correcting the file.
