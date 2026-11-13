#!/bin/sh
# Install a published Linux binary; no sudo, Rust, Python or shell-config edits.
# Usage: curl -fsSL https://raw.githubusercontent.com/ewiggin/pi-sandbox/main/install.sh | sh
set -eu

fail() {
    printf 'pi-sandbox installer: %s\n' "$*" >&2
    exit 1
}

cleanup() {
    if [ -n "${stage:-}" ]; then rm -f -- "$stage"; fi
    if [ -n "${tmp:-}" ]; then rm -rf -- "$tmp"; fi
}

main() {
    for tool in uname curl sha256sum mktemp mkdir cp chmod mv rm; do
        command -v "$tool" >/dev/null 2>&1 || fail "Required tool not found: $tool"
    done
    [ "$(uname -s)" = Linux ] || fail 'Only Linux is supported.'
    case "$(uname -m)" in
        x86_64 | amd64) asset=pi-sandbox-linux-x86_64 ;;
        *) fail 'Releases are only available for Linux x86_64; build from source for other architectures.' ;;
    esac
    [ -n "${HOME:-}" ] || fail 'HOME is not set.'
    destination=${PI_SANDBOX_INSTALL_DIR:-"$HOME/.local/bin"}
    case "$destination" in
        /*) ;;
        *) fail 'PI_SANDBOX_INSTALL_DIR must be an absolute path.' ;;
    esac
    version=${PI_SANDBOX_VERSION:-latest}
    case "$version" in
        *[!a-zA-Z0-9._-]* | . | ..) fail 'PI_SANDBOX_VERSION must be a tag such as v0.1.0 or latest.' ;;
    esac
    base=https://github.com/ewiggin/pi-sandbox/releases
    if [ "$version" = latest ]; then
        base=$base/latest/download
    else
        base=$base/download/$version
    fi

    tmp=''
    stage=''
    trap cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    trap 'exit 129' HUP
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/pi-sandbox-install.XXXXXXXX")
    printf 'Downloading pi-sandbox (%s, Linux x86_64)...\n' "$version"
    for file in "$asset" "$asset.sha256"; do
        curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
            --tlsv1.2 --output "$tmp/$file" "$base/$file" ||
            fail "Could not download $file. Check your connection and that a release with these assets exists."
    done

    # Do not blindly trust filenames in the downloaded checksum manifest.
    # Verify exactly the binary we will install, never another named file.
    expected=''
    filename=''
    read -r expected filename < "$tmp/$asset.sha256" || fail 'Incomplete checksum.'
    [ "$filename" = "$asset" ] || fail 'The checksum does not match the expected binary filename.'
    [ "${#expected}" -eq 64 ] || fail 'Invalid SHA-256 checksum.'
    case "$expected" in *[!0-9a-f]*) fail 'Invalid SHA-256 checksum.' ;; esac
    printf '%s  %s\n' "$expected" "$asset" | (cd "$tmp" && sha256sum -c -) ||
        fail 'SHA-256 verification failed. The binary has not been installed.'

    mkdir -p -- "$destination"
    # Stage on the destination filesystem, then rename atomically. A failed
    # download/check/copy leaves the previous installation untouched.
    [ ! -d "$destination/pi-sandbox" ] || fail 'The pi-sandbox destination is a directory.'
    stage=$(mktemp "$destination/.pi-sandbox.XXXXXXXX")
    cp -- "$tmp/$asset" "$stage"
    chmod 755 "$stage"
    mv -f -- "$stage" "$destination/pi-sandbox"
    stage=''
    printf 'Installed to %s/pi-sandbox\n' "$destination"
    case ":${PATH:-}:" in
        *:"$destination":*) ;;
        *) printf 'Add %s to your PATH to use the pi-sandbox command.\n' "$destination" ;;
    esac
    printf '\nChecking Pi and Bubblewrap...\n'
    if ! "$destination/pi-sandbox" --check; then
        printf '\nLauncher installed. Resolve the dependencies listed above and run pi-sandbox --check again.\n' >&2
    fi
}

main "$@"
