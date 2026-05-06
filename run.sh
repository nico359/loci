#!/bin/sh
# Quick dev build+run script (not used by meson/flatpak)
set -e
cd "$(dirname "$0")"
glib-compile-resources src/loci.gresource.xml --target=loci.gresource
cargo build
GSETTINGS_SCHEMA_DIR=data ./target/debug/loci
