#!/usr/bin/env bash
set -euo pipefail

dry_run=0

usage() {
    cat <<'USAGE'
Usage: scripts/install.sh [--dry-run]

Checks the Flow AI private preview environment, builds and stages Studio static
assets, installs py-flowai-harness from source into .venv, and prints the
installed flowai-harness version.
USAGE
}

while (($#)); do
    case "$1" in
        --dry-run)
            dry_run=1
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            printf "Unknown argument: %s\n" "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

SCRIPT_PATH="${BASH_SOURCE[0]}"
SCRIPT_DIR="${SCRIPT_PATH%/*}"
SCRIPT_DIR="$(cd "$SCRIPT_DIR" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VENV_PYTHON="$REPO_ROOT/.venv/bin/python"
VENV_FLOWAI="$REPO_ROOT/.venv/bin/flowai-harness"
CHECK_BASH="${BASH:-bash}"

run_step() {
    local display="$1"
    shift
    if ((dry_run)); then
        printf "Would run: %s\n" "$display"
    else
        "$@"
    fi
}

cd "$REPO_ROOT"

if ! "$CHECK_BASH" "$SCRIPT_DIR/check-env.sh"; then
    printf "\nEnvironment check failed.\n"
    printf "Fix the dependency above, or run ./scripts/setup-env.sh.\n"
    exit 1
fi

run_step \
    "bun install --cwd studio --frozen-lockfile" \
    bun install --cwd studio --frozen-lockfile

run_step \
    "uv venv .venv --python 3.12 --clear" \
    uv venv .venv --python 3.12 --clear

run_step \
    ".venv/bin/python scripts/build_studio_static.py --skip-install" \
    "$VENV_PYTHON" scripts/build_studio_static.py --skip-install

run_step \
    "uv pip install --python .venv/bin/python ./py-flowai-harness" \
    uv pip install --python "$VENV_PYTHON" ./py-flowai-harness

run_step \
    ".venv/bin/flowai-harness --version" \
    "$VENV_FLOWAI" --version

if ((dry_run)); then
    printf "\nDry run complete. Re-run without --dry-run to install.\n"
else
    installed_version="$("$VENV_FLOWAI" --version)"
    printf "\nInstalled %s\n" "$installed_version"
    printf "Flow AI private preview install succeeded.\n"
fi
