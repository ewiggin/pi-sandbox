#!/bin/sh
# Native musl release by default; TARGET may select another installed Rust target.
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
if [ -z "${TARGET:-}" ]; then
    case "$(uname -m)" in
        x86_64) TARGET=x86_64-unknown-linux-musl ;;
        aarch64) TARGET=aarch64-unknown-linux-musl ;;
        *) echo 'Unrecognized architecture; set TARGET explicitly.' >&2; exit 1 ;;
    esac
fi
cargo build --release --locked --target "$TARGET"
package="pi-sandbox-$TARGET"
out="dist/$package"
mkdir -p "$out"
install -m755 "target/$TARGET/release/pi-sandbox" "$out/pi-sandbox"
install -m644 README.md "$out/README.md"
tar -C dist -czf "dist/$package.tar.gz" "$package"
(cd dist && sha256sum "$package.tar.gz" > "$package.tar.gz.sha256")
printf 'Binary: %s/pi-sandbox\nPackage: dist/%s.tar.gz\n' "$out" "$package"
