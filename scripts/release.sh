#!/usr/bin/env bash
# Thin convenience wrapper — the release automation itself is plain Rust so it
# works the same on Linux/macOS/Windows: see xtask/src/main.rs.
#
#   scripts/release.sh --bump patch   ==   cargo xtask release --bump patch
exec cargo xtask release "$@"
