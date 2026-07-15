//! Survive `SIGBUS` when another process truncates a file we have mmapped.
//!
//! A [`memmap2::Mmap`] freezes the file's length at map time, but nothing
//! stops another process from truncating the file (or rotating a shorter one
//! over the same inode) afterwards. Touching a mapped page past the new EOF
//! then raises `SIGBUS`, which is not a `panic!` or an `Err` — it aborts the
//! whole process. For an engine whose contract is "never falls over", that is
//! the single worst failure mode: a shared log shrinking mid-`sort` kills the
//! editor and every other open tab with it (issue #200).
//!
//! This module turns that abort into an ordinary error:
//!
//! 1. Every long-lived read-only mapping registers its address range in a
//!    fixed-size process-global [`REGISTRY`] via [`MapWatch::watch`].
//! 2. The first registration installs a `SIGBUS` handler. When a fault lands
//!    inside a registered range, the handler maps a fresh anonymous page over
//!    the faulting page (so the interrupted load retries and reads zeros
//!    instead of faulting forever) and sets that registration's sticky
//!    `faulted` flag. Faults anywhere else are forwarded to whatever handler
//!    was installed before us (usually the Rust runtime's), preserving normal
//!    crash behaviour for genuine bugs.
//! 3. Readers call [`MapWatch::faulted`] at their existing error seams (per
//!    scan batch, before publishing results, before committing an output
//!    file) and turn a `true` into `Error::BaseFileChanged`.
//!
//! The registry is global rather than thread-local on purpose: the sparse
//! index is built on rayon worker threads, and viewport slices borrowed from
//! the map escape into callers, so the fault can surface on any thread at any
//! time while the mapping is alive. Registration therefore spans the mapping's
//! whole lifetime, not a single read.
//!
//! Zero-filled pages are safe: every consumer treats mapped bytes as untrusted
//! input already (bounds-checked slices, lossy decoding), and the sticky flag
//! guarantees any result computed over a zeroed page is discarded before it is
//! observable. The mapping stays poisoned until the document is reopened.
//!
//! On non-Unix platforms this module compiles to a no-op: Windows fails
//! `SetEndOfFile` on a file with live section mappings, so the shrink cannot
//! happen underneath us there.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Watches one mapping's address range for `SIGBUS` faults. Register with
/// [`MapWatch::watch`] immediately after creating the map and keep the watch
/// alive exactly as long as the mapping (drop it *before* the map unmaps so
/// the handler never resurrects a recycled address range).
#[derive(Debug)]
pub(crate) struct MapWatch {
    slot: Option<usize>,
}

impl MapWatch {
    /// Register `buf` (a live mapping) for fault absorption. An empty buffer,
    /// a full registry, or a non-Unix platform yields an inert watch — callers
    /// keep working, protected only by their stat-based length checks.
    pub(crate) fn watch(buf: &[u8]) -> MapWatch {
        MapWatch {
            slot: imp::register(buf),
        }
    }

    /// True once any read inside the watched range faulted. Sticky: the pages
    /// are zero-holes from then on, so the mapping must not be trusted again.
    pub(crate) fn faulted(&self) -> bool {
        match self.slot {
            Some(i) => REGISTRY.slots[i].faulted.load(Ordering::Acquire),
            None => false,
        }
    }
}

impl Drop for MapWatch {
    fn drop(&mut self) {
        if let Some(i) = self.slot {
            imp::deregister(i);
        }
    }
}

/// One registered mapping. `start` doubles as the slot's state: [`FREE`],
/// [`CLAIMING`] (mid-registration), or the mapping's base address.
struct Slot {
    start: AtomicUsize,
    len: AtomicUsize,
    faulted: AtomicBool,
}

struct Registry {
    slots: [Slot; SLOTS],
}

const FREE: usize = 0;
const CLAIMING: usize = usize::MAX;
/// Upper bound on concurrently watched mappings (open documents + per-file
/// grep maps + spill offset tables). Overflow degrades to "no absorption",
/// never to an error.
const SLOTS: usize = 256;

