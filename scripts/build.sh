#!/usr/bin/env bash
# Build example carts into dist/<name>.wasc.
#   scripts/build.sh                 # every example
#   scripts/build.sh star-catcher    # one
# Needs: rustup (rust-toolchain.toml pins stable + wasm32-wasip1) and node.
# wasmcart-pack comes from npx (the wasmcart package), or from a wasmcart
# checkout named by WASMCART_DIR.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
mkdir -p dist

pack() {
  if [[ -n "${WASMCART_DIR:-}" ]]; then
    node "$WASMCART_DIR/bin/wasmcart-pack.js" "$@"
  else
    npx --yes -p wasmcart wasmcart-pack "$@"
  fi
}

# name width height controls title description [package features]
CARTS=(
  "hello-ecs|320|240|dpad,left_stick,a,b|Hello ECS|Bevy's ECS with no renderer: pad-steered squares in the 2D framebuffer, with sound."
  "star-catcher|640|360|dpad,left_stick,a,start|Star Catcher|Fly a ship, catch falling stars, dodge rocks. A 2D Bevy game on WebGPU."
  "lit-scene|960|540|dpad,left_stick,right_stick,a,b,l,r,left_trigger,right_trigger|Lit Scene|Bevy's PBR renderer: shadowed sun, coloured point lights, metal and plastic, a pad-orbited camera."
  "lit-scene-core|960|540|dpad,left_stick,right_stick,a,b,l,r,left_trigger,right_trigger|Lit Scene (core)|Lit Scene built with CartConfig::prefer_core: asks for a core WebGPU device where the GPU has one.|lit-scene|prefer-core"
)

want="${1:-}"
for entry in "${CARTS[@]}"; do
  IFS='|' read -r name w h controls title desc pkg features <<<"$entry"
  [[ -n "$want" && "$want" != "$name" ]] && continue
  pkg="${pkg:-$name}"
  cargo build --release -p "$pkg" ${features:+--features "$features"}
  wasm="target/wasm32-wasip1/release/${pkg//-/_}.wasm"
  node scripts/wasm-imports.mjs "$wasm" >/dev/null
  args=(--wasm "$wasm" --name "$name" --title "$title" --description "$desc"
        --version "$(cargo metadata --no-deps --format-version 1 | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).packages[0].version))')"
        --width "$w" --height "$h" --controls "$controls" --output "dist/$name.wasc")
  [[ -d "examples/$pkg/assets" ]] && args+=(--assets "examples/$pkg/assets")
  pack "${args[@]}" | tail -1
  echo "built dist/$name.wasc"
done
