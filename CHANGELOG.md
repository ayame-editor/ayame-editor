# Changelog

All notable changes to Ayame Editor are tracked here.

## Unreleased

- Added session restore for open tabs, active tab persistence, and shared
  server-backed recent-file/search-history state across native windows.
- Expanded UTF-16LE/UTF-16BE support for opening, reopening, converting, saving,
  search indexing, and folder grep.
- Improved Replace All so large result sets are paged automatically and applied
  against the original match set instead of stopping after the first chunk.
- Expanded Settings visibility controls and macOS native menus, including
  dynamic language refresh for the native menu bar.
- Added safe tab drag support for native-window workflows: clean tabs can move
  to another Ayame tabbar or drag out into a new window, while dirty tabs are
  kept in place to avoid losing unsaved edits.
- Added a complete CLI reference for every public `ayame` subcommand.
- Added architecture documentation for the Rust core, local server, web UI,
  crash-recovery WAL, type generation, and release automation.
- Added contributor guidance covering CI gates, Japanese documentation sync, and
  screenshot refreshes.
- Added documentation screenshots and expanded shortcut coverage for GUI actions
  that are configurable but unassigned by default.

Release artifacts are published from GitHub Actions and listed on the
[releases page](https://github.com/hjosugi/ayame-editor/releases).
