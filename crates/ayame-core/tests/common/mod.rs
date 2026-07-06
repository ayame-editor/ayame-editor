//! Shared harness for the data-integrity verification suite (issue #54).
//!
//! Everything here is deliberately dependency-light: a `tempfile`-backed
//! document builder, a SHA-256 checksum utility (the issue asks for byte
//! comparison "by checksum"), a deterministic seeded RNG (so property/fuzz
//! runs are reproducible without pulling `proptest`), and a plain-`Vec`
//! reference model for the edit overlay. Each integration-test binary pulls
//! this in with `mod common;`, so some helpers are unused in any single
//! binary — hence the blanket `dead_code` allow.
#![allow(dead_code)]

use std::io::Write;
use std::path::PathBuf;

use ayame_core::{Document, EditSession, Encoding, OpenOptions};
use tempfile::{NamedTempFile, TempDir};

/// Write `bytes` to a temp file and open it with auto-detected encoding.
/// The returned `NamedTempFile` must be kept alive to keep the path valid.
pub fn open_doc(bytes: &[u8]) -> (NamedTempFile, Document) {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(bytes).unwrap();
    f.flush().unwrap();
    let doc = Document::open(f.path(), &OpenOptions::default()).unwrap();
    (f, doc)
}

/// Same as [`open_doc`] but forces a specific source encoding.
pub fn open_doc_as(bytes: &[u8], enc: Encoding) -> (NamedTempFile, Document) {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(bytes).unwrap();
    f.flush().unwrap();
    let opts = OpenOptions {
        encoding: Some(enc),
        ..Default::default()
    };
    let doc = Document::open(f.path(), &opts).unwrap();
    (f, doc)
}

/// A throwaway directory for op outputs. The transform/split/sort APIs refuse
/// to overwrite an existing target, so tests point each run at a fresh name
/// inside one of these.
pub fn scratch() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// Read a file back as raw bytes.
pub fn read(path: impl AsRef<std::path::Path>) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// SHA-256 of a byte slice as lowercase hex — the checksum utility the suite
/// uses to assert byte-exact equality (a mismatch is astronomically unlikely
/// to hash-collide, and the digest is far cheaper to eyeball in failures than
/// two multi-KB buffers).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(out, "{byte:02x}").unwrap();
    }
    out
}

/// Assert two byte buffers are identical, reporting via checksum + length so a
/// failure is legible even for large inputs.
#[track_caller]
pub fn assert_bytes_eq(actual: &[u8], expected: &[u8], what: &str) {
    if actual != expected {
        panic!(
            "{what}: bytes differ\n  expected len={} sha256={}\n  actual   len={} sha256={}",
            expected.len(),
            sha256_hex(expected),
            actual.len(),
            sha256_hex(actual),
        );
    }
}

/// The decoded, terminator-stripped logical view of an edit session — exactly
/// what the editor shows, line by line. This is the value the reference model
/// is compared against.
pub fn view(session: &EditSession, doc: &Document) -> Vec<String> {
    let n = session.total_lines(doc);
    (0..n)
        .map(|i| session.line(doc, i).map(|l| l.text).unwrap_or_default())
        .collect()
}

/// Reopen a saved file and return its logical lines (content only).
pub fn reopen_lines(path: &std::path::Path) -> Vec<String> {
    let doc = Document::open(path, &OpenOptions::default()).unwrap();
    let n = doc.line_count();
    (0..n).map(|i| doc.line(i).unwrap()).collect()
}

/// Deterministic SplitMix64 — reproducible randomness with no dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (returns 0 when `n == 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// A plain-`Vec` model of the logical lines, with snapshot-based undo/redo, used
/// as the trusted reference the real [`EditSession`] must match line-for-line.
///
/// It mirrors the crate's line-level ops (each successful mutation is one undo
/// generation), so as long as callers only feed it *real* changes it stays in
/// lockstep with the overlay's own history.
#[derive(Clone, Default)]
pub struct RefModel {
    pub lines: Vec<String>,
    undo: Vec<Vec<String>>,
    redo: Vec<Vec<String>>,
}

impl RefModel {
    pub fn from_doc(doc: &Document) -> Self {
        let n = doc.line_count();
        RefModel {
            lines: (0..n).map(|i| doc.line(i).unwrap()).collect(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn record(&mut self) {
        self.undo.push(self.lines.clone());
        self.redo.clear();
    }

    pub fn replace_line(&mut self, i: usize, text: String) {
        self.record();
        self.lines[i] = text;
    }

    pub fn insert_before(&mut self, i: usize, text: String) {
        self.record();
        self.lines.insert(i, text);
    }

    pub fn delete_line(&mut self, i: usize) {
        self.record();
        self.lines.remove(i);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Mirror an `undo`; returns whether a step was available.
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut self.lines, prev));
                true
            }
            None => false,
        }
    }

    /// Mirror a `redo`; returns whether a step was available.
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut self.lines, next));
                true
            }
            None => false,
        }
    }
}

/// A staged output path inside a scratch dir (never pre-created).
pub fn out_in(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}
