#!/usr/bin/env bash
# Cross-build cadastre-server.exe for Windows Server 2012 R2 from Linux, and package the deploy kit.
#
#   sudo apt install gcc-mingw-w64-x86-64      # once
#   rustup toolchain install nightly --profile minimal -c rust-src   # once
#   scripts/build-windows.sh                   # -> dist/cadastre-server-windows-<version>.zip
#
# Target x86_64-win7-windows-gnu (tier 3: nightly + build-std). It links against msvcrt.dll, which
# every Server 2012 R2 has, so nothing else (no VC++ redistributable, no UCRT update) is needed on
# the box. See docs/windows-2012r2-build.md.
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET=x86_64-win7-windows-gnu
export CARGO_TARGET_X86_64_WIN7_WINDOWS_GNU_LINKER="${CARGO_TARGET_X86_64_WIN7_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}"

command -v "$CARGO_TARGET_X86_64_WIN7_WINDOWS_GNU_LINKER" >/dev/null \
  || { echo "missing $CARGO_TARGET_X86_64_WIN7_WINDOWS_GNU_LINKER: sudo apt install gcc-mingw-w64-x86-64" >&2; exit 1; }

cargo +nightly build --release --target "$TARGET" -Z build-std=std,panic_abort -p cadastre-server

EXE="target/$TARGET/release/cadastre-server.exe"

# Guard: the exe must not import anything newer than Windows 8.1 / Server 2012 R2. These are the
# APIs a dependency is most likely to pull in that do not exist there.
if x86_64-w64-mingw32-objdump -p "$EXE" | grep -qiE \
    'api-ms-win-core-synch-l1-2-0|SetThreadDescription|GetThreadDescription|CreateFile2FromAppW|GetTempPath2|VirtualAlloc2|ProcessPrng|api-ms-win-crt-'; then
  echo "ERROR: $EXE imports an API that Windows Server 2012 R2 lacks:" >&2
  x86_64-w64-mingw32-objdump -p "$EXE" | grep -iE 'DLL Name|SetThreadDescription|GetTempPath2|VirtualAlloc2|ProcessPrng' >&2
  exit 1
fi

VERSION="$(git describe --always --dirty)"
STAGE="dist/cadastre-server-windows-$VERSION"
rm -rf "$STAGE" "$STAGE.zip"
mkdir -p "$STAGE"
cp "$EXE" deploy/windows/* "$STAGE/"
echo "$VERSION" > "$STAGE/VERSION.txt"
(cd dist && python3 -c 'import shutil,sys; shutil.make_archive(sys.argv[1], "zip", ".", sys.argv[1])' "$(basename "$STAGE")")
echo "built $STAGE.zip"
