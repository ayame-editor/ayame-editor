# Ayame Design List

Figma connector が使えない場合でも共有できる、静的なデザイン一覧です。

- Browser catalog: [design-list.html](./design-list.html)
- Live iframe gallery: [index.html](./index.html)
- Current GUI prototype: [gui/iris-light.html](./gui/iris-light.html)
- Figma generation script: [figma/ayame-editor-design.use-figma.js](./figma/ayame-editor-design.use-figma.js)
- UI placement rationale (why everything sits where it does): [UX_RATIONALE.md](./UX_RATIONALE.md)

## Recommended Direction

| Priority | Design | Preview | Notes |
|---|---|---|---|
| 1 | Iris Light | [PNG](./renders/iris-light.png) / [Prototype](./gui/iris-light.html) | Primary direction. Light-only, editable JSON themes, compact native editor UI. |
| 2 | Iris Sumi Clean | [PNG](./renders/iris-sumi-clean.png) / [Prototype](./gui/iris-light.html?theme=sumi-light&panel=0) | Default-app candidate: quiet, readable, production-friendly. |
| 3 | Power Split | [PNG](./renders/17-power-split.png) / [Mockup](./mockups/17-power-split.html) | Best structure for huge-file search/diff workflows. |
| 4 | Command Palette | [PNG](./renders/16-command-palette.png) / [Mockup](./mockups/16-command-palette.html) | Best command/navigation model for advanced editor operations. |

## Full Catalog

| # | Design | Category | Preview | Mockup |
|---|---|---|---|---|
| 00 | Graphite | Reference | [PNG](./renders/00-graphite.png) | [_base.html](./_base.html) |
| 01 | Ayame Zen | Japanese | [PNG](./renders/01-ayame-zen.png) | [HTML](./mockups/01-ayame-zen.html) |
| 02 | Kanagawa | Japanese | [PNG](./renders/02-kanagawa.png) | [HTML](./mockups/02-kanagawa.html) |
| 03 | Sumi-e | Japanese | [PNG](./renders/03-sumi-e.png) | [HTML](./mockups/03-sumi-e.html) |
| 04 | Neon Glass Terminal | Neon | [PNG](./renders/04-neon-glass.png) | [HTML](./mockups/04-neon-glass.html) |
| 05 | Synthwave '84 | Neon | [PNG](./renders/05-synthwave.png) | [HTML](./mockups/05-synthwave.html) |
| 06 | Matrix Phosphor | Hacker | [PNG](./renders/06-matrix.png) | [HTML](./mockups/06-matrix.html) |
| 07 | Tokyo Night | Dev classic | [PNG](./renders/07-tokyo-night.png) | [HTML](./mockups/07-tokyo-night.html) |
| 08 | Catppuccin Mocha | Dev classic | [PNG](./renders/08-catppuccin.png) | [HTML](./mockups/08-catppuccin.html) |
| 09 | Rose Pine Moon | Dev classic | [PNG](./renders/09-rose-pine.png) | [HTML](./mockups/09-rose-pine.html) |
| 10 | Nord Frost | Calm | [PNG](./renders/10-nord.png) | [HTML](./mockups/10-nord.html) |
| 11 | Gruvbox | Retro | [PNG](./renders/11-gruvbox.png) | [HTML](./mockups/11-gruvbox.html) |
| 12 | Alabaster Light | Light | [PNG](./renders/12-alabaster-light.png) | [HTML](./mockups/12-alabaster-light.html) |
| 13 | Frostbite Wireframe | Modern | [PNG](./renders/13-frostbite.png) | [HTML](./mockups/13-frostbite.html) |
| 14 | Aurora Silk Glass | Modern | [PNG](./renders/14-aurora-glass.png) | [HTML](./mockups/14-aurora-glass.html) |
| 15 | Amber CRT | Retro | [PNG](./renders/15-amber-crt.png) | [HTML](./mockups/15-amber-crt.html) |
| 16 | Command Palette | Layout | [PNG](./renders/16-command-palette.png) | [HTML](./mockups/16-command-palette.html) |
| 17 | Power Split | Layout | [PNG](./renders/17-power-split.png) | [HTML](./mockups/17-power-split.html) |
| Iris Dawn | Iris preset | Current direction | [PNG](./renders/iris-dawn.png) | [Prototype](./gui/iris-light.html?theme=iris-dawn) |
| Iris Light | Iris preset | Current direction | [PNG](./renders/iris-light.png) | [Prototype](./gui/iris-light.html?theme=iris-light) |
| Iris Mist Clean | Iris preset | Current direction | [PNG](./renders/iris-mist-clean.png) | [Prototype](./gui/iris-light.html?theme=iris-mist&panel=0) |
| Iris Mono | Iris preset | Accessibility | [PNG](./renders/iris-mono.png) | [Prototype](./gui/iris-light.html?theme=mono-paper) |
| Iris Sumi Clean | Iris preset | Current direction | [PNG](./renders/iris-sumi-clean.png) | [Prototype](./gui/iris-light.html?theme=sumi-light&panel=0) |

## Native Editor Coverage

The Figma-ready design script covers these native-editor screens:

- Main editor workspace with file tree, tabs, search bar, line gutter, range selection, rectangle selection, minimap, and status bar.
- All menu groups: file, edit, selection, view, tools.
- Open file dialog with date-variable filename templates.
- Settings dialog for theme, background, font, line number, search, save, huge-file, and keymap settings.
- Diff dialog inspired by delta / WinMerge with side-by-side hunks and dirty-buffer state.
- Keymap dialog, case conversion, selection replace, and component/token sheet.

