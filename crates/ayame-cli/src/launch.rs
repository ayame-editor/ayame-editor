//! Typed editor launch targets shared by CLI dispatch, the native shell and
//! the server-side caret resolver (#248).
//!
//! User-facing coordinates are deliberately kept 1-based until the document
//! is open.  The server is the single place that converts them to the editor's
//! 0-based logical coordinates, because only it has the authoritative line
//! count and decoded text.

#![cfg_attr(not(feature = "gui"), allow(dead_code))]

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const MAX_SAFE_JS_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct LaunchPosition {
    /// 1-based line number, or -1 for the final logical line.
    pub(crate) line: i64,
    /// 1-based Unicode-scalar column.
    pub(crate) column: u64,
}

impl LaunchPosition {
    pub(crate) fn checked(line: i64, column: u64) -> Result<Self> {
        if line != -1 && line <= 0 {
            bail!("line must be -1 or a positive integer");
        }
        if line > MAX_SAFE_JS_INTEGER as i64 {
            bail!("line is too large to represent exactly in the editor");
        }
        if column == 0 || column > MAX_SAFE_JS_INTEGER {
            bail!("column must be a positive exactly representable integer");
        }
        Ok(Self { line, column })
    }

    pub(crate) fn parse(line: Option<&str>, column: Option<&str>) -> Result<Option<Self>> {
        if line.is_none() && column.is_none() {
            return Ok(None);
        }
        let line = match line {
            Some(value) => parse_line(value)?,
            None => 1,
        };
        let column = match column {
            Some(value) => parse_column(value)?,
            None => 1,
        };
        Ok(Some(Self::checked(line, column)?))
    }

    /// Resolve the special EOF line and clamp a normal 1-based line against
    /// the open document. The column is clamped separately after decoding the
    /// selected line.
    pub(crate) fn zero_based_line(self, total_lines: u64) -> u64 {
        let last = total_lines.saturating_sub(1);
        if self.line == -1 {
            last
        } else {
            (self.line as u64).saturating_sub(1).min(last)
        }
    }

    pub(crate) fn zero_based_column(self) -> u64 {
        self.column.saturating_sub(1)
    }
}

pub(crate) fn from_options(opts: &HashMap<String, String>) -> Result<Option<LaunchPosition>> {
    LaunchPosition::parse(
        opts.get("--line").map(String::as_str),
        opts.get("--column").map(String::as_str),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PathPosition {
    pub(crate) path: String,
    pub(crate) position: LaunchPosition,
}

/// Parse `path:line[:column]` from the right. Numeric suffixes are the only
/// separators that count, so Windows drive prefixes (`C:\\`), UNC paths,
/// spaces, colons inside Unix names and non-ASCII paths remain intact.
pub(crate) fn parse_path_position(value: &str) -> Option<PathPosition> {
    let (before_last, last) = value.rsplit_once(':')?;
    let (path, line, column) = match before_last.rsplit_once(':') {
        Some((path, line)) if parse_line(line).is_ok() && parse_positive(last).is_ok() => {
            (path, parse_line(line).ok()?, parse_positive(last).ok()?)
        }
        _ => (before_last, parse_line(last).ok()?, 1),
    };
    if path.is_empty() || line == 0 {
        return None;
    }
    Some(PathPosition {
        path: path.to_string(),
        position: LaunchPosition::checked(line, column).ok()?,
    })
}

/// Expand an editor-style suffix only when the literal argv path does not
/// exist and the suffix-free path does. This preserves real files whose names
/// happen to end in `:123` on filesystems that allow colons.
pub(crate) fn existing_path_position(value: &str) -> Option<PathPosition> {
    if Path::new(value).exists() {
        return None;
    }
    parse_path_position(value).filter(|target| Path::new(&target.path).exists())
}

fn parse_line(value: &str) -> Result<i64> {
    let line: i64 = value
        .trim()
        .parse()
        .with_context(|| format!("--line must be -1 or a positive integer, got '{value}'"))?;
    if line == -1 || line > 0 {
        Ok(line)
    } else {
        bail!("--line must be -1 or a positive integer, got '{value}'")
    }
}

fn parse_column(value: &str) -> Result<u64> {
    parse_positive(value)
        .with_context(|| format!("--column must be a positive integer, got '{value}'"))
}

fn parse_positive(value: &str) -> Result<u64> {
    let parsed: u64 = value.trim().parse()?;
    if parsed == 0 {
        bail!("value must be positive");
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_coordinates_validate_and_resolve_once() {
        let position = LaunchPosition::parse(Some("-1"), Some("67"))
            .unwrap()
            .unwrap();
        assert_eq!(position.zero_based_line(10_000_000_000), 9_999_999_999);
        assert_eq!(position.zero_based_column(), 66);

        let position = LaunchPosition::parse(Some("12345"), None).unwrap().unwrap();
        assert_eq!(position.zero_based_line(20_000), 12_344);
        assert_eq!(position.zero_based_column(), 0);
        assert!(LaunchPosition::parse(Some("0"), None).is_err());
        assert!(LaunchPosition::parse(Some("-2"), None).is_err());
        assert!(LaunchPosition::parse(None, Some("0")).is_err());
    }

    #[test]
    fn path_suffix_parser_preserves_cross_platform_paths() {
        let cases = [
            ("/var/log/app.log:123:67", "/var/log/app.log", 123, 67),
            (r"C:\logs\app.log:123:67", r"C:\logs\app.log", 123, 67),
            (r"\\server\share\app.log:9", r"\\server\share\app.log", 9, 1),
            ("/tmp/a:b/日本語.log:42", "/tmp/a:b/日本語.log", 42, 1),
            ("/tmp/app.log:-1", "/tmp/app.log", -1, 1),
        ];
        for (input, path, line, column) in cases {
            let parsed = parse_path_position(input).unwrap();
            assert_eq!(parsed.path, path);
            assert_eq!(parsed.position, LaunchPosition { line, column });
        }
    }

    #[test]
    fn path_suffix_parser_rejects_ambiguous_or_invalid_suffixes() {
        assert!(parse_path_position(r"C:\logs\app.log").is_none());
        assert!(parse_path_position("/tmp/app.log:0").is_none());
        assert!(parse_path_position("/tmp/app.log:4:0").is_none());
        assert!(parse_path_position("https://example.test/a").is_none());
    }
}
