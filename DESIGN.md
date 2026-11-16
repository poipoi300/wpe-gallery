---
name: WPE Gallery
description: A compact native wallpaper gallery with a persistent details pane.
colors:
  canvas: "#181b20"
  pane: "#22262d"
  artwork-ground: "#121519"
  foreground: "#edf0f4"
  secondary-text: "#b6beca"
  control: "#303741"
  control-hover: "#414c5b"
  divider: "#363d47"
  display-selected: "#325c8a"
  apply: "#2866ad"
  selection: "#569dff"
typography:
  body:
    fontFamily: "sans-serif"
    fontSize: "0.875rem"
    fontWeight: 400
  app-title:
    fontFamily: "sans-serif"
    fontSize: "1.125rem"
    fontWeight: 600
  wallpaper-title:
    fontFamily: "sans-serif"
    fontSize: "1.25rem"
    fontWeight: 600
rounded:
  sm: "0.125rem"
spacing:
  compact: "0.25rem"
  small: "0.5rem"
  related: "0.75rem"
  group: "1rem"
  pane: "1.25rem"
components:
  button-primary:
    backgroundColor: "{colors.apply}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.sm}"
    padding: "0.5rem 1rem"
  button-secondary:
    backgroundColor: "{colors.control}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.sm}"
    padding: "0.5rem 1rem"
  button-hover:
    backgroundColor: "{colors.control-hover}"
  display-selected:
    backgroundColor: "{colors.display-selected}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.sm}"
    padding: "0.5rem 1rem"
  tag:
    backgroundColor: "{colors.control}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.sm}"
    padding: "0.25rem 0.5rem"
  wallpaper-tile:
    backgroundColor: "{colors.pane}"
    textColor: "{colors.foreground}"
---

# Design System: WPE Gallery

## Overview

**Creative North Star: "Native gallery"**

The Windows Wallpaper Engine composition is the user's visual reference.
A graphite work surface, compact system sans typography, and blue selection
keep the controls quiet while wallpaper artwork carries the color.

**Key Characteristics:**
- Dense artwork browsing with titles immediately beneath previews.
- Persistent display selection and a separate details pane.
- Blue identifies selection and the Apply action.

## Colors

### Primary

Selection blue outlines the chosen wallpaper. Apply blue fills the main action;
a muted blue fills the chosen display control.

**The Artwork Leads Rule.** Keep interface color concentrated in selection and
actions so installed wallpaper previews remain the visual content.

### Neutral

Canvas separates the gallery from the slightly lighter pane. The darkest ground
backs artwork and missing-preview placeholders. Foreground carries titles and
controls; secondary text carries metadata, descriptions, and shortcut hints.
Control surfaces and dividers establish boundaries without shadows.

## Typography

All text uses the platform-resolved sans-serif family. The app title and selected
wallpaper title are semibold; gallery labels and metadata use regular body text.
The frontmatter records the three sizes actually used in the GPUI implementation.

Tile titles stay on one line and truncate with an ellipsis. The details title and
description wrap within the sidebar. Native font fallback determines coverage for
Workshop titles in additional languages.

## Layout

The display strip sits above the gallery and details pane, with status beneath.
The sidebar stays at 340 logical pixels and scrolls independently. Gallery rows
are virtualized, 164 logical pixels high, with 112 logical pixel artwork areas.
Columns adapt from one to twelve; equal computed widths keep long titles inside
their tiles. Related items use the spacing scale recorded above.

The selected row scrolls into view after a selection or column-count change.
The selected large preview has a 210 logical pixel area with contained artwork;
tile artwork fills its available area by cropping. The window minimum is
760 by 520 logical pixels. This is a desktop layout, with no mobile variant.

## Elevation & Depth

The interface uses tonal layers and borders. It has no authored shadows, blur, or
decorative elevation. A two-pixel blue outline distinguishes the selected tile;
the normal tile outline uses the control surface color.

## Shapes

Gallery tiles and pane boundaries are square. Buttons and tag chips have the
small corner radius in the frontmatter. Artwork is clipped to its tile area.

## Components

### Buttons

The shared button treatment uses compact vertical and wider horizontal padding.
Apply uses the primary fill; other controls use the control fill. Pointer hover
uses the shared hover fill. Disabled Apply is half opaque and has no click handler;
its label changes to “Applying…” while work runs. Status and errors appear below
the main area. Keyboard actions are handled at the gallery root.

### Display selection

Each output control shows its name and resolution. The selected output uses the
muted blue fill. The active display does not imply an applied wallpaper assignment.

### Tags

Tags are informational chips with a control-colored fill, small corners, and
compact padding. They are not interactive filters.

### Wallpaper tiles

Tiles join an artwork area and a truncated title. Hover changes the surface fill;
selection changes the outline. Missing artwork has a text placeholder.
Only the selected large GIF preview animates; gallery thumbnails remain static.

## Do's and Don'ts

### Do:
- **Do** keep artwork, its title, and selection state visually adjacent.
- **Do** use the existing tonal surfaces and blue action hierarchy.
- **Do** preserve visible loading, disabled, empty, and error feedback.

### Don't:
- **Don't** animate every gallery tile.
- **Don't** let long Workshop titles widen their gallery column.
- **Don't** present informational tags as working filters.
