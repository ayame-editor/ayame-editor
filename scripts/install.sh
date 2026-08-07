#!/bin/sh
set -eu

repo="ayame-editor/ayame-editor"
base_url="https://github.com/$repo"
version="${AYAME_VERSION:-${VERSION:-latest}}"
install_dir="${AYAME_INSTALL_DIR:-${INSTALL_DIR:-}}"

say() {
  printf '%s\n' "$*"
}

die() {
  printf 'ayame install: %s\n' "$*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

resolve_version() {
  if [ "$version" = "latest" ]; then
    effective="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$base_url/releases/latest")"
    tag="${effective##*/}"
    case "$tag" in
      v*) version="${tag#v}" ;;
      *) die "could not resolve latest release from $effective" ;;
    esac
  else
    version="${version#v}"
  fi
}

detect_target() {
  os="$(uname -s)"
  arch="$(uname -m)"
  exe_name="ayame"
  kind="bin" # "bin" (Linux/Windows binary) or "app" (macOS .app bundle)

  case "$os:$arch" in
    Linux:x86_64|Linux:amd64)
      target="linux-x86_64"
      ;;
    Darwin:arm64|Darwin:aarch64)
      target="macos-aarch64.zip"
      kind="app"
      ;;
    Darwin:x86_64)
      target="macos-x86_64.zip"
      kind="app"
      ;;
    MINGW*:x86_64|MSYS*:x86_64|CYGWIN*:x86_64)
      target="windows-x86_64.exe"
      exe_name="ayame.exe"
      ;;
    *)
      die "unsupported platform: $os $arch"
      ;;
  esac
}

default_install_dir() {
  if [ -n "$install_dir" ]; then
    return
  fi
  case "$kind:$target" in
    app:*) install_dir="$HOME/Applications" ;;
    *:windows-*) install_dir="$HOME/bin" ;;
    *) install_dir="$HOME/.local/bin" ;;
  esac
}

download() {
  tmp="${TMPDIR:-/tmp}/ayame-install.$$"
  mkdir -p "$tmp"
  trap 'rm -rf "$tmp"' EXIT INT TERM

  asset="ayame-v${version}-${target}"
  url="$base_url/releases/download/v${version}/$asset"
  sum_url="$url.sha256"

  say "download: $url"
  curl -fL --retry 3 --retry-delay 1 "$url" -o "$tmp/$asset"
  curl -fL --retry 3 --retry-delay 1 "$sum_url" -o "$tmp/$asset.sha256"
}

verify_sha256() {
  say "verify: $asset.sha256"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$tmp" && sha256sum -c "$asset.sha256")
  elif command -v shasum >/dev/null 2>&1; then
    (cd "$tmp" && shasum -a 256 -c "$asset.sha256")
  else
    die "sha256sum or shasum is required for checksum verification"
  fi
}

install_artifact() {
  mkdir -p "$install_dir"

  if [ "$kind" = "app" ]; then
    need unzip
    rm -rf "$tmp/extract"
    mkdir -p "$tmp/extract"
    (cd "$tmp/extract" && unzip -oq "$tmp/$asset")
    app="$(cd "$tmp/extract" && ls -d ./*.app 2>/dev/null | head -n1 | sed 's|^\./||')"
    [ -n "$app" ] || die "no .app bundle found inside $asset"
    dest="$install_dir/$app"
    rm -rf "$dest"
    cp -R "$tmp/extract/$app" "$dest"
    xattr -dr com.apple.quarantine "$dest" 2>/dev/null || true
    return
  fi

  dest="$install_dir/$exe_name"
  if command -v install >/dev/null 2>&1; then
    install -m 0755 "$tmp/$asset" "$dest"
  else
    cp "$tmp/$asset" "$dest"
    chmod 0755 "$dest"
  fi

  if [ "$(uname -s)" = "Darwin" ]; then
    xattr -d com.apple.quarantine "$dest" 2>/dev/null || true
  fi
}

path_line() {
  case "$install_dir" in
    "$HOME"/*)
      rel="\$HOME/${install_dir#"$HOME"/}"
      printf 'export PATH="%s:$PATH"\n' "$rel"
      ;;
    *)
      printf 'export PATH="%s:$PATH"\n' "$install_dir"
      ;;
  esac
}

profile_file() {
  shell_name="${SHELL:-}"
  case "$(uname -s):$shell_name" in
    Darwin:*zsh*) printf '%s\n' "$HOME/.zprofile" ;;
    *:*zsh*) printf '%s\n' "$HOME/.zshrc" ;;
    *:*bash*) printf '%s\n' "$HOME/.bashrc" ;;
    *) printf '%s\n' "$HOME/.profile" ;;
  esac
}

ensure_path() {
  case ":$PATH:" in
    *":$install_dir:"*) return ;;
  esac

  line="$(path_line)"
  profile="$(profile_file)"
  mkdir -p "$HOME"
  touch "$profile"
  if ! grep -qxF "$line" "$profile"; then
    {
      printf '\n'
      printf '# Ayame\n'
      printf '%s\n' "$line"
    } >> "$profile"
  fi

  PATH="$install_dir:$PATH"
  export PATH
  say "PATH updated for this install. New shells will read: $profile"
}

main() {
  : "${HOME:?HOME is required}"
  need curl
  resolve_version
  detect_target
  default_install_dir
  download
  verify_sha256
  install_artifact

  # macOS: an .app bundle you launch from Finder, not a PATH binary.
  if [ "$kind" = "app" ]; then
    say "installed: $dest"
    say "Launch Ayame from Finder, or run: open \"$dest\""
    return
  fi

  ensure_path
  say "installed: $dest"
  if [ "$(uname -s)" = "Linux" ]; then
    say "note: the Linux app needs WebKitGTK at runtime (e.g. Debian/Ubuntu: libwebkit2gtk-4.1-0)."
  fi
  "$dest" --version
  if command -v "$exe_name" >/dev/null 2>&1; then
    "$exe_name" --version >/dev/null
  else
    say "open a new shell, or run: $(path_line)"
  fi
}

main "$@"
