# WPE Gallery
## Platform
Native cross-platform Rust desktop application using GPUI. Linux/Wayland is the
first supported Apply backend; other operating systems can browse local libraries.
## Users and task
Wallpaper Engine owners choose an installed Steam Workshop wallpaper, inspect its
preview and metadata, select an output, and apply it via linux-wallpaperengine.
## Contract
The implementation brief specifies a Windows Wallpaper Engine style gallery:
display selection above, gallery in the main area, selected-item details at right.
Discover Steam libraries rather than assuming a library location. Preserve renderer
FPS and pause configuration. Preview work must be bounded and idle work minimal.
## Stack
Rust, GPUI, systemd user service, linux-wallpaperengine. MIT licensed.
## Scope
Local installed wallpapers only in v1. Subscription, editing, scheduling, and
performance controls are tracked as future issues. No Steam account integration.
