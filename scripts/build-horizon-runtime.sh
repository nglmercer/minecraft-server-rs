#!/usr/bin/env bash
# Build and stage the pinned, headless Horizon runtime for a release package.
set -euo pipefail

if [[ $# != 2 ]]; then
  echo "Usage: $0 TARGET OUTPUT_DIRECTORY" >&2
  exit 2
fi
horizon_target="$1"
horizon_output="$2"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
source_dir="$repo_root/vendor/horizon"
expected="$(git -C "$repo_root" rev-parse HEAD:vendor/horizon)"
actual="$(git -C "$source_dir" rev-parse HEAD)"
if [[ "$actual" != "$expected" ]] || [[ -n "$(git -C "$source_dir" status --porcelain --untracked-files=no)" ]]; then
  echo "The Horizon submodule must be clean and match the pinned revision." >&2
  exit 1
fi
if [[ -e "$horizon_output" ]]; then
  echo "Refusing to replace $horizon_output" >&2
  exit 1
fi

# The headless client is the engine; keep defaults disabled just as Horizon's
# own dedicated-server build does. Android needs the NDK toolchain wrapper.
if [[ "$horizon_target" == aarch64-linux-android && -n "${ANDROID_NDK_HOME:-${ANDROID_NDK_LATEST_HOME:-}}" ]]; then
  cargo_cmd=("$repo_root/scripts/cargo-android.sh")
  target_args=()
else
  cargo_cmd=(cargo)
  target_args=(--target "$horizon_target")
fi
manifest="$source_dir/Cargo.toml"
"${cargo_cmd[@]}" build --manifest-path "$manifest" --release --locked "${target_args[@]}" -p horizon-game --no-default-features --bin horizon-client
"${cargo_cmd[@]}" build --manifest-path "$manifest" --release --locked "${target_args[@]}" -p horizon-server --bin horizon-server -p horizon-prepare --bin horizon-prepare

suffix=""
[[ "$horizon_target" == *windows* ]] && suffix=".exe"
target_dir="${CARGO_TARGET_DIR:-$source_dir/target}"
[[ "$target_dir" = /* || "$target_dir" =~ ^[A-Za-z]:[\\/] ]] || target_dir="$repo_root/$target_dir"
binaries="$target_dir/$horizon_target/release"
mkdir -p "$horizon_output/prepare/mods/horizon/src/test"
cp "$binaries/horizon-server$suffix" "$horizon_output/horizon-server$suffix"
cp "$binaries/horizon-client$suffix" "$horizon_output/horizon-engine$suffix"
cp "$binaries/horizon-prepare$suffix" "$horizon_output/prepare/horizon-prepare$suffix"
cp "$source_dir/research-lock.json" "$horizon_output/prepare/research-lock.json"
cp "$source_dir/LICENSE" "$horizon_output/LICENSE"
cp -R "$source_dir/mods/horizon/src/test/java" "$horizon_output/prepare/mods/horizon/src/test/"
printf '%s\n' "$expected" > "$horizon_output/REVISION"
