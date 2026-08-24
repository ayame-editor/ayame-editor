// Ambient declarations for the non-standard globals the native host (wry/tao)
// injects on `window`, plus a couple of legacy fields the editor probes. This
// file is type-only — build.rs skips `.d.ts`, so nothing is emitted or served.
export {};

declare global {
  interface Window {
    /** Native IPC bridge, present only in the desktop (gui) build. */
    ipc?: { postMessage(msg: string): void };
    /** Legacy IE-era clipboard fallback probed in the paste handler. */
    clipboardData?: DataTransfer;
    /** Native → page: open real file paths dropped on the window. */
    __ayameOpenNativePaths?: (paths: unknown) => void;
    /** Native → page: the window was asked to close. */
    __ayameNativeCloseRequested?: () => void;
    /** Native menu bar → page: run an in-app menu action by id. */
    __ayameMenu?: (action: string) => void;
    /** Native launch-with-file: path to open once the UI is ready. */
    __ayamePendingOpen?: string;
    /** Native launch caret using the documented 1-based coordinates. */
    __ayamePendingPosition?: { line: number; column: number };
    /** Authenticated native instance → page open request (--reuse-window). */
    __ayameReuseOpen?: (request: {
      path?: string | null;
      position?: { line: number; column: number } | null;
    }) => void;
    /** Set alongside __ayamePendingOpen by a dirty-tab handoff (issue #35):
     *  replay the path's detached crash log without the crash prompt. */
    __ayamePendingRecover?: boolean;
    /** Native → page: result of the OS save dialog (path or null). */
    __ayameSaveDialogDone?: (path: unknown) => void;
    /** Native → page: result of the OS open dialog (paths or null). */
    __ayameOpenDialogDone?: (paths: unknown) => void;
  }
}
