#!/usr/bin/env bash
# Install the productized Mojo scoring+sampling backend.
#
# Builds the release binary and installs a self-contained runtime:
#
#   <prefix>/bin/phlow-trainer-mojo     (the CLI / production entry)
#   <prefix>/lib/libphlow_scoring.so    (the Mojo kernel library)
#   <prefix>/lib/libKGENCompilerRTShared.so, libAsyncRT*.so,
#   libMSupportGlobals.so, libstdc++.so.6, libgcc_s.so.1
#                                       (vendored Mojo runtime,
#                                        copied by build.rs)
#
# The binary carries an $ORIGIN/../lib rpath and the kernel library
# an $ORIGIN rpath, so the installed tree runs with NO pixi
# environment, NO LD_LIBRARY_PATH, and no environment variables at
# all (proven with `env -i` in the productization record).
#
# Usage: scripts/install-runtime.sh [prefix]
# Default prefix: ~/.local/share/phlow-trainlab-mojo
set -euo pipefail

PREFIX="${1:-$HOME/.local/share/phlow-trainlab-mojo}"
CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_DIR="$(cd "$CRATE_DIR/../.." && pwd)"

echo "install-runtime: building release (workspace $WORKSPACE_DIR)"
# Run cargo from the workspace root so rustup honors the workspace's
# pinned toolchain (rust-toolchain.toml), not the caller's default.
(cd "$WORKSPACE_DIR" && cargo build --release -p phlow-trainer-mojo)

# The build-script output dir appears under two layouts
# (build/<pkg>-<hash>/out and build/<pkg>/<hash>/out); take the
# newest one that actually holds the kernel library.
OUT_DIR=""
for candidate in $(ls -dt \
    "$WORKSPACE_DIR"/target/release/build/phlow-trainer-mojo-*/out \
    "$WORKSPACE_DIR"/target/release/build/phlow-trainer-mojo/*/out 2>/dev/null); do
    if [[ -f "$candidate/libphlow_scoring.so" ]]; then
        OUT_DIR="$candidate"
        break
    fi
done
if [[ -z "$OUT_DIR" ]]; then
    echo "install-runtime: kernel library not found under target/release/build" >&2
    exit 1
fi

mkdir -p "$PREFIX/bin" "$PREFIX/lib"
install -m 0755 "$WORKSPACE_DIR/target/release/phlow-trainer-mojo" "$PREFIX/bin/phlow-trainer-mojo"
for lib in libphlow_scoring.so libKGENCompilerRTShared.so libAsyncRTMojoBindings.so \
           libMSupportGlobals.so libAsyncRTRuntimeGlobals.so libstdc++.so.6 libgcc_s.so.1; do
    if [[ ! -f "$OUT_DIR/$lib" ]]; then
        echo "install-runtime: vendored library $lib missing from $OUT_DIR" >&2
        exit 1
    fi
    install -m 0755 "$OUT_DIR/$lib" "$PREFIX/lib/$lib"
done

echo "install-runtime: installed to $PREFIX"
env -i "$PREFIX/bin/phlow-trainer-mojo" version
