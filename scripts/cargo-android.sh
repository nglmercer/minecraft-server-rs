#!/usr/bin/env bash
# Cross-compile the Termux ARM64 binary using the Android NDK on Linux.
# Native Termux builds use ordinary cargo / build.sh instead.
set -euo pipefail

ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_LATEST_HOME:-}}"
if [[ -z "$ndk" ]]; then
  echo "Set ANDROID_NDK_HOME to an installed Android NDK." >&2
  exit 1
fi
compiler_dir="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
compiler="$compiler_dir/aarch64-linux-android24-clang"
if [[ ! -x "$compiler" ]]; then
  echo "Android API 24 ARM64 compiler not found: $compiler" >&2
  exit 1
fi

# ring builds C in addition to Rust; both compilers must target Android.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER:-$compiler}"
export CC_aarch64_linux_android="${CC_aarch64_linux_android:-$compiler}"
export AR_aarch64_linux_android="${AR_aarch64_linux_android:-$compiler_dir/llvm-ar}"

if [[ $# == 0 ]]; then
  set -- build --release --locked -p panel --bin mcpanel
fi
cargo_command="$1"
shift
exec cargo "$cargo_command" --target aarch64-linux-android "$@"
