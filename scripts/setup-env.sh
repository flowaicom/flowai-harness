#!/usr/bin/env bash
set -euo pipefail

RUST_MIN_VERSION="1.88.0"
RUST_INSTALL_VERSION="1.94.0"
PYTHON_VERSION="3.12.13"
PYTHON_MAJOR_MINOR="3.12"
UV_VERSION="0.10.9"
BUN_VERSION="1.3.5"

dry_run=0

SCRIPT_PATH="${BASH_SOURCE[0]}"
SCRIPT_DIR="${SCRIPT_PATH%/*}"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
    cat <<'USAGE'
Usage: scripts/setup-env.sh [--dry-run]

Installs the pinned Flow AI private preview toolchain into the current user's
environment when tools are missing or incompatible.
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

version_ge() {
    local have="$1"
    local need="$2"
    local have_major have_minor have_patch need_major need_minor need_patch
    IFS=. read -r have_major have_minor have_patch <<<"$have"
    IFS=. read -r need_major need_minor need_patch <<<"$need"
    have_patch="${have_patch:-0}"
    need_patch="${need_patch:-0}"

    if ((have_major > need_major)); then return 0; fi
    if ((have_major < need_major)); then return 1; fi
    if ((have_minor > need_minor)); then return 0; fi
    if ((have_minor < need_minor)); then return 1; fi
    ((have_patch >= need_patch))
}

version_major_minor() {
    local version="$1"
    local major minor _patch
    IFS=. read -r major minor _patch <<<"$version"
    printf "%s.%s" "$major" "$minor"
}

extract_semver() {
    local value="$1"
    if [[ "$value" =~ ([0-9]+)\.([0-9]+)\.([0-9]+) ]]; then
        printf "%s.%s.%s" "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "${BASH_REMATCH[3]}"
        return 0
    fi
    return 1
}

has_version_at_least() {
    local command_name="$1"
    local required="$2"
    local output version
    command -v "$command_name" >/dev/null 2>&1 || return 1
    output="$("$command_name" --version 2>&1 || true)"
    version="$(extract_semver "$output")" || return 1
    version_ge "$version" "$required"
}

has_rust_toolchain() {
    has_version_at_least rustc "$RUST_MIN_VERSION" \
        && has_version_at_least cargo "$RUST_MIN_VERSION"
}

has_python312() {
    local output version
    if command -v python3.12 >/dev/null 2>&1; then
        output="$(python3.12 --version 2>&1 || true)"
        version="$(extract_semver "$output")" || return 1
        [[ "$(version_major_minor "$version")" == "$PYTHON_MAJOR_MINOR" ]]
        return
    fi

    command -v uv >/dev/null 2>&1 && uv python find "$PYTHON_MAJOR_MINOR" >/dev/null 2>&1
}

run_or_print() {
    local message="$1"
    shift
    if ((dry_run)); then
        printf "Would %s\n" "$message"
    else
        "$@"
    fi
}

require_curl() {
    if ! command -v curl >/dev/null 2>&1; then
        printf "curl is required to install preview tooling. Install curl and rerun this script.\n" >&2
        exit 1
    fi
}

install_uv() {
    if has_version_at_least uv "$UV_VERSION"; then
        printf "uv: already compatible\n"
        return
    fi

    if ((dry_run)); then
        printf "Would install uv %s\n" "$UV_VERSION"
        return
    fi

    require_curl
    curl -LsSf "https://astral.sh/uv/${UV_VERSION}/install.sh" | sh
    export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
}

install_python() {
    if has_python312; then
        printf "Python %s: already compatible\n" "$PYTHON_MAJOR_MINOR"
        return
    fi

    if ((dry_run)); then
        printf "Would install Python %s via uv\n" "$PYTHON_VERSION"
        return
    fi

    if ! command -v uv >/dev/null 2>&1; then
        printf "uv is required before installing Python %s.\n" "$PYTHON_VERSION" >&2
        exit 1
    fi
    uv python install "$PYTHON_VERSION"
}

install_rust() {
    if has_rust_toolchain; then
        printf "Rust: already compatible\n"
        return
    fi

    if ((dry_run)); then
        printf "Would install Rust %s via rustup\n" "$RUST_INSTALL_VERSION"
        return
    fi

    require_curl
    if ! command -v rustup >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
            | sh -s -- -y --default-toolchain none
        export PATH="$HOME/.cargo/bin:$PATH"
    fi
    rustup toolchain install "$RUST_INSTALL_VERSION"
    rustup override set "$RUST_INSTALL_VERSION"
}

install_bun() {
    if has_version_at_least bun "$BUN_VERSION"; then
        printf "Bun: already compatible\n"
        return
    fi

    if ((dry_run)); then
        printf "Would install Bun %s\n" "$BUN_VERSION"
        return
    fi

    require_curl
    curl -fsSL https://bun.com/install | bash -s "bun-v${BUN_VERSION}"
    export PATH="$HOME/.bun/bin:$PATH"
}

install_uv
cd "$REPO_ROOT"
install_python
install_rust
install_bun

if ((dry_run)); then
    printf "\nDry run complete. Re-run without --dry-run to install missing tools.\n"
else
    printf "\nPreview tooling setup complete. Run ./scripts/check-env.sh to verify.\n"
fi
