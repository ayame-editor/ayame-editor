// Ayame Editor — shared modal ownership check.
//
// Keep this module independent from feature controllers: keyboard, menus,
// search, and rendering all need the answer, but none should import another
// feature module merely to ask whether a blocking surface is visible.

const BLOCKING_SURFACE_IDS = [
  "prompt",
  "form-modal",
  "confirm",
  "settings",
  "keymap-modal",
  "command-palette",
  "grep-modal",
  "bookmark-modal",
  "analysis-modal",
  "opener",
  "convert-modal",
  "overlay",
];

export function anyModalOpen() {
  return BLOCKING_SURFACE_IDS.some((id) => {
    const element = document.getElementById(id);
    return !!element && !element.classList.contains("hidden");
  });
}
