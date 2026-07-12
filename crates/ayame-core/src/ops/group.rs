use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::document::Document;
use crate::fields::{decoded_text_key_into, field_bytes, FieldSpec};
use crate::Result;

use super::common::SpillGuard;
use super::spill::{self, HeapEntry, Payload, RunReader};

// ======================= group-by (hash aggregation) =========================

/// How to group and aggregate.
#[derive(Clone, Debug)]
pub struct GroupOptions {
    /// 1-based key field; `None` groups on the whole line.
    pub key_column: Option<usize>,
    /// 1-based numeric value field for sum/min/max/avg; `None` = count only.
    pub value_column: Option<usize>,
    /// How to locate key/value fields.
    pub fields: FieldSpec,
    /// Memory budget for the in-memory aggregation map before spilling.
    pub budget_bytes: usize,
    pub spill_dir: PathBuf,
}

impl Default for GroupOptions {
    fn default() -> Self {
        GroupOptions {
            key_column: None,
            value_column: None,
            fields: FieldSpec::default(),
            budget_bytes: super::common::DEFAULT_BUDGET_BYTES,
            spill_dir: std::env::temp_dir().join("ayame-group"),
        }
    }
}

/// One aggregated group emitted by [`group`].
#[derive(Clone, Debug)]
pub struct GroupRow {
    pub key: Vec<u8>,
    pub count: u64,
    /// How many rows had a parseable numeric value (for avg).
    pub numeric_count: u64,
    pub sum: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl GroupRow {
    pub fn avg(&self) -> Option<f64> {
        if self.numeric_count > 0 {
            Some(self.sum / self.numeric_count as f64)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GroupStats {
    pub groups: u64,
    pub runs: usize,
    pub spill_bytes: u64,
}

#[derive(Clone, Copy)]
struct Acc {
    count: u64,
    ncount: u64,
    sum: ExactSum,
    min: f64,
    max: f64,
}

// Every finite f64 is an integer multiple of 2^-1074. Keeping that integer in
// a fixed-width sign/magnitude accumulator makes addition exact and associative,
// so spilling and merging partial groups cannot change the final rounded f64.
const SUM_LIMBS: usize = 34;

#[derive(Clone, Copy)]
struct ExactSum {
    magnitude: [u64; SUM_LIMBS],
    negative: bool,
}

impl ExactSum {
    const fn zero() -> Self {
        Self {
            magnitude: [0; SUM_LIMBS],
            negative: false,
        }
    }

    fn add_f64(&mut self, value: f64) {
        debug_assert!(value.is_finite());
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as usize;
        let mantissa = bits & ((1u64 << 52) - 1);
        let (mantissa, shift) = if exponent == 0 {
            (mantissa, 0)
        } else {
            (mantissa | (1u64 << 52), exponent - 1)
        };
        if mantissa == 0 {
            return;
        }
        let mut term = [0u64; SUM_LIMBS];
        let limb = shift / 64;
        let offset = shift % 64;
        term[limb] = mantissa << offset;
        if offset != 0 {
            term[limb + 1] = mantissa >> (64 - offset);
        }
        self.add_signed(&term, bits >> 63 != 0);
    }

    fn combine(&mut self, other: &Self) {
        self.add_signed(&other.magnitude, other.negative);
    }

    fn add_signed(&mut self, term: &[u64; SUM_LIMBS], negative: bool) {
        if term.iter().all(|&limb| limb == 0) {
            return;
        }
        if self.magnitude.iter().all(|&limb| limb == 0) {
            self.magnitude = *term;
            self.negative = negative;
            return;
        }
        if self.negative == negative {
            add_magnitude(&mut self.magnitude, term);
            return;
        }

        match compare_magnitude(&self.magnitude, term) {
            std::cmp::Ordering::Greater => subtract_magnitude(&mut self.magnitude, term),
            std::cmp::Ordering::Equal => {
                self.magnitude = [0; SUM_LIMBS];
                self.negative = false;
            }
            std::cmp::Ordering::Less => {
                let current = self.magnitude;
                self.magnitude = *term;
                subtract_magnitude(&mut self.magnitude, &current);
                self.negative = negative;
            }
        }
    }

    fn as_f64(self) -> f64 {
        let Some(mut highest) = highest_bit(&self.magnitude) else {
            return 0.0;
        };
        let sign = u64::from(self.negative) << 63;
        if highest < 52 {
            return f64::from_bits(sign | self.magnitude[0]);
        }

        let shift = highest - 52;
        let mut mantissa = shifted_low_u64(&self.magnitude, shift);
        if shift > 0 {
            let halfway = bit_is_set(&self.magnitude, shift - 1);
            let below_halfway = any_bit_below(&self.magnitude, shift - 1);
            if halfway && (below_halfway || mantissa & 1 != 0) {
                mantissa += 1;
                if mantissa == 1u64 << 53 {
                    mantissa >>= 1;
                    highest += 1;
                }
            }
        }
        if highest > 2097 {
            return f64::from_bits(sign | (0x7ffu64 << 52));
        }
        let exponent_bits = (highest - 51) as u64;
        let fraction = mantissa & ((1u64 << 52) - 1);
        f64::from_bits(sign | (exponent_bits << 52) | fraction)
    }
}

fn compare_magnitude(a: &[u64; SUM_LIMBS], b: &[u64; SUM_LIMBS]) -> std::cmp::Ordering {
    a.iter().rev().cmp(b.iter().rev())
}

fn add_magnitude(a: &mut [u64; SUM_LIMBS], b: &[u64; SUM_LIMBS]) {
    let mut carry = false;
    for (left, right) in a.iter_mut().zip(b) {
        let (sum, carry1) = left.overflowing_add(*right);
        let (sum, carry2) = sum.overflowing_add(u64::from(carry));
        *left = sum;
        carry = carry1 || carry2;
    }
    debug_assert!(!carry, "exact f64 accumulator overflow");
}

fn subtract_magnitude(a: &mut [u64; SUM_LIMBS], b: &[u64; SUM_LIMBS]) {
    let mut borrow = false;
    for (left, right) in a.iter_mut().zip(b) {
        let (difference, borrow1) = left.overflowing_sub(*right);
        let (difference, borrow2) = difference.overflowing_sub(u64::from(borrow));
        *left = difference;
        borrow = borrow1 || borrow2;
    }
    debug_assert!(!borrow, "magnitude subtraction underflow");
}

fn highest_bit(value: &[u64; SUM_LIMBS]) -> Option<usize> {
    value
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, &limb)| (limb != 0).then(|| i * 64 + (63 - limb.leading_zeros() as usize)))
}

fn shifted_low_u64(value: &[u64; SUM_LIMBS], shift: usize) -> u64 {
    let limb = shift / 64;
    let offset = shift % 64;
    let mut out = value[limb] >> offset;
    if offset != 0 && limb + 1 < SUM_LIMBS {
        out |= value[limb + 1] << (64 - offset);
    }
    out
}

fn bit_is_set(value: &[u64; SUM_LIMBS], bit: usize) -> bool {
    value[bit / 64] & (1u64 << (bit % 64)) != 0
}

fn any_bit_below(value: &[u64; SUM_LIMBS], bit: usize) -> bool {
    let full_limbs = bit / 64;
    value[..full_limbs].iter().any(|&limb| limb != 0)
        || (!bit.is_multiple_of(64) && value[full_limbs] & ((1u64 << (bit % 64)) - 1) != 0)
}

impl Acc {
    fn new() -> Acc {
        Acc {
            count: 0,
            ncount: 0,
            sum: ExactSum::zero(),
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
    fn add(&mut self, v: Option<f64>) {
        self.count += 1;
        if let Some(x) = v {
            self.ncount += 1;
            self.sum.add_f64(x);
            self.min = self.min.min(x);
            self.max = self.max.max(x);
        }
    }
    fn combine(&mut self, o: &Acc) {
        self.count += o.count;
        self.ncount += o.ncount;
        self.sum.combine(&o.sum);
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
    }
}

impl Payload for Acc {
    const LEN: usize = 305;
    fn write_to(&self, out: &mut impl Write) -> std::io::Result<()> {
        out.write_all(&self.count.to_le_bytes())?;
        out.write_all(&self.ncount.to_le_bytes())?;
        out.write_all(&[u8::from(self.sum.negative)])?;
        for limb in self.sum.magnitude {
            out.write_all(&limb.to_le_bytes())?;
        }
        out.write_all(&self.min.to_le_bytes())?;
        out.write_all(&self.max.to_le_bytes())?;
        Ok(())
    }
    fn read_from(bytes: &[u8]) -> Acc {
        Acc {
            count: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            ncount: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            sum: ExactSum {
                negative: bytes[16] != 0,
                magnitude: std::array::from_fn(|i| {
                    let start = 17 + i * 8;
                    u64::from_le_bytes(bytes[start..start + 8].try_into().unwrap())
                }),
            },
            min: f64::from_le_bytes(bytes[289..297].try_into().unwrap()),
            max: f64::from_le_bytes(bytes[297..305].try_into().unwrap()),
        }
    }
}

/// Group `doc` by the key field and aggregate, calling `emit` once per group in
/// ascending key order. Keeps an in-memory map up to `budget_bytes`; when it
/// overflows it spills a sorted partial-aggregate run and continues, then
/// k-way-merges the runs combining equal keys. The common case (few groups)
/// never touches disk; unbounded cardinality stays within budget.
pub fn group(
    doc: &Document,
    opts: &GroupOptions,
    mut emit: impl FnMut(&GroupRow),
) -> Result<GroupStats> {
    use std::collections::HashMap;
    let spill = SpillGuard::create(&opts.spill_dir)?;
    let spill_dir = spill.path().to_path_buf();
    let enc = doc.encoding();

    let mut map: HashMap<Vec<u8>, Acc> = HashMap::new();
    let mut map_bytes = 0usize;
    let mut runs: Vec<PathBuf> = Vec::new();
    let mut spill_bytes = 0u64;

    let mut field_scratch = Vec::new();
    let mut key_scratch = Vec::new();
    doc.try_for_each_raw_line(
        |_ln, raw| {
            decoded_text_key_into(
                raw,
                enc,
                opts.key_column,
                &opts.fields,
                &mut field_scratch,
                &mut key_scratch,
            );
            let value = opts.value_column.and_then(|c| {
                let f = field_bytes(raw, Some(c), &opts.fields, &mut field_scratch);
                enc.decode_line(f)
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
            });
            match map.get_mut(key_scratch.as_slice()) {
                Some(acc) => acc.add(value),
                None => {
                    map_bytes += key_scratch.len() + std::mem::size_of::<Acc>() + 48;
                    let mut acc = Acc::new();
                    acc.add(value);
                    map.insert(key_scratch.clone(), acc);
                    if map_bytes >= opts.budget_bytes {
                        spill_bytes += spill_group(&mut map, &spill_dir, &mut runs)?;
                        map_bytes = 0;
                    }
                }
            }
            Ok(())
        },
        |_| {},
    )?;

    let mut groups = 0u64;
    if runs.is_empty() {
        // Fast path: everything fit in budget, no disk used. Emit sorted.
        let mut entries: Vec<(Vec<u8>, Acc)> = map.into_iter().collect();
        entries.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
        for (key, acc) in entries {
            emit(&group_row(key, &acc));
            groups += 1;
        }
    } else {
        if !map.is_empty() {
            spill_bytes += spill_group(&mut map, &spill_dir, &mut runs)?;
        }
        groups = merge_groups(&runs, &mut emit)?;
        for r in &runs {
            let _ = fs::remove_file(r);
        }
    }
    spill.finish();

    Ok(GroupStats {
        groups,
        runs: runs.len(),
        spill_bytes,
    })
}

fn group_row(key: Vec<u8>, acc: &Acc) -> GroupRow {
    GroupRow {
        key,
        count: acc.count,
        numeric_count: acc.ncount,
        sum: acc.sum.as_f64(),
        min: (acc.ncount > 0).then_some(acc.min),
        max: (acc.ncount > 0).then_some(acc.max),
    }
}

/// Build the merge-heap element for a group run: the payload is the aggregate,
/// and the tie-break is the run index. Equal keys are folded during the merge,
/// so their order among runs is irrelevant — it only has to be deterministic.
fn group_heap_item(key: Vec<u8>, acc: Acc, run: usize) -> HeapEntry<Acc> {
    HeapEntry {
        key,
        payload: acc,
        tiebreak: run as u64,
        run,
        reverse: false,
    }
}

fn spill_group(
    map: &mut std::collections::HashMap<Vec<u8>, Acc>,
    dir: &Path,
    runs: &mut Vec<PathBuf>,
) -> Result<u64> {
    let mut entries: Vec<(Vec<u8>, Acc)> = map.drain().collect();
    let path = dir.join(format!("grp{:05}.bin", runs.len()));
    let bytes = spill::write_run(&mut entries, |a, b| a.0.cmp(&b.0), &path)?;
    runs.push(path);
    Ok(bytes)
}

/// K-way-merge the spilled runs in a single pass, folding equal keys back
/// together — group runs hold partial aggregates, so any key appearing across
/// runs just needs its accumulators combined.
fn merge_groups(runs: &[PathBuf], emit: &mut impl FnMut(&GroupRow)) -> Result<u64> {
    let mut readers: Vec<RunReader<Acc>> = runs
        .iter()
        .map(|p| RunReader::open(p))
        .collect::<Result<_>>()?;
    let mut heap = spill::seed_heap(&mut readers, group_heap_item)?;

    let mut groups = 0u64;
    while let Some(first) = heap.pop() {
        let key = first.key;
        let mut acc = first.payload;
        if let Some((k, a)) = readers[first.run].next_record()? {
            heap.push(group_heap_item(k, a, first.run));
        }
        // Fold in the same key wherever it appears across the other runs.
        while heap.peek().is_some_and(|t| t.key == key) {
            let it = heap.pop().unwrap();
            acc.combine(&it.payload);
            if let Some((k, a)) = readers[it.run].next_record()? {
                heap.push(group_heap_item(k, a, it.run));
            }
        }
        emit(&group_row(key, &acc));
        groups += 1;
    }
    Ok(groups)
}
