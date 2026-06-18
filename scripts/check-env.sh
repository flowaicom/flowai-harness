#!/usr/bin/env bash
set -euo pipefail

RUST_MIN_VERSION="1.88.0"
PYTHON_VERSION="3.12.13"
PYTHON_MAJOR_MINOR="3.12"
UV_VERSION="0.10.9"
BUN_VERSION="1.3.5"

failures=0

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

report_missing() {
    local name="$1"
    local expected="$2"
    printf "%s: missing (expected %s)\n" "$name" "$expected"
    failures=$((failures + 1))
}

report_incompatible() {
    local name="$1"
    local have="$2"
    local expected="$3"
    printf "%s: incompatible (%s; expected %s)\n" "$name" "$have" "$expected"
    failures=$((failures + 1))
}

check_rust() {
    if ! command -v rustc >/dev/null 2>&1; then
        report_missing "rustc" ">= $RUST_MIN_VERSION"
    else
        local rustc_output rustc_version
        rustc_output="$(rustc --version 2>&1 || true)"
        if ! rustc_version="$(extract_semver "$rustc_output")"; then
            report_incompatible "rustc" "$rustc_output" ">= $RUST_MIN_VERSION"
        elif version_ge "$rustc_version" "$RUST_MIN_VERSION"; then
            printf "rustc: ok (%s)\n" "$rustc_version"
        else
            report_incompatible "rustc" "$rustc_version" ">= $RUST_MIN_VERSION"
        fi
    fi

    if ! command -v cargo >/dev/null 2>&1; then
        report_missing "cargo" ">= $RUST_MIN_VERSION"
        return
    fi

    local cargo_output cargo_version
    cargo_output="$(cargo --version 2>&1 || true)"
    if ! cargo_version="$(extract_semver "$cargo_output")"; then
        report_incompatible "cargo" "$cargo_output" ">= $RUST_MIN_VERSION"
    elif version_ge "$cargo_version" "$RUST_MIN_VERSION"; then
        printf "cargo: ok (%s)\n" "$cargo_version"
    else
        report_incompatible "cargo" "$cargo_version" ">= $RUST_MIN_VERSION"
    fi
}

check_uv() {
    if ! command -v uv >/dev/null 2>&1; then
        report_missing "uv" ">= $UV_VERSION"
        return
    fi

    local output version
    output="$(uv --version 2>&1 || true)"
    if ! version="$(extract_semver "$output")"; then
        report_incompatible "uv" "$output" ">= $UV_VERSION"
        return
    fi
    if version_ge "$version" "$UV_VERSION"; then
        printf "uv: ok (%s)\n" "$version"
    else
        report_incompatible "uv" "$version" ">= $UV_VERSION"
    fi
}

check_python() {
    local output version
    if command -v python3.12 >/dev/null 2>&1; then
        output="$(python3.12 --version 2>&1 || true)"
        if version="$(extract_semver "$output")" \
            && [[ "$(version_major_minor "$version")" == "$PYTHON_MAJOR_MINOR" ]]; then
            printf "python: ok (%s)\n" "$version"
            return
        fi
        report_incompatible "python" "${version:-$output}" "$PYTHON_MAJOR_MINOR.x"
        return
    fi

    if command -v uv >/dev/null 2>&1 && uv python find "$PYTHON_MAJOR_MINOR" >/dev/null 2>&1; then
        printf "python: ok (%s via uv managed Python)\n" "$PYTHON_MAJOR_MINOR"
        return
    fi

    report_missing "python" "$PYTHON_MAJOR_MINOR.x"
}

check_bun() {
    if ! command -v bun >/dev/null 2>&1; then
        report_missing "bun" ">= $BUN_VERSION"
        return
    fi

    local output version
    output="$(bun --version 2>&1 || true)"
    if ! version="$(extract_semver "$output")"; then
        report_incompatible "bun" "$output" ">= $BUN_VERSION"
        return
    fi
    if version_ge "$version" "$BUN_VERSION"; then
        printf "bun: ok (%s)\n" "$version"
    else
        report_incompatible "bun" "$version" ">= $BUN_VERSION"
    fi
}

check_rust
check_uv
check_python
check_bun

if ((failures > 0)); then
    printf "\nRun ./scripts/setup-env.sh to install missing or incompatible tools.\n"
    exit 1
fi

printf "\nFlow AI preview environment is ready.\n"
