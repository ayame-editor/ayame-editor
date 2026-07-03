use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use ayame_core::{Document, Encoding, OpenOptions};

pub(crate) type ParsedArgs = (Vec<String>, HashMap<String, String>, HashSet<String>);
pub(crate) type OpenedDoc = (
    Document,
    Vec<String>,
    HashMap<String, String>,
    HashSet<String>,
);

const OPEN_VALUE_OPTS: &[&str] = &["--encoding", "--stride", "--cache-dir"];
const OPEN_FLAG_OPTS: &[&str] = &["--no-cache"];

/// Split argv into positionals, valued options, and boolean flags.
/// `valued` lists the option names (incl. aliases) that consume the next token.
pub(crate) fn parse_checked(
    args: &[String],
    valued: &[&str],
    flags: &[&str],
) -> Result<ParsedArgs> {
    let mut pos = Vec::new();
    let mut opts = HashMap::new();
    let mut parsed_flags = HashSet::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            pos.extend(args[i + 1..].iter().cloned());
            break;
        }
        if a.starts_with('-') && a != "-" {
            if valued.contains(&a.as_str()) {
                let Some(v) = args.get(i + 1) else {
                    bail!("{a} requires a value");
                };
                opts.insert(a.clone(), v.clone());
                i += 2;
                continue;
            }
            if flags.contains(&a.as_str()) {
                parsed_flags.insert(a.clone());
            } else {
                bail!("unknown option '{a}'");
            }
        } else {
            pos.push(a.clone());
        }
        i += 1;
    }
    Ok((pos, opts, parsed_flags))
}

pub(crate) fn first_opt<'a>(opts: &'a HashMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| opts.get(*k).map(|s| s.as_str()))
}

pub(crate) fn has_flag(flags: &HashSet<String>, keys: &[&str]) -> bool {
    keys.iter().any(|k| flags.contains(*k))
}

pub(crate) fn open_opts(
    opts: &HashMap<String, String>,
    flags: &HashSet<String>,
) -> Result<OpenOptions> {
    let mut o = OpenOptions::default();
    if let Some(enc) = first_opt(opts, &["--encoding"]) {
        o.encoding =
            Some(Encoding::parse(enc).with_context(|| format!("unknown encoding '{enc}'"))?);
    }
    if let Some(s) = first_opt(opts, &["--stride"]) {
        o.stride = Some(s.parse().context("--stride must be a number")?);
    }
    // Index caching is on by default (huge wins on reopen); --no-cache disables.
    o.cache_dir = if has_flag(flags, &["--no-cache"]) {
        None
    } else if let Some(d) = first_opt(opts, &["--cache-dir"]) {
        Some(PathBuf::from(d))
    } else {
        default_cache_dir()
    };
    Ok(o)
}

/// Default index-cache directory, in the OS-natural per-user cache location:
/// `%LOCALAPPDATA%\ayame` on Windows, `~/Library/Caches/ayame` on macOS,
/// `$XDG_CACHE_HOME/ayame` (else `~/.cache/ayame`) elsewhere. `$AYAME_CACHE_DIR`
/// overrides all of it. `None` only when even the home dir is unknown (caching
/// then stays off rather than scattering files next to the executable).
pub(crate) fn default_cache_dir() -> Option<PathBuf> {
    if let Some(d) = non_empty_var_os("AYAME_CACHE_DIR") {
        return Some(PathBuf::from(d));
    }
    os_cache_root().map(|root| root.join("ayame"))
}

fn non_empty_var_os(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

#[cfg(target_os = "windows")]
fn os_cache_root() -> Option<PathBuf> {
    non_empty_var_os("LOCALAPPDATA")
        .or_else(|| non_empty_var_os("APPDATA"))
        .map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn os_cache_root() -> Option<PathBuf> {
    non_empty_var_os("HOME").map(|h| PathBuf::from(h).join("Library").join("Caches"))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn os_cache_root() -> Option<PathBuf> {
    if let Some(d) = non_empty_var_os("XDG_CACHE_HOME") {
        return Some(PathBuf::from(d));
    }
    non_empty_var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))
}

pub(crate) fn open_doc(
    args: &[String],
    valued_extra: &[&str],
    flag_extra: &[&str],
) -> Result<OpenedDoc> {
    let mut valued = Vec::from(OPEN_VALUE_OPTS);
    valued.extend_from_slice(valued_extra);
    let mut allowed_flags = Vec::from(OPEN_FLAG_OPTS);
    allowed_flags.extend_from_slice(flag_extra);
    let (pos, opts, flags) = parse_checked(args, &valued, &allowed_flags)?;
    let path = pos.first().context("expected a FILE argument")?.clone();
    let doc = Document::open(&path, &open_opts(&opts, &flags)?)
        .with_context(|| format!("opening '{path}'"))?;
    Ok((doc, pos, opts, flags))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn checked_parse_rejects_unknown_options() {
        let err = parse_checked(
            &argv(&["file.txt", "--encdoing", "shift_jis"]),
            &["--encoding"],
            &[],
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown option '--encdoing'"));
    }

    #[test]
    fn checked_parse_rejects_missing_values() {
        let err =
            parse_checked(&argv(&["file.txt", "--encoding"]), &["--encoding"], &[]).unwrap_err();

        assert!(err.to_string().contains("--encoding requires a value"));
    }

    #[test]
    fn checked_parse_keeps_dash_dash_positionals_unchecked() {
        let (pos, opts, flags) = parse_checked(
            &argv(&["file.txt", "--json", "--", "--literal-pattern"]),
            &[],
            &["--json"],
        )
        .unwrap();

        assert_eq!(pos, vec!["file.txt", "--literal-pattern"]);
        assert!(opts.is_empty());
        assert!(flags.contains("--json"));
    }
}
