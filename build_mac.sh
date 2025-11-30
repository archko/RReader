#!/bin/bash

# Script to build Mac app bundle for RReader
# Usage: ./build_mac.sh [intel|arm]
# Default: arm

# Parse argument
if [ "$1" = "intel" ]; then
    echo "Building for Intel (x86_64-apple-darwin)"
else
    echo "Building for ARM (aarch64-apple-darwin)"
fi

# Ensure necessary dirs
APP_NAME="RReader"
APP_DIR="${APP_NAME}.app"
CONTENTS_DIR="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"

# Script is in project root
# No need to cd

# Build release binary
echo "Building release binary..."
cargo build --release

if [ $? -ne 0 ]; then
    echo "Build failed!"
    exit 1
fi

# Clean previous build
rm -rf "$APP_DIR"

# Create app bundle structure
echo "Creating app bundle structure..."
mkdir -p "$MACOS_DIR"
mkdir -p "$RESOURCES_DIR"

# Copy binary from target/release
echo "Copying binary..."
cp "target/release/rreader" "$MACOS_DIR/"

# Copy plist and icon
echo "Copying assets..."
cp "assets/Info.plist" "$CONTENTS_DIR/"
cp "assets/app_icon.icns" "$RESOURCES_DIR/"

echo "App bundle created: ${APP_DIR}"
echo "To create DMG: hdiutil create -volname '${APP_NAME}' -srcfolder '${APP_DIR}' -ov '${APP_NAME}.dmg'"
