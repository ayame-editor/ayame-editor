#!/usr/bin/env bash
set -euo pipefail

target="${1:-}"
if [ -z "$target" ]; then
  target="$(rustc -Vv | awk '/^host:/ { print $2 }')"
fi

version="$(cargo pkgid -p ayame-cli | sed 's/.*#//')"
ext=""
case "$target" in
  *windows*) ext=".exe" ;;
esac
case "$target" in
  *windows-msvc*) export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-feature=+crt-static" ;;
esac

cargo build --release --locked --features gui --target "$target"

target_dir="$(cargo metadata --format-version=1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
mkdir -p dist
src="$target_dir/$target/release/ayame$ext"
if [ ! -f "$src" ] && [ "$target" = "$(rustc -Vv | awk '/^host:/ { print $2 }')" ]; then
  src="$target_dir/release/ayame$ext"
fi
out="dist/ayame-v${version}-${target}${ext}"
cp "$src" "$out"
strip "$out" 2>/dev/null || true

out_dir="$(dirname "$out")"
out_name="$(basename "$out")"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$out_dir" && sha256sum "$out_name") > "$out.sha256"
else
  (cd "$out_dir" && shasum -a 256 "$out_name") > "$out.sha256"
fi

printf 'built %s\n' "$out"
printf 'checksum %s\n' "$out.sha256"