#[allow(clippy::declare_interior_mutable_const)]
const EMPTY_SLOT: Slot = Slot {
    start: AtomicUsize::new(FREE),
    len: AtomicUsize::new(0),
    faulted: AtomicBool::new(false),
};

static REGISTRY: Registry = Registry {
    slots: [EMPTY_SLOT; SLOTS],
};

#[cfg(unix)]
mod imp {
    use super::{Registry, CLAIMING, FREE, REGISTRY};
    use std::mem::MaybeUninit;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Once;

    static INSTALL: Once = Once::new();
    static PAGE_SIZE: AtomicUsize = AtomicUsize::new(0);
    /// Handler installed before us, forwarded to for faults that are not ours.
    /// Written exactly once under `INSTALL`, read only from the handler.
    static mut PREVIOUS: MaybeUninit<libc::sigaction> = MaybeUninit::uninit();

    pub(super) fn register(buf: &[u8]) -> Option<usize> {
        if buf.is_empty() {
            return None;
        }
        INSTALL.call_once(install);
        let start = buf.as_ptr() as usize;
        debug_assert!(start != FREE && start != CLAIMING);
        for (i, slot) in REGISTRY.slots.iter().enumerate() {
            if slot
                .start
                .compare_exchange(FREE, CLAIMING, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                // Publish the range only after len/faulted are in place; the
                // handler skips CLAIMING slots, so it never sees a torn entry.
                slot.faulted.store(false, Ordering::Release);
                slot.len.store(buf.len(), Ordering::Release);
                slot.start.store(start, Ordering::Release);
                return Some(i);
            }
        }
        None
    }

    pub(super) fn deregister(i: usize) {
        // The caller (MapWatch's owner) guarantees the mapping is still alive
        // here and unmaps strictly after; once `start` is FREE the handler can
        // no longer overlay pages in this (soon recycled) address range.
        REGISTRY.slots[i].start.store(FREE, Ordering::Release);
    }

