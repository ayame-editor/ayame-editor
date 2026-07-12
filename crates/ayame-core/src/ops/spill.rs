//! Shared external-merge primitives (issue #81.1).
//!
//! `sort` and `group` both spill sorted runs to disk and k-way-merge them, and
//! used to each hand-roll the run codec, the run reader, the sorted-run writer,
//! and the merge-heap element. Those live here now, generic over a fixed-width
//! [`Payload`] that rides next to the key: `sort` carries the line number
//! (`u64`), `group` carries its aggregate accumulator (`Acc`).
//!
//! What stays in `sort`/`group` is the merge *policy*, because it genuinely
//! differs: `sort` emits every record through a bounded multi-pass fan-in;
//! `group` folds equal keys in a single pass. Both drive the same
//! [`RunReader`] + [`HeapEntry`] underneath.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::marker::PhantomData;
use std::path::Path;

use rayon::prelude::*;

use super::common::read_full;
use crate::Result;

/// A fixed-width value stored next to a key in a spill run. `sort` uses `u64`
/// (the line number, 8 bytes); `group` uses its `Acc` (which carries an exact
/// sum expansion, a few hundred bytes). A generic `[0u8; P::LEN]` is not
/// allowed on stable, so [`RunReader`] keeps one `P::LEN`-sized buffer that
/// every `next_record` call reuses.
pub(super) trait Payload: Copy + Send + Sync {
    /// Serialized width in bytes (constant for the type).
    const LEN: usize;
    /// Append exactly `LEN` bytes.
    fn write_to(&self, out: &mut impl Write) -> std::io::Result<()>;
    /// Decode from a `LEN`-byte slice.
    fn read_from(bytes: &[u8]) -> Self;
}

impl Payload for u64 {
    const LEN: usize = 8;
    fn write_to(&self, out: &mut impl Write) -> std::io::Result<()> {
        out.write_all(&self.to_le_bytes())
    }
    fn read_from(bytes: &[u8]) -> u64 {
        u64::from_le_bytes(bytes.try_into().unwrap())
    }
}

/// Sort `records` by `cmp` (in parallel), then write them to a run file at
/// `path` framed as `[key_len: u32 LE][key][payload: Payload::LEN]`. Returns the
/// bytes written and leaves `records` empty (so the caller can reuse the buffer).
pub(super) fn write_run<P: Payload>(
    records: &mut Vec<(Vec<u8>, P)>,
    cmp: impl Fn(&(Vec<u8>, P), &(Vec<u8>, P)) -> Ordering + Sync,
    path: &Path,
) -> Result<u64> {
    records.par_sort_unstable_by(cmp);
    let mut w = BufWriter::new(File::create(path)?);
    let mut bytes = 0u64;
    for (key, payload) in records.iter() {
        w.write_all(&(key.len() as u32).to_le_bytes())?;
        w.write_all(key)?;
        payload.write_to(&mut w)?;
        bytes += 4 + key.len() as u64 + P::LEN as u64;
    }
    w.flush()?;
    records.clear();
    Ok(bytes)
}

/// Streams `(key, payload)` records back out of a run written by [`write_run`].
pub(super) struct RunReader<P> {
    r: BufReader<File>,
    /// Exactly `P::LEN` bytes, reused for every record.
    payload_buf: Vec<u8>,
    _payload: PhantomData<P>,
}

impl<P: Payload> RunReader<P> {
    pub(super) fn open(path: &Path) -> Result<RunReader<P>> {
        Ok(RunReader {
            r: BufReader::new(File::open(path)?),
            payload_buf: vec![0u8; P::LEN],
            _payload: PhantomData,
        })
    }

    pub(super) fn next_record(&mut self) -> Result<Option<(Vec<u8>, P)>> {
        let mut len_b = [0u8; 4];
        if !read_full(&mut self.r, &mut len_b)? {
            return Ok(None);
        }
        let mut key = vec![0u8; u32::from_le_bytes(len_b) as usize];
        self.r.read_exact(&mut key)?;
        self.r.read_exact(&mut self.payload_buf)?;
        Ok(Some((key, P::read_from(&self.payload_buf))))
    }
}

/// A k-way-merge heap element. `BinaryHeap` is a max-heap, so `Ord` is arranged
/// so the record that should be emitted *next* compares greatest: the smallest
/// key for an ascending merge (the largest when `reverse`), ties broken by the
/// smaller `tiebreak` (`sort` uses the line number for a stable sort; `group`
/// uses the run index, whose order among equal keys is irrelevant since they
/// are folded together).
pub(super) struct HeapEntry<P> {
    pub(super) key: Vec<u8>,
    pub(super) payload: P,
    pub(super) tiebreak: u64,
    pub(super) run: usize,
    pub(super) reverse: bool,
}

impl<P> PartialEq for HeapEntry<P> {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl<P> Eq for HeapEntry<P> {}
impl<P> PartialOrd for HeapEntry<P> {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl<P> Ord for HeapEntry<P> {
    fn cmp(&self, o: &Self) -> Ordering {
        let key_ord = self.key.cmp(&o.key);
        // Ascending: the smaller key must compare "greater" so the max-heap pops
        // it first; `reverse` flips that. All entries in one heap share `reverse`.
        let primary = if self.reverse {
            key_ord
        } else {
            key_ord.reverse()
        };
        // Smaller tiebreak emitted first => it must compare "greater".
        primary.then_with(|| o.tiebreak.cmp(&self.tiebreak))
    }
}

/// Prime a merge heap with the first record of every run. `make` builds the
/// entry from `(key, payload, run_index)`, letting the caller set the payload,
/// tiebreak, and direction for its op.
pub(super) fn seed_heap<P: Payload>(
    readers: &mut [RunReader<P>],
    mut make: impl FnMut(Vec<u8>, P, usize) -> HeapEntry<P>,
) -> Result<BinaryHeap<HeapEntry<P>>> {
    let mut heap = BinaryHeap::with_capacity(readers.len());
    for (i, rr) in readers.iter_mut().enumerate() {
        if let Some((key, payload)) = rr.next_record()? {
            heap.push(make(key, payload, i));
        }
    }
    Ok(heap)
}
