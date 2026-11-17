# NixOS module for configurations WITHOUT flakes:
#
#   imports = [ "${builtins.fetchTarball "https://github.com/pabl0conepc-alt/DarkMaterial-on-Rust/archive/main.tar.gz"}/nix/module.nix" ];
#   programs.darkmaterial.enable = true;
#
# driftwm itself is enabled through driftwm's own module (programs.driftwm),
# as in its README — install.sh adds it for you when it is missing.
import ./nixos.nix { dmrFor = pkgs: pkgs.callPackage ./package.nix { }; }
