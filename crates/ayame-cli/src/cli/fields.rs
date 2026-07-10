use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use ayame_core::FieldSpec;

use super::args::{first_opt, has_flag};

pub(crate) fn parse_key(opts: &HashMap<String, String>) -> Result<Option<usize>> {
    match first_opt(opts, &["--key", "-k"]) {
        Some(s) => Ok(Some(s.parse().context("--key must be a number")?)),
        None => Ok(None),
    }
}

pub(crate) fn parse_keys(opts: &HashMap<String, String>) -> Result<Vec<usize>> {
    let Some(raw) = first_opt(opts, &["--key", "-k"]) else {
        return Ok(Vec::new());
    };
    raw.split(|ch| matches!(ch, ',' | ';'))
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

/// `--budget` memory bound for the spill-to-disk ops (default 256 MiB).
pub(crate) fn parse_budget(opts: &HashMap<String, String>) -> Result<usize> {
    match first_opt(opts, &["--budget"]) {
        Some(s) => parse_size(s),
        None => Ok(256 * 1024 * 1024),
    }
}

pub(crate) fn field_spec(opts: &HashMap<String, String>, flags: &HashSet<String>) -> FieldSpec {
    let delimiter = first_opt(opts, &["--delim", "-t"])
        .and_then(|s| match s.as_str() {
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

    #[test]
    fn parses_ordered_key_columns_and_tsv_delimiter_aliases() {
        let opts = HashMap::from([
            ("--key".to_string(), "3,1,2".to_string()),
            ("--delim".to_string(), "\\t".to_string()),
        ]);
        assert_eq!(parse_keys(&opts).unwrap(), vec![3, 1, 2]);
        assert_eq!(field_spec(&opts, &HashSet::new()).delimiter, b'\t');
    }
}
