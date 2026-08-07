use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use ayame_core::{FieldSpec, DEFAULT_BUDGET_BYTES};

use super::args::{first_opt, has_flag};

pub(crate) fn parse_key(opts: &HashMap<String, String>) -> Result<Option<usize>> {
    match first_opt(opts, &["--key", "-k"]) {
        Some(s) => {
            let key: usize = s.parse().context("--key must be a number")?;
            // 1-based, like `parse_keys`. Without this, `-k 0` slipped through and
            // silently degraded to a whole-line key instead of erroring (#105).
            anyhow::ensure!(key > 0, "--key columns are 1-based and must be at least 1");
            Ok(Some(key))
        }
        None => Ok(None),
    }
}

pub(crate) fn parse_keys(opts: &HashMap<String, String>) -> Result<Vec<usize>> {
    let Some(raw) = first_opt(opts, &["--key", "-k"]) else {
        return Ok(Vec::new());
    };
    raw.split([',', ';'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let key = part
                .parse::<usize>()
                .with_context(|| format!("--key contains an invalid column '{part}'"))?;
            anyhow::ensure!(key > 0, "--key columns are 1-based and must be at least 1");
            Ok(key)
        })
        .collect()
}

/// `--budget` memory bound for the spill-to-disk ops. The default mirrors core's
/// [`DEFAULT_BUDGET_BYTES`] (256 MiB) so the CLI never drifts from the op
/// defaults used by `SortOptions`/`GroupOptions` (#105).
pub(crate) fn parse_budget(opts: &HashMap<String, String>) -> Result<usize> {
    match first_opt(opts, &["--budget"]) {
        Some(s) => parse_size(s),
        None => Ok(DEFAULT_BUDGET_BYTES),
    }
}

pub(crate) fn field_spec(opts: &HashMap<String, String>, flags: &HashSet<String>) -> FieldSpec {
    let delimiter = first_opt(opts, &["--delim", "-t"])
        .and_then(|s| match s {
            "\\t" | "tab" | "TAB" => Some(b'\t'),
            _ => s.as_bytes().first().copied(),
        })
        .unwrap_or(b',');
    let quote = first_opt(opts, &["--quote"])
        .and_then(|s| s.as_bytes().first().copied())
        .unwrap_or(b'"');
    FieldSpec {
        delimiter,
        quote,
        csv: has_flag(flags, &["--csv"]),
    }
}

/// Parse a byte size with an optional binary suffix (K/KiB, M/MiB, G/GiB).
pub(crate) fn parse_size(s: &str) -> Result<usize> {
    let lower = s.trim().to_ascii_lowercase();
    let (num, mult): (&str, usize) = if let Some(n) = lower
        .strip_suffix("gib")
        .or_else(|| lower.strip_suffix('g'))
    {
        (n, 1 << 30)
    } else if let Some(n) = lower
        .strip_suffix("mib")
        .or_else(|| lower.strip_suffix('m'))
    {
        (n, 1 << 20)
    } else if let Some(n) = lower
        .strip_suffix("kib")
        .or_else(|| lower.strip_suffix('k'))
    {
        (n, 1 << 10)
    } else if let Some(n) = lower.strip_suffix('b') {
        (n, 1)
    } else {
        (lower.as_str(), 1)
    };
    let val: f64 = num
        .trim()
        .parse()
        .with_context(|| format!("invalid size '{s}'"))?;
    Ok((val * mult as f64) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--budget` is how a user bounds a sort's memory before it spills, so a
    /// misparse is a silent out-of-memory or a needless spill. Untested until
    /// now (#113).
    #[test]
    fn parses_sizes_with_and_without_units() {
        assert_eq!(parse_size("512").unwrap(), 512);
        assert_eq!(parse_size("512b").unwrap(), 512);
        assert_eq!(parse_size("2k").unwrap(), 2 << 10);
        assert_eq!(parse_size("2KiB").unwrap(), 2 << 10);
        assert_eq!(parse_size("256m").unwrap(), 256 << 20);
        assert_eq!(parse_size("256MiB").unwrap(), 256 << 20);
        assert_eq!(parse_size("2g").unwrap(), 2 << 30);
        assert_eq!(parse_size("2GiB").unwrap(), 2usize << 30);
        // Units are case-insensitive and surrounding space is ignored, because
        // both come from a shell where quoting is easy to get slightly wrong.
        assert_eq!(parse_size(" 1gib ").unwrap(), 1 << 30);
        // Fractions are accepted: "1.5GiB" is a natural thing to type.
        assert_eq!(parse_size("1.5MiB").unwrap(), 1_572_864);
    }

    #[test]
    fn rejects_sizes_that_are_not_numbers() {
        for bad in ["", "abc", "12x", "m", "1.2.3"] {
            let err = parse_size(bad).unwrap_err().to_string();
            assert!(err.contains("invalid size"), "{bad:?} -> {err}");
        }
    }

    #[test]
    fn parses_ordered_key_columns_and_tsv_delimiter_aliases() {
        let opts = HashMap::from([
            ("--key".to_string(), "3,1,2".to_string()),
            ("--delim".to_string(), "\\t".to_string()),
        ]);
        assert_eq!(parse_keys(&opts).unwrap(), vec![3, 1, 2]);
        assert_eq!(field_spec(&opts, &HashSet::new()).delimiter, b'\t');
    }

    #[test]
    fn parse_key_rejects_zero_and_matches_parse_keys_validation() {
        let zero = HashMap::from([("--key".to_string(), "0".to_string())]);
        assert!(parse_key(&zero).is_err(), "-k 0 must be rejected");
        // parse_keys already rejected 0; the single-key path now agrees.
        assert!(parse_keys(&zero).is_err());

        assert_eq!(
            parse_key(&HashMap::from([("--key".to_string(), "2".to_string())])).unwrap(),
            Some(2)
        );
        assert_eq!(parse_key(&HashMap::new()).unwrap(), None);
    }

    #[test]
    fn budget_default_tracks_the_core_constant() {
        assert_eq!(parse_budget(&HashMap::new()).unwrap(), DEFAULT_BUDGET_BYTES);
        // And an explicit --budget goes through parse_size.
        let opts = HashMap::from([("--budget".to_string(), "8MiB".to_string())]);
        assert_eq!(parse_budget(&opts).unwrap(), 8 << 20);
    }
}
