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
        command -v "$tool" >/dev/null 2>&1 || fail "Falta la herramienta: $tool"
    done
    [ "$(uname -s)" = Linux ] || fail 'Solo se admite Linux.'
    case "$(uname -m)" in
        x86_64 | amd64) asset=pi-sandbox-linux-x86_64 ;;
        *) fail 'Solo hay releases para Linux x86_64; compila desde el código fuente para otra arquitectura.' ;;
    esac
    [ -n "${HOME:-}" ] || fail 'HOME no está definido.'
    destination=${PI_SANDBOX_INSTALL_DIR:-"$HOME/.local/bin"}
    case "$destination" in
        /*) ;;
        *) fail 'PI_SANDBOX_INSTALL_DIR debe ser una ruta absoluta.' ;;
    esac
    version=${PI_SANDBOX_VERSION:-latest}
    case "$version" in
        *[!a-zA-Z0-9._-]* | . | ..) fail 'PI_SANDBOX_VERSION debe ser un tag como v0.1.0 o latest.' ;;
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
    printf 'Descargando pi-sandbox (%s, Linux x86_64)...\n' "$version"
    for file in "$asset" "$asset.sha256"; do
        curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
            --tlsv1.2 --output "$tmp/$file" "$base/$file" ||
            fail "No se pudo descargar $file. Comprueba la conexión y que exista una release con estos assets."
    done

    # Do not blindly trust filenames in the downloaded checksum manifest.
    # Verify exactly the binary we will install, never another named file.
    expected=''
    filename=''
    read -r expected filename < "$tmp/$asset.sha256" || fail 'Checksum incompleto.'
    [ "$filename" = "$asset" ] || fail 'El checksum no corresponde al binario esperado.'
    [ "${#expected}" -eq 64 ] || fail 'Checksum SHA-256 inválido.'
    case "$expected" in *[!0-9a-f]*) fail 'Checksum SHA-256 inválido.' ;; esac
    printf '%s  %s\n' "$expected" "$asset" | (cd "$tmp" && sha256sum -c -) ||
        fail 'Falló la verificación SHA-256. No se ha instalado el binario.'

    mkdir -p -- "$destination"
    # Stage on the destination filesystem, then rename atomically. A failed
    # download/check/copy leaves the previous installation untouched.
    [ ! -d "$destination/pi-sandbox" ] || fail 'El destino pi-sandbox es un directorio.'
    stage=$(mktemp "$destination/.pi-sandbox.XXXXXXXX")
    cp -- "$tmp/$asset" "$stage"
    chmod 755 "$stage"
    mv -f -- "$stage" "$destination/pi-sandbox"
    stage=''
    printf 'Instalado en %s/pi-sandbox\n' "$destination"
    case ":${PATH:-}:" in
        *:"$destination":*) ;;
        *) printf 'Añade %s a tu PATH para usar el comando pi-sandbox.\n' "$destination" ;;
    esac
    printf '\nComprobando Pi y Bubblewrap...\n'
    if ! "$destination/pi-sandbox" --check; then
        printf '\nLauncher instalado. Resuelve las dependencias indicadas y ejecuta pi-sandbox --check de nuevo.\n' >&2
    fi
}

main "$@"
