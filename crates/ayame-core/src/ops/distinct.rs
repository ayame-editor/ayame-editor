use crate::document::Document;
use crate::fields::{field_bytes, FieldSpec};

// ===================== DISTINCT (HyperLogLog) ================================

/// Options for [`distinct`].
#[derive(Clone, Debug)]
pub struct DistinctOptions {
    pub key_column: Option<usize>,
    pub fields: FieldSpec,
    /// HLL precision `p` (registers = 2^p). Clamped to [4, 18]. 14 ≈ 0.8% error.
    pub precision: u32,
}

impl Default for DistinctOptions {
    fn default() -> Self {
        DistinctOptions {
            key_column: None,
            fields: FieldSpec::default(),
            precision: 14,
        }
    }
}

/// Approximate distinct-value count of a field.
#[derive(Clone, Copy, Debug)]
pub struct DistinctResult {
    pub estimate: u64,
    pub registers: usize,
    pub memory_bytes: usize,
}

/// HyperLogLog: estimate cardinality in fixed memory (2^p bytes), independent of
/// how many distinct values there are.
struct Hll {
    reg: Vec<u8>,
    p: u32,
}

impl Hll {
    fn new(p: u32) -> Hll {
        Hll {
            reg: vec![0u8; 1usize << p],
            p,
        }
    }
    fn add(&mut self, h: u64) {
        let idx = (h >> (64 - self.p)) as usize; // top p bits select the register
                                                 // Remaining bits shifted to the top; a guard bit bounds rho.
        let w = (h << self.p) | (1u64 << (self.p - 1));
        let rho = w.leading_zeros() as u8 + 1;
        if rho > self.reg[idx] {
            self.reg[idx] = rho;
        }
    }
    fn estimate(&self) -> f64 {
        let m = self.reg.len() as f64;
        let alpha = match self.reg.len() {
            16 => 0.673,
            32 => 0.697,
            64 => 0.709,
            _ => 0.7213 / (1.0 + 1.079 / m),
        };
        let sum: f64 = self.reg.iter().map(|&r| 2f64.powi(-(r as i32))).sum();
        let raw = alpha * m * m / sum;
        if raw <= 2.5 * m {
            // Small-range correction: linear counting over empty registers.
            let zeros = self.reg.iter().filter(|&&r| r == 0).count() as f64;
            if zeros > 0.0 {
                return m * (m / zeros).ln();
            }
        }
        raw // 64-bit hash => no large-range (2^32) correction needed
    }
}

/// Estimate the number of distinct values of the configured field.
pub fn distinct(doc: &Document, opts: &DistinctOptions) -> DistinctResult {
    use std::hash::{Hash, Hasher};
    let mut hll = Hll::new(opts.precision.clamp(4, 18));
    let mut scratch = Vec::new();
    // The scan cannot fail (the closure is infallible), so the `Result` is Ok.
    doc.for_each_raw_line(|_ln, raw| {
        // Distinctness is over the (unescaped) field bytes; identical bytes
        // hash identically, so no decode is needed here.
        let field = field_bytes(raw, opts.key_column, &opts.fields, &mut scratch);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        field.hash(&mut h);
        hll.add(h.finish());
        Ok(())
    })
    .expect("distinct scan is infallible");
    DistinctResult {
        estimate: hll.estimate().round() as u64,
        registers: hll.reg.len(),
        memory_bytes: hll.reg.len(),
    }
}
