---
name: WPE Gallery
description: A native wallpaper library with matte black glass surfaces and a persistent inspector.
colors:
  canvas: "#161717"
  pane: "#242626"
  artwork-ground: "#111212"
  foreground: "#f2f1ed"
  secondary-text: "#aaaead"
  control: "#292b2a"
  control-hover: "#333633"
  divider: "#373937"
  apply: "#d8e8da"
  apply-text: "#19251d"
  selection: "#b1ccb6"
typography:
  body:
    fontFamily: Inter
    fontSize: "14px"
    fontWeight: 400
  app-title:
    fontFamily: Inter
    fontSize: "18px"
    fontWeight: 600
  wallpaper-title:
    fontFamily: Inter
    fontSize: "25px"
    fontWeight: 600
rounded:
  control: "7px"
  tile: "9px"
spacing:
  tile-gap: "14px"
  pane: "20px"
  gallery: "24px"
components:
  button-primary:
    backgroundColor: "{colors.apply}"
    textColor: "{colors.apply-text}"
    rounded: "{rounded.control}"
  button-secondary:
    backgroundColor: "{colors.control}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.control}"
  wallpaper-tile:
    textColor: "{colors.foreground}"
    rounded: "{rounded.tile}"
  tag:
    textColor: "{colors.secondary-text}"
---

# WPE Gallery

## Surfaces and light

A single static grain image gives the black canvas a matte finish. A translucent inspector lets that texture show through. Each thumbnail supplies two color samples, averaged from the left and right halves with extra weight for brighter pixels. These colors tint the caption, the shadow under the artwork, and the selected preview's surround. Pearl green identifies the selected tile and the Apply action across all artwork colors.

## Layout

The header holds the application name and a display menu showing the active output and its resolution. The virtualized library has one to twelve columns of 16:9 crops. A title sits below each image and scrolls on hover when it exceeds the caption width. The filter and review reload control move below the library heading in narrow windows. Keyboard hints stay beneath the grid.

The 344-pixel inspector fits the entire selected preview in an aspect-aware frame. It lists tags, type, Workshop ID, property count, description and verification status. Review details expand to show the reviewer, date, notes and hash. The body scrolls independently above the pinned Apply button.

## Components and states

Use GPUI Kit controls, labels, icons and layout primitives throughout. The artwork and grain use GPUI image elements. Keep thumbnail frames static. Animate only the selected GIF and the hovered overflowing title.

Safe only defaults on. When it is off, an overlaid status tag identifies unverified or blocked items. The inspector gives the reason and disables Apply. Loading, verification errors and empty results have distinct messages.
