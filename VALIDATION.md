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
