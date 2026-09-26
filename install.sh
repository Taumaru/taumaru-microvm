#!/bin/sh
# Installer for the taumaru-microvm CLI (`microvm`).
#
#   curl -fsSL https://microvm.taumaru.com/install.sh | sh
#
# Environment overrides:
#   TAUMARU_MICROVM_VERSION        Release to install, e.g. v0.3.0 (default: latest)
#   TAUMARU_INSTALL_DIR            Directory for the binary (default: ~/.local/bin or ~/bin)
#   TAUMARU_NO_MODIFY_PATH=1       Do not edit shell profile files
#   TAUMARU_MICROVM_DOWNLOAD_BASE  Base URL hosting the release archives (for mirrors/testing)
set -eu

REPO="Taumaru/taumaru-microvm"
BIN_NAME="microvm"
MARKER_BEGIN="# >>> taumaru-microvm >>>"
MARKER_END="# <<< taumaru-microvm <<<"

if [ -t 2 ] && [ -z "${NO_COLOR:-}" ]; then
    BOLD="$(printf '\033[1m')"
    RED="$(printf '\033[31m')"
    YELLOW="$(printf '\033[33m')"
    GREEN="$(printf '\033[32m')"
    RESET="$(printf '\033[0m')"
else
    BOLD="" RED="" YELLOW="" GREEN="" RESET=""
fi

info() { printf '%s\n' "$*" >&2; }
warn() { printf '%swarning:%s %s\n' "$YELLOW" "$RESET" "$*" >&2; }
fail() {
    printf '%serror:%s %s\n' "$RED" "$RESET" "$*" >&2
    exit 1
}
has() { command -v "$1" >/dev/null 2>&1; }

detect_target() {
    os="$(uname -s)"
    [ "$os" = "Linux" ] || fail "unsupported operating system '$os': microvm runs Firecracker, which requires Linux."

    arch="$(uname -m)"
    case "$arch" in
        x86_64 | amd64) arch="x86_64" ;;
        aarch64 | arm64) arch="aarch64" ;;
        *) fail "unsupported architecture '$arch': prebuilt binaries exist for x86_64 and aarch64." ;;
    esac
    TARGET="${arch}-unknown-linux-musl"
}

require_tools() {
    if has curl; then
        DOWNLOADER="curl"
    elif has wget; then
        DOWNLOADER="wget"
    else
        fail "either 'curl' or 'wget' is required to download microvm."
    fi
    has tar || fail "'tar' is required to extract microvm."
    has mktemp || fail "'mktemp' is required."
    if has sha256sum; then
        SHA256="sha256sum"
    elif has shasum; then
        SHA256="shasum -a 256"
    else
        fail "'sha256sum' or 'shasum' is required to verify the download."
    fi
}

download() {
    url="$1"
    dest="$2"
    if [ "$DOWNLOADER" = "curl" ]; then
        curl -fsSL --retry 3 -o "$dest" "$url"
    else
        wget -q -O "$dest" "$url"
    fi
}

release_base_url() {
    if [ -n "${TAUMARU_MICROVM_DOWNLOAD_BASE:-}" ]; then
        printf '%s' "${TAUMARU_MICROVM_DOWNLOAD_BASE%/}"
        return
    fi
    version="${TAUMARU_MICROVM_VERSION:-latest}"
    if [ "$version" = "latest" ]; then
        printf 'https://github.com/%s/releases/latest/download' "$REPO"
    else
        case "$version" in
            v*) ;;
            *) version="v$version" ;;
        esac
        printf 'https://github.com/%s/releases/download/%s' "$REPO" "$version"
    fi
}

path_contains() {
    case ":${PATH:-}:" in
        *":$1:"*) return 0 ;;
        *) return 1 ;;
    esac
}

choose_install_dir() {
    if [ -n "${TAUMARU_INSTALL_DIR:-}" ]; then
        INSTALL_DIR="$TAUMARU_INSTALL_DIR"
        return
    fi
    [ -n "${HOME:-}" ] || fail "HOME is not set; set TAUMARU_INSTALL_DIR to choose where to install."

    # Prefer a home bin directory that is already on PATH, then one that already exists.
    for dir in "$HOME/.local/bin" "$HOME/bin"; do
        if path_contains "$dir"; then
            INSTALL_DIR="$dir"
            return
        fi
    done
    for dir in "$HOME/.local/bin" "$HOME/bin"; do
        if [ -d "$dir" ]; then
            INSTALL_DIR="$dir"
            return
        fi
    done
    INSTALL_DIR="$HOME/.local/bin"
}

detect_profile() {
    shell_name="$(basename "${SHELL:-sh}")"
    case "$shell_name" in
        bash) PROFILE="$HOME/.bashrc" ;;
        zsh) PROFILE="${ZDOTDIR:-$HOME}/.zshrc" ;;
        fish) PROFILE="${XDG_CONFIG_HOME:-$HOME/.config}/fish/config.fish" ;;
        *) PROFILE="$HOME/.profile" ;;
    esac
}

