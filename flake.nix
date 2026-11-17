{
  description = "DarkMaterial — DankMaterialShell recreated for driftwm with EWW + a Rust daemon";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    driftwm = {
      url = "github:malbiruk/driftwm";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self, nixpkgs, driftwm, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (s: f nixpkgs.legacyPackages.${s});
    in
    {
      packages = forAll (pkgs: rec {
        darkmaterial = pkgs.callPackage ./nix/package.nix { };
        default = darkmaterial;
      });

      overlays.default = final: _prev: { darkmaterial = final.callPackage ./nix/package.nix { }; };

      # `darkmaterial`: the shell only (use it if you already import driftwm's module).
      # `default`: driftwm's module + the shell.
      nixosModules.darkmaterial = import ./nix/nixos.nix {
        dmrFor = pkgs: self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      };
      nixosModules.default = { imports = [ driftwm.nixosModules.default self.nixosModules.darkmaterial ]; };

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc rustfmt clippy pkg-config eww ];
        };
      });
    };
}
