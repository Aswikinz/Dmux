#!/usr/bin/env bash
# Installs everything Dmux development needs on Arch Linux.
# Read it before you run it:   bash scripts/arch-install.sh
# It uses sudo for pacman only. It does not install OpenCode (you already have it).
set -euo pipefail

echo "==> System packages"
sudo pacman -S --needed \
  base-devel git github-cli rustup nodejs pnpm python uv \
  podman podman-compose postgresql \
  webkit2gtk-4.1 openssl librsvg libappindicator-gtk3 xdotool curl wget file \
  fuse2 xorg-server-xvfb flatpak flatpak-builder actionlint gitleaks shellcheck

echo "==> Rust toolchain"
rustup default stable
rustup component add clippy rustfmt llvm-tools-preview

echo "==> Cargo tools (this takes a while the first time)"
cargo install --locked tauri-cli cargo-llvm-cov cargo-deny cargo-audit cargo-cyclonedx cargo-mutants tauri-driver

echo "==> Rootless Podman check"
if ! grep -q "^$USER:" /etc/subuid 2>/dev/null; then
  echo "    adding sub-user ID ranges for $USER (needed for rootless containers)"
  sudo usermod --add-subuids 100000-165535 --add-subgids 100000-165535 "$USER"
  podman system migrate
fi
podman info --format '    podman {{.Version.Version}}, rootless: {{.Host.Security.Rootless}}'

echo
echo "Done. Notes:"
echo " - The 'postgresql' package is installed for psql, pg_dump and pg_restore only."
echo "   Do not start its service; the test databases run in Podman."
echo " - Make sure ~/.cargo/bin is on your PATH."
