#!/usr/bin/env bash
# Regenerate bevy-patches/: copies of the few Bevy crates whose manifests turn
# on Bevy's browser ("web") code paths for every wasm32 target, with only that
# manifest line removed. Everything else is the crates.io release, unchanged.
#
# Why: bevy_asset, bevy_render and bevy_log list
#   [target.'cfg(target_arch = "wasm32")'.dependencies.bevy_platform]
#   features = ["web"]
# (and the same for bevy_app and bevy_reflect). target_arch = "wasm32" is also
# true for wasm32-wasip1, so a cart would get web-time (performance.now via
# wasm-bindgen) for its clock and wasm-bindgen-futures for its task executor:
# imports no wasmcart host provides. Without the feature, bevy_platform uses
# std::time (WASI clock_time_get) and bevy_tasks uses its local executor.
#
# Usage: scripts/vendor-bevy.sh [version]   (default: the pinned version)
set -euo pipefail
VERSION="${1:-0.20.1}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/bevy-patches"
CRATES=(bevy_asset bevy_render bevy_log)

mkdir -p "$OUT"
for c in "${CRATES[@]}"; do
  dir="$OUT/$c"
  rm -rf "$dir"
  mkdir -p "$dir"
  curl -fsSL "https://static.crates.io/crates/$c/$c-$VERSION.crate" | tar -xz -C "$dir" --strip-components=1
  # Drop `features = ["web"]` from the wasm32-only bevy_app / bevy_platform /
  # bevy_reflect dependency tables, and nothing else.
  awk '
    /^\[target\.\x27cfg\(target_arch = "wasm32"\)\x27\.dependencies\.bevy_(app|platform|reflect)\]$/ { inweb = 1; print; next }
    /^\[/ { inweb = 0 }
    inweb && $0 == "features = [\"web\"]" { next }
    { print }
  ' "$dir/Cargo.toml" > "$dir/Cargo.toml.new"
  mv "$dir/Cargo.toml.new" "$dir/Cargo.toml"
  rm -f "$dir/Cargo.toml.orig" "$dir/.cargo_vcs_info.json" "$dir/Cargo.lock"
  rm -rf "$dir/examples" "$dir/benches" "$dir/tests"
  printf 'Vendored from crates.io %s %s by scripts/vendor-bevy.sh.\nOnly change: no "web" feature on wasm32 bevy_app/bevy_platform/bevy_reflect deps.\n' "$c" "$VERSION" > "$dir/WASMCART-PATCH.txt"
  n=$(grep -c 'features = \["web"\]' "$dir/Cargo.toml" || true)
  echo "$c $VERSION: web features left in manifest: $n"
done
