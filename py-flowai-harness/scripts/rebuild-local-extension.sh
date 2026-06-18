#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Rebuild and install the local flowai_harness._internal extension.

This targets the Python interpreter used by the local flowai-harness script,
then copies Cargo's cdylib output into flowai_harness/_internal<EXT_SUFFIX>.

Usage:
  py-flowai-harness/scripts/rebuild-local-extension.sh [cargo build args]

Environment:
  FLOWAI_HARNESS_PYTHON   Python interpreter to target.
  FLOWAI_HARNESS_VENV     Venv directory. Defaults to <repo>/.venv.
  CARGO_TARGET_DIR        Optional Cargo target directory.

Examples:
  py-flowai-harness/scripts/rebuild-local-extension.sh
  FLOWAI_HARNESS_PYTHON=.venv/bin/python py-flowai-harness/scripts/rebuild-local-extension.sh
  py-flowai-harness/scripts/rebuild-local-extension.sh --release
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
package_dir="$(cd "$script_dir/.." && pwd)"
repo_dir="$(cd "$package_dir/.." && pwd)"

venv_dir="${FLOWAI_HARNESS_VENV:-$repo_dir/.venv}"
python="${FLOWAI_HARNESS_PYTHON:-$venv_dir/bin/python}"

if [[ ! -x "$python" ]]; then
  echo "error: Python interpreter is not executable: $python" >&2
  echo "set FLOWAI_HARNESS_PYTHON or FLOWAI_HARNESS_VENV to target the right environment" >&2
  exit 2
fi

extension_suffix="$("$python" - <<'PY'
import sysconfig

suffix = sysconfig.get_config_var("EXT_SUFFIX")
if not suffix:
    raise SystemExit("Python did not report EXT_SUFFIX")
print(suffix)
PY
)"

profile_dir="debug"
next_is_profile=0
for arg in "$@"; do
  if [[ "$next_is_profile" == "1" ]]; then
    profile_dir="$arg"
    next_is_profile=0
    continue
  fi
  case "$arg" in
    --release)
      profile_dir="release"
      ;;
    --profile)
      next_is_profile=1
      ;;
    --profile=*)
      profile_dir="${arg#--profile=}"
      ;;
  esac
done
if [[ "$profile_dir" == "dev" ]]; then
  profile_dir="debug"
fi

case "$(uname -s)" in
  Darwin)
    built_lib="lib_internal.dylib"
    rustflags_extra="-C link-arg=-undefined -C link-arg=dynamic_lookup"
    ;;
  Linux)
    built_lib="lib_internal.so"
    rustflags_extra=""
    ;;
  *)
    echo "error: unsupported platform for direct extension install: $(uname -s)" >&2
    echo "use maturin develop for this platform" >&2
    exit 2
    ;;
esac

target_root="${CARGO_TARGET_DIR:-$repo_dir/target}"
source_lib="$target_root/$profile_dir/$built_lib"
dest_lib="$package_dir/flowai_harness/_internal$extension_suffix"

export PYO3_PYTHON="$python"
if [[ -n "$rustflags_extra" ]]; then
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }$rustflags_extra"
fi

echo "python: $python"
echo "extension suffix: $extension_suffix"
echo "cargo profile dir: $profile_dir"

cargo build -p flowai-harness-python "$@"

if [[ ! -f "$source_lib" ]]; then
  echo "error: expected Cargo output does not exist: $source_lib" >&2
  exit 1
fi

tmp_lib="$dest_lib.tmp.$$"
cp "$source_lib" "$tmp_lib"
mv "$tmp_lib" "$dest_lib"

echo "installed: $dest_lib"

PYTHONPATH="$package_dir${PYTHONPATH:+:$PYTHONPATH}" "$python" - "$dest_lib" <<'PY'
import pathlib
import sys

expected = pathlib.Path(sys.argv[1]).resolve()
import flowai_harness._internal as internal

actual = pathlib.Path(internal.__file__).resolve()
if actual != expected:
    raise SystemExit(f"imported {actual}, expected {expected}")

print(f"verified import: {actual}")
PY
