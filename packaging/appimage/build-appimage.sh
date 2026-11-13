#!/bin/sh
# Baut eine x86_64-AppImage ohne appimagetool:  runtime (eigenes statisches ELF) + SquashFS (gzip).
# Aufruf: build-appimage.sh <AppDir> <Ausgabe.AppImage>
# Umgebung: BLOCK_SIZE (Standard 1M; 128K ist ca. 30 % langsamer beim Bauen, gleich schnell beim Entpacken), CC (Standard gcc)
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
[ $# -eq 2 ] || { echo "Aufruf: $0 <AppDir> <Ausgabe.AppImage>" >&2; exit 2; }
APPDIR=$1
OUT=$2
BLOCK_SIZE=${BLOCK_SIZE:-1M}
CC=${CC:-gcc}

[ -d "$APPDIR" ] || { echo "AppDir $APPDIR fehlt" >&2; exit 1; }
[ -x "$APPDIR/AppRun" ] || { echo "$APPDIR/AppRun fehlt oder ist nicht ausfuehrbar" >&2; exit 1; }
command -v mksquashfs >/dev/null || { echo "mksquashfs fehlt (apt install squashfs-tools)" >&2; exit 1; }

# --- Runtime bei Bedarf (neu) bauen (make-artig: nur wenn runtime.c neuer ist)
RT="$HERE/runtime"
if [ ! -x "$RT" ] || [ "$HERE/runtime.c" -nt "$RT" ] || [ "$0" -nt "$RT" ]; then
    echo "Baue Runtime ..."
    "$CC" -static -O2 -s -Wall -Wno-format-truncation -o "$RT.tmp" "$HERE/runtime.c" -lz
    # Magic 'A','I',0x02 an Offset 8 (e_ident-Padding / EI_ABIVERSION), wie die offizielle Typ-2-Runtime
    printf 'AI\002' | dd of="$RT.tmp" bs=1 seek=8 count=3 conv=notrunc status=none
    # Ende des ELF laut Section-Header muss dem Dateiende entsprechen (sonst stimmt der Payload-Offset nicht)
    shoff=$(od -An -t u8 -j 40 -N 8 "$RT.tmp" | tr -d ' ')
    shentsize=$(od -An -t u2 -j 58 -N 2 "$RT.tmp" | tr -d ' ')
    shnum=$(od -An -t u2 -j 60 -N 2 "$RT.tmp" | tr -d ' ')
    end=$((shoff + shentsize * shnum))
    size=$(stat -c %s "$RT.tmp")
    [ "$end" -eq "$size" ] || { echo "ELF-Ende ($end) != Dateigroesse ($size)" >&2; rm -f "$RT.tmp"; exit 1; }
    mv "$RT.tmp" "$RT"
fi

# --- SquashFS erzeugen und anhaengen
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM
mksquashfs "$APPDIR" "$TMP/payload.sqfs" -comp gzip -b "$BLOCK_SIZE" \
    -root-owned -noappend -no-xattrs -quiet -no-progress >/dev/null
cat "$RT" "$TMP/payload.sqfs" > "$OUT.part"
chmod 755 "$OUT.part"
mv "$OUT.part" "$OUT"
echo "Fertig: $OUT ($(stat -c %s "$OUT") Bytes, Payload-Offset $(stat -c %s "$RT"))"
