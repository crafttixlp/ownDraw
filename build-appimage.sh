#!/bin/bash
# Baut OwnDraw-<Version>-x86_64.AppImage: cargo build --release -> AppDir -> eigene Runtime + SquashFS
# Voraussetzungen: Rust (cargo), gcc, zlib1g-dev (statische libz), squashfs-tools
set -euo pipefail
cd "$(dirname "$0")"
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cargo build --release -p owndraw
rm -rf target/AppDir
cp -a packaging/appimage/appdir target/AppDir
mkdir -p target/AppDir/usr/bin
cp target/release/owndraw target/AppDir/usr/bin/
packaging/appimage/build-appimage.sh target/AppDir "OwnDraw-$VER-x86_64.AppImage"
ls -la "OwnDraw-$VER-x86_64.AppImage"
