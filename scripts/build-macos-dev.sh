#!/bin/bash
# Build the isolated Moor Dev app; never replace the released Moor installation.
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo 'This build requires macOS and Apple toolchains.' >&2; exit 2; }
[[ "$(uname -m)" == arm64 ]] || { echo 'This build currently targets Apple Silicon.' >&2; exit 2; }
command -v cargo >/dev/null || { echo 'Install the Rust toolchain first.' >&2; exit 2; }
command -v pnpm >/dev/null || { echo 'Install pnpm first.' >&2; exit 2; }
command -v xcrun >/dev/null || { echo 'Install Xcode Command Line Tools first.' >&2; exit 2; }
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
[[ "$(node -p 'require("./src-tauri/tauri.conf.json").identifier')" == 'com.specdest.moor-dev' ]] || {
  echo 'Unexpected bundle ID: refusing to build over released Moor.' >&2; exit 2;
}
pnpm install --frozen-lockfile
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml profile_tokens_enforce_agent_isolation --lib
cargo test --manifest-path src-tauri/Cargo.toml isolated_tokens_and_rotation --lib
pnpm tauri build --target aarch64-apple-darwin --bundles dmg
printf '\nMoor Dev DMG artifacts:\n'
find src-tauri/target/aarch64-apple-darwin/release/bundle/dmg -maxdepth 1 -name '*Moor*Dev*.dmg' -print
