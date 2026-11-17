#!/usr/bin/env bash
# DarkMaterial installer for NixOS — one command, also used to update:
#
#   curl -fsSL https://raw.githubusercontent.com/pabl0conepc-alt/DarkMaterial-on-Rust/main/install.sh | bash
#
# It adds /etc/nixos/darkmaterial.nix (imported from configuration.nix, a
# backup is kept), rebuilds the system with little memory, then runs
# `dmr setup` for your user (driftwm config with the DMS keybinds, wallpapers
# folder, health check).
set -euo pipefail

REF="${DMR_REF:-main}"
URL="https://github.com/pabl0conepc-alt/DarkMaterial-on-Rust/archive/${REF}.tar.gz"
CONF=/etc/nixos/configuration.nix
MOD=/etc/nixos/darkmaterial.nix

say() { printf '\033[1;35m::\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

[ -f /etc/NIXOS ] || die "this installer is for NixOS (see the README for other setups)"
[ -f "$CONF" ] || die "$CONF not found"
[ "$(id -u)" -ne 0 ] || die "run it as your user (it asks for sudo when needed)"

# Already installed through a flake input? Then only update that input.
if grep -qE 'darkmaterial\.nixosModules|nixosModules\.darkmaterial' /etc/nixos/*.nix 2>/dev/null \
   && ! grep -q 'darkmaterial.nix' "$CONF"; then
  say "DarkMaterial comes from your flake: updating it"
  input=$(grep -oE '\b[A-Za-z0-9_-]*darkmaterial[A-Za-z0-9_-]*\.url' /etc/nixos/flake.nix | head -1 | sed 's/\.url$//')
  (cd /etc/nixos && sudo nix flake update "${input:-darkmaterial}") || true
else
  if [ ! -f "$MOD" ]; then
    say "writing $MOD"
    # driftwm too, unless the system already sets it up.
    DRIFT=false
    if ! grep -qE 'driftwm' "$CONF" /etc/nixos/flake.nix 2>/dev/null; then
      say "driftwm not found in your config: adding it as well"
      DRIFT=true
    fi
    sudo tee "$MOD" >/dev/null <<EOF
# Added by the DarkMaterial installer. Remove this file and its line in
# configuration.nix to uninstall.
{ pkgs, lib, ... }:
let
  system = pkgs.stdenv.hostPlatform.system;
  withDriftwm = $DRIFT;
  drift = builtins.fetchTarball "https://github.com/malbiruk/driftwm/archive/main.tar.gz";
  driftPkg = ((import "\${drift}/flake.nix").outputs {
    self = drift;
    nixpkgs = { lib = pkgs.lib; legacyPackages.\${system} = pkgs; };
  }).packages.\${system}.default;
in
{
  imports = [ "\${builtins.fetchTarball "$URL"}/nix/module.nix" ]
    ++ lib.optional withDriftwm "\${drift}/nix/nixos-module.nix";
  programs = { darkmaterial.enable = true; } // lib.optionalAttrs withDriftwm {
    driftwm = { enable = true; package = driftPkg; };
  };
}
EOF
  fi
  if ! grep -q './darkmaterial.nix' "$CONF"; then
    grep -q './hardware-configuration.nix' "$CONF" \
      || die "could not find ./hardware-configuration.nix in $CONF: add ./darkmaterial.nix to your imports by hand"
    sudo cp "$CONF" "$CONF.bak-darkmaterial"
    say "adding ./darkmaterial.nix to the imports of $CONF (backup: $CONF.bak-darkmaterial)"
    sudo sed -i '0,/\.\/hardware-configuration\.nix/s//&\n      .\/darkmaterial.nix/' "$CONF"
  fi
fi

say "rebuilding (the first time takes a while: dmr and its EWW are compiled)"
# tarball-ttl 0: always fetch the latest DarkMaterial/driftwm tarballs.
FLAGS=(--option cores 2 --option tarball-ttl 0)
if [ -f /etc/nixos/flake.nix ]; then
  sudo nixos-rebuild switch --impure "${FLAGS[@]}"
else
  sudo nixos-rebuild switch "${FLAGS[@]}"
fi

say "setting up your user"
/run/current-system/sw/bin/dmr setup

say "done — log out and pick driftwm on the login screen"
