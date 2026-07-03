#!/usr/bin/env bash
# One-command release: runs the full gate from docs/RELEASE.md, builds and
# smoke-tests the local artifact, then tags v<version> and pushes so the
# GitHub Release workflow publishes all platforms.
#
# Usage:
#   scripts/release.sh                     release the version already in Cargo.toml
#   scripts/release.sh --bump patch        bump 0.3.0 -> 0.3.1, commit "release: v0.3.1", then release
#   scripts/release.sh --bump minor|major  likewise
#   scripts/release.sh --bump 1.2.3        set an explicit version
# Options:
#   --yes         no confirmation prompt (CI / non-interactive use)
#   --dry-run     run every check but skip commit/tag/push
#   --skip-gate   skip fmt/clippy/test/build (only for re-runs after a green gate)
set -euo pipefail
cd "$(dirname "$0")/.."

bump=""
yes=0
dry=0
skip_gate=0
while [ $# -gt 0 ]; do
  case "$1" in
    --bump) bump="${2:?--bump needs an argument}"; shift 2 ;;
    --yes) yes=1; shift ;;
    --dry-run) dry=1; shift ;;
    --skip-gate) skip_gate=1; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

say()  { printf '\033[1;35m== %s\033[0m\n' "$*"; }
fail() { printf '\033[1;31mrelease: %s\033[0m\n' "$*" >&2; exit 1; }

# ---- repo preflight ---------------------------------------------------------
say "preflight"
branch="$(git branch --show-current)"
[ "$branch" = "main" ] || { [ "$yes" = 1 ] || fail "not on main (on '$branch'); pass --yes to release from a branch"; }
git diff --quiet && git diff --cached --quiet || fail "working tree is not clean — commit or stash first"
git fetch origin --tags --quiet
upstream="origin/$branch"
if git rev-parse --verify -q "$upstream" >/dev/null; then
  behind="$(git rev-list --count "HEAD..$upstream")"
  [ "$behind" = 0 ] || fail "HEAD is $behind commit(s) behind $upstream — pull first"
fi

# ---- version (and optional bump) -------------------------------------------
current="$(cargo pkgid -p ayame-cli | sed 's/.*#//')"
version="$current"
if [ -n "$bump" ]; then
  IFS=. read -r maj min pat <<EOF
$current
EOF
  case "$bump" in
    patch) version="$maj.$min.$((pat + 1))" ;;
    minor) version="$maj.$((min + 1)).0" ;;
    major) version="$((maj + 1)).0.0" ;;
    *.*.*) version="$bump" ;;
    *) fail "--bump must be patch, minor, major, or X.Y.Z" ;;
  esac
  say "bump $current -> $version"
  sed -i.bak "s/^version = \"$current\"/version = \"$version\"/" Cargo.toml && rm -f Cargo.toml.bak
  grep -q "^version = \"$version\"" Cargo.toml || fail "failed to bump the workspace version in Cargo.toml"
  cargo build --quiet 2>/dev/null || cargo build   # refresh Cargo.lock
  if [ "$dry" = 1 ]; then
    say "dry-run: leaving the bump uncommitted"
  else
    git add Cargo.toml Cargo.lock
    git commit -m "release: v$version"
  fi
fi
tag="v$version"
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && fail "tag $tag already exists — bump the version first (--bump patch)"

# ---- gate (docs/RELEASE.md "Local Gate") ------------------------------------
if [ "$skip_gate" = 0 ]; then
  say "gate: fmt / clippy / test"
  cargo fmt --all --check
  cargo clippy --all-targets --locked -- -D warnings
  cargo clippy --all-targets --locked --features gui -- -D warnings
  cargo test --locked
  say "gate: release builds"
  cargo build --release --locked
  cargo build --release --locked --features gui
  if [ -x scripts/crash-isolation-test.sh ]; then
    say "gate: crash isolation"
    scripts/crash-isolation-test.sh
  fi
fi

# ---- local artifact + smoke (docs/RELEASE.md "Local Artifact") --------------
say "artifact: scripts/release-local.sh"
scripts/release-local.sh
target="$(rustc -Vv | awk '/^host:/ { print $2 }')"
ext=""; case "$target" in *windows*) ext=".exe" ;; esac
bin="dist/ayame-v${version}-${target}${ext}"
[ -x "$bin" ] || fail "expected artifact $bin was not produced"
got="$("$bin" --version)"
[ "$got" = "ayame $version" ] || fail "artifact reports '$got', expected 'ayame $version'"

say "smoke: CLI on a temp file"
smoke="$(mktemp -d)"
trap 'rm -rf "$smoke"' EXIT
"$bin" gen "$smoke/s.csv" --lines 1000 >/dev/null
"$bin" stat "$smoke/s.csv" >/dev/null
"$bin" search "$smoke/s.csv" "row" --max 3 >/dev/null
"$bin" sort "$smoke/s.csv" --out "$smoke/sorted.csv" >/dev/null
"$bin" split "$smoke/s.csv" --lines 400 --out-dir "$smoke" >/dev/null
[ -f "$smoke/sorted.csv" ] || fail "sort smoke produced no output"
ls "$smoke"/s.part*.csv >/dev/null 2>&1 || fail "split smoke produced no parts"
say "smoke: OK ($(basename "$bin"))"

# ---- confirm + tag + push ----------------------------------------------------
say "release summary"
echo "  version : $version  (tag $tag)"
echo "  branch  : $branch @ $(git rev-parse --short HEAD)"
echo "  head    : $(git log -1 --format=%s)"
if [ "$dry" = 1 ]; then
  say "dry-run: stopping before tag/push"
  exit 0
fi
if [ "$yes" = 0 ]; then
  printf 'Tag and push %s now? [y/N] ' "$tag"
  read -r answer
  [ "$answer" = y ] || [ "$answer" = Y ] || fail "aborted"
fi

git tag "$tag"
git push origin "$branch" "$tag"

# ---- watch the workflow -------------------------------------------------------
if command -v gh >/dev/null 2>&1; then
  say "waiting for the Release workflow"
  sleep 10
  run_id="$(gh run list --workflow=Release --limit 1 --json databaseId --jq '.[0].databaseId')"
  gh run watch "$run_id" --exit-status
  say "published"
  gh release view "$tag" | sed -n '1,20p'
else
  say "gh not found — watch https://github.com/hjosugi/ayame-editor/actions manually"
fi

say "reminder: manual platform checks (docs/RELEASE.md)"
cat <<'EOF'
  - ayame            -> native window opens without a file
  - ayame <FILE>     -> opens the file natively
  - --encoding Shift_JIS search / worker smoke
  - dirty save / save-as / close-with-unsaved-confirm
  - macOS/Windows: menu, shortcuts, icon, drag&drop (issue #3 checklist)
EOF