    fn install() {
        // SAFETY: called exactly once. Installs a SIGBUS handler that only
        // touches async-signal-safe state (atomics and raw syscalls) and
        // forwards unrecognized faults to the previously installed handler.
        unsafe {
            let page = libc::sysconf(libc::_SC_PAGESIZE);
            PAGE_SIZE.store(
                if page > 0 { page as usize } else { 4096 },
                Ordering::Relaxed,
            );
            let mut sa: libc::sigaction = std::mem::zeroed();
            let h: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) = handler;
            sa.sa_sigaction = h as usize;
            // SA_ONSTACK: run on the sigaltstack std sets up, so a stack
            // overflow that arrives as SIGBUS (macOS) still reaches std's
            // handler through our forwarding with stack to spare.
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
            libc::sigemptyset(&mut sa.sa_mask);
            let prev: *mut libc::sigaction = (&raw mut PREVIOUS).cast();
            libc::sigaction(libc::SIGBUS, &sa, prev);
        }
    }

    extern "C" fn handler(sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        // SAFETY: async-signal context. Everything here is atomics, direct
        // syscalls, or a tail-call into the previous handler.
        unsafe {
            let addr = fault_addr(info);
            if addr != 0 {
                if let Some(slot) = find(&REGISTRY, addr) {
                    let page = PAGE_SIZE.load(Ordering::Relaxed).max(4096);
                    let base = addr & !(page - 1);
                    // Overlay one anonymous zero page so the interrupted load
                    // retries successfully. Read-only: these mappings are
                    // never written through.
                    let mapped = libc::mmap(
                        base as *mut libc::c_void,
                        page,
                        libc::PROT_READ,
                        libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_FIXED,
                        -1,
                        0,
                    );
                    if mapped != libc::MAP_FAILED {
                        slot.faulted.store(true, Ordering::Release);
                        return;
                    }
                    // Could not remap (address space exhaustion): fall through
                    // and crash via the previous handler rather than spin.
                }
            }
            forward(sig, info, ctx);
        }
    }

    fn find(registry: &Registry, addr: usize) -> Option<&super::Slot> {
        registry.slots.iter().find(|slot| {
            let start = slot.start.load(Ordering::Acquire);
            if start == FREE || start == CLAIMING {
                return false;
            }
            let len = slot.len.load(Ordering::Acquire);
            addr >= start && addr - start < len
        })
    }

    unsafe fn fault_addr(info: *mut libc::siginfo_t) -> usize {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            (*info).si_addr() as usize
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            (*info).si_addr as usize
        }
    }

    /// Deliver a fault that is not in any watched range to the handler that
    /// was installed before ours (Rust's runtime handler, or the default).
    unsafe fn forward(sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        let prev = *(&raw const PREVIOUS).cast::<libc::sigaction>();
        let action = prev.sa_sigaction;
        if prev.sa_flags & libc::SA_SIGINFO != 0
            && action != libc::SIG_DFL
            && action != libc::SIG_IGN
        {
            let f: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) =
                std::mem::transmute(action);
            f(sig, info, ctx);
        } else if action != libc::SIG_DFL && action != libc::SIG_IGN {
            let f: extern "C" fn(libc::c_int) = std::mem::transmute(action);
            f(sig);
        } else {
            // SIG_DFL — or SIG_IGN, which for a synchronous fault would retry
            // the same instruction forever. Restore the default action and
            // return; the faulting instruction re-executes and terminates the
            // process with the usual SIGBUS report.
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = libc::SIG_DFL;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

#[cfg(not(unix))]
mod imp {
    pub(super) fn register(_buf: &[u8]) -> Option<usize> {
        None
    }

    pub(super) fn deregister(_i: usize) {}
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;

    fn page_size() -> usize {
        // SAFETY: plain sysconf query.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        usize::try_from(page).unwrap_or(4096).max(4096)
    }

    /// Reads every byte through volatile loads so the compiler cannot skip
    /// the accesses that must fault.
    fn read_all(buf: &[u8]) -> u8 {
        let mut acc = 0u8;
        let mut i = 0;
        while i < buf.len() {
            // SAFETY: i < buf.len().
            acc ^= unsafe { std::ptr::read_volatile(buf.as_ptr().add(i)) };
            i += 64;
        }
        acc
    }

    #[test]
    fn truncation_fault_is_absorbed_and_flagged() {
        let page = page_size();
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&vec![b'x'; page * 4]).unwrap();
        f.flush().unwrap();

        // SAFETY: the fault this test provokes is exactly what MapWatch absorbs.
        let map = unsafe { memmap2::Mmap::map(f.as_file()).unwrap() };
        let watch = MapWatch::watch(&map);
        assert_eq!(read_all(&map), 0, "even number of 'x' bytes XORs to zero");
        assert!(!watch.faulted(), "no fault while the file is intact");

        // Shrink the file under the live mapping — pages past EOF now SIGBUS.
        f.as_file().set_len(1).unwrap();
        let _ = read_all(&map);
        assert!(
            watch.faulted(),
            "reads past the new EOF must set the sticky fault flag instead of aborting"
        );
        // The flag stays set even after further clean reads of the first page.
        let _ = read_all(&map[..1]);
        assert!(watch.faulted());
    }

    #[test]
    fn watch_slots_recycle_after_drop() {
        let data = vec![b'y'; 4096];
        let first = MapWatch::watch(&data);
        let slot = first.slot;
        assert!(slot.is_some());
        drop(first);
        // A watch on another buffer can reuse the freed slot; more importantly
        // registration keeps succeeding after many register/drop cycles.
        for _ in 0..SLOTS * 2 {
            let w = MapWatch::watch(&data);
            assert!(w.slot.is_some());
            assert!(!w.faulted());
        }
    }
}
