#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
native_dir=${PYDOCCRAFT_GO_LIB_DIR:-bindings/go/native}
mkdir -p "$native_dir"
native_dir=$(cd "$native_dir" && pwd -P)
# rustc and the linker may also create large temporary files.
TMPDIR=${TMPDIR:-$native_dir}
export TMPDIR
cargo build --locked --no-default-features --lib
target_dir=${CARGO_TARGET_DIR:-target}
set -- "$target_dir"/debug/deps/libserde_json-*.rlib
if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
  echo "expected exactly one serde_json rlib in $target_dir/debug/deps; use a dedicated CARGO_TARGET_DIR" >&2
  exit 1
fi
rustc --edition=2021 --crate-type cdylib bindings/go/native/bridge.rs \
  --extern _core="$target_dir/debug/lib_core.rlib" --extern serde_json="$1" -L dependency="$target_dir/debug/deps" \
  -o "$native_dir/libpydoccraft_go.so"