append_path_block() {
    file="$1"
    if [ -f "$file" ] && grep -Fq "$MARKER_BEGIN" "$file"; then
        return 1
    fi
    mkdir -p "$(dirname "$file")"
    # Write paths under HOME as $HOME/... so the profile stays portable.
    case "$INSTALL_DIR" in
        "$HOME"/*) dir="\$HOME${INSTALL_DIR#"$HOME"}" ;;
        *) dir="$INSTALL_DIR" ;;
    esac
    case "$file" in
        *.fish) line="fish_add_path -g \"$dir\"" ;;
        *) line="case \":\$PATH:\" in *\":$dir:\"*) ;; *) export PATH=\"$dir:\$PATH\" ;; esac" ;;
    esac
    {
        printf '\n%s\n' "$MARKER_BEGIN"
        printf '%s\n' "$line"
        printf '%s\n' "$MARKER_END"
    } >>"$file"
    return 0
}

configure_path() {
    PATH_UPDATED=0
    if path_contains "$INSTALL_DIR"; then
        return
    fi
    if [ "${TAUMARU_NO_MODIFY_PATH:-0}" = "1" ]; then
        warn "$INSTALL_DIR is not on your PATH; add it manually."
        return
    fi

    detect_profile
    if append_path_block "$PROFILE"; then
        info "Added $INSTALL_DIR to PATH in $PROFILE"
    fi
    # Login bash shells read ~/.bash_profile instead of ~/.bashrc; cover it when it does not
    # already source ~/.bashrc.
    if [ "$(basename "${SHELL:-sh}")" = "bash" ] && [ -f "$HOME/.bash_profile" ] \
        && ! grep -Eq '\.bashrc' "$HOME/.bash_profile"; then
        if append_path_block "$HOME/.bash_profile"; then
            info "Added $INSTALL_DIR to PATH in $HOME/.bash_profile"
        fi
    fi
    PATH_UPDATED=1
}

check_runtime() {
    if [ ! -e /dev/kvm ]; then
        warn "/dev/kvm was not found; Firecracker MicroVMs need KVM (enable virtualization or load the kvm module)."
    fi
    if ! has sudo && ! has pkexec; then
        warn "neither 'sudo' nor 'pkexec' was found; most microvm commands need root privileges."
    fi
}

main() {
    detect_target
    require_tools
    choose_install_dir

    base_url="$(release_base_url)"
    archive="${BIN_NAME}-${TARGET}.tar.gz"

    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "$tmp_dir"' EXIT INT TERM

    info "Downloading ${BOLD}${archive}${RESET} from ${base_url}"
    download "$base_url/$archive" "$tmp_dir/$archive" \
        || fail "could not download $base_url/$archive"
    download "$base_url/$archive.sha256" "$tmp_dir/$archive.sha256" \
        || fail "could not download checksum $base_url/$archive.sha256"

    expected="$(cut -d ' ' -f 1 <"$tmp_dir/$archive.sha256")"
    actual="$(cd "$tmp_dir" && $SHA256 "$archive" | cut -d ' ' -f 1)"
    [ -n "$expected" ] && [ "$expected" = "$actual" ] \
        || fail "checksum mismatch for $archive (expected $expected, got $actual)."

    tar -xzf "$tmp_dir/$archive" -C "$tmp_dir" "$BIN_NAME" \
        || fail "could not extract $BIN_NAME from $archive"

    mkdir -p "$INSTALL_DIR" || fail "could not create $INSTALL_DIR"
    cp "$tmp_dir/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME.tmp"
    chmod 755 "$INSTALL_DIR/$BIN_NAME.tmp"
    mv -f "$INSTALL_DIR/$BIN_NAME.tmp" "$INSTALL_DIR/$BIN_NAME"

    configure_path
    export PATH="$INSTALL_DIR:$PATH"

    installed_version="$("$INSTALL_DIR/$BIN_NAME" --version 2>/dev/null)" \
        || fail "$INSTALL_DIR/$BIN_NAME was installed but does not run on this system."

    check_runtime

    info ""
    info "${GREEN}${BOLD}Installed${RESET} ${installed_version} to $INSTALL_DIR/$BIN_NAME"
    if [ "$PATH_UPDATED" = "1" ]; then
        info ""
        info "To use microvm in this terminal, run:"
        case "$PROFILE" in
            *.fish) info "  ${BOLD}source $PROFILE${RESET}" ;;
            *) info "  ${BOLD}. \"$PROFILE\"${RESET}" ;;
        esac
        info "New terminals will pick it up automatically."
    fi
    info ""
    info "Get started with: ${BOLD}${BIN_NAME} --help${RESET}"
}

main "$@"
