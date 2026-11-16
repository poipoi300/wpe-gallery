# v0.1 validation

Tested on Arch Linux with an NVIDIA Vulkan driver. The installed Steam library
contained 476 valid Wallpaper Engine projects; scan returned all 476 with no
skipped items.

GUI checks ran on headless sway with a 5120×1440 output at scale 1.25.
The selected GIF changed between captures while the thumbnail grid remained
unchanged. Long titles stayed inside their tiles; selecting initially by ID
scrolled the highlighted tile into view. Independent review cleared those fixes
and the viewport-scaled cache, cancellable selection decoder and platform gating.

With a static selection after decoding settled, the release app used 0.10% of one
CPU core over a 10-second sample and 360,992 KiB RSS (about 353 MiB).
These are one-system observations, including GPUI/Vulkan allocations, not
cross-platform guarantees.

Apply was tested with XDG_CONFIG_HOME pointing at a scratch directory and a
mock systemctl on PATH: it wrote HEADLESS-1's assignment and invoked exactly
--user restart linux-wallpaperengine.service. The renderer launcher was also run
on the invisible HEADLESS-1 output with scene 3633967177, FPS 240, fullscreen pause
and --silent. It stayed alive through the test. The renderer confirmed that audio
was disabled. It logged a missing MDLV mesh and needed SIGKILL after taking longer
than 10 seconds to exit following SIGTERM.

An earlier real-unit test, before isolation requirements changed, wrote the
assignment and restarted the unit. DVD Screensaver (web, 1589757429) trapped in the
renderer CEF library. The original scene 3633967177 was restored and the service
was verified active with FPS 240. The CEF investigation is tracked as upstream #9.

Checks: cargo build --release --locked; cargo test --locked (2 contract tests);
Python launcher contract suite (3 tests); cargo fmt; cargo clippy -- -D warnings.
The transitive proc-macro-error2 dependency emits a future compatibility warning.
Linux is the validated build; macOS/Windows builds and Apply backends remain future
work. Headless tests use the real local library but contain no copyrighted preview
artwork in the repository.

## v0.2 follow-up

Headless sway at 5120×1440 scale 1.25 with a real Wayland virtual pointer:
image/title selection, a held press across redraws, pointer Apply, safe-only
filtering and blocked/unreviewed/changed-item refusal passed using synthetic
fixtures and a mocked systemctl. No live windows, audio or wallpaper-service
restart were used in this follow-up. The exact old-version selection failure
was not reproduced in isolated sway or nested niri; selection now happens on
mouse-down rather than depending on a later mouse-up in the virtualized list.

Six Rust unit tests and two CLI integration tests pass. They cover missing,
malformed, safe, blocked and changed reviews; last-safe fallback revalidation;
verified renderer paths; symlink rejection; and atomic replacement during hashing.
A Python compatibility test agrees with the independent triage implementation
for hidden/Unicode file names and empty directories. Three real reviewed hashes
were also checked read-only against the user's private whitelist; no review
entries or private data are included here. Independent security review cleared
the hash-contract and file-replacement findings after their fixes.


## v0.2.2 startup repaint

The private-runtime headless sway + nested niri regression starts the gallery in
a 1331-logical-pixel column at scale 1.25. A synthetic 4 GiB sparse file keeps
verification running beyond the first configure/paint. The initial capture shows
Verifying, and the completion capture contains thumbnails without any intervening
resize or input. Subsequent same-column-count resize and fullscreen/return keep
tiles painted. Broken-whitelist capture shows its path and parse diagnostic.

The reported 0.2.1 failure was not reproduced with these synthetic fixtures.
The completion paths now update through the owning window and explicitly request
a repaint, while list height comes from its flex viewport even when initially
empty. Geometry uses the configured content viewport and observes configure
changes. Non-GUI tests cover first-configure geometry and distinct checking,
failed and completed empty states.


## v0.2.3 NVIDIA/niri repaint

Root cause of "Verifying… 0 of 464" until resize: GPUI 0.2.2 renders through
blade-graphics 0.7.1, whose `acquire_frame` turned `VK_ERROR_OUT_OF_DATE_KHR`
into a frame that is never presented and never rebuilt the swapchain. NVIDIA's
Wayland driver reports out-of-date after the first frame on niri. Every later
draw (verification progress, tiles, hit-test regions) ran but was dropped until a
size change reconfigured the swapchain. Earlier headless runs used RADV on the
integrated GPU (Mesa's device selection), not NVIDIA, which is why they passed.

Validation used private headless sway (5120×1440, scale 1.25) with nested niri
using the user's layout/window rules, NVIDIA Vulkan forced (`ZED_DEVICE_ID`), the
real 464-item library with evicted page cache and the real whitelist read-only,
and the gallery unfocused while verification finished. With the test layer, the
installed 0.2.2 reproduced the report exactly (0 of 464, Verifying, no tiles);
0.2.3 recovered the swapchain and painted 438 verified tiles without resize.
`tests/gui_startup.py` fails on 0.2.2 and passes on 0.2.3; `tests/gui_pointer.py`
passes. Idle after verification: 0.0–0.1% of one core; GPUI still answers
compositor frame callbacks (about one wakeup per refresh) while mapped.
