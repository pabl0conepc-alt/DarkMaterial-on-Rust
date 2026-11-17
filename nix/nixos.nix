# NixOS module. `dmrFor pkgs` returns the package. It does not import the
# driftwm module itself (the flake's `nixosModules.default` adds it), so it
# also works when driftwm is already set up some other way.
{ dmrFor }:
{ config, lib, pkgs, options, ... }:
let
  cfg = config.programs.darkmaterial;
  dmr = dmrFor pkgs;
in
{
  options.programs.darkmaterial = {
    enable = lib.mkEnableOption "DarkMaterial, a DankMaterialShell-style shell for driftwm";
    package = lib.mkOption {
      type = lib.types.package;
      default = dmr;
      defaultText = lib.literalExpression "the DarkMaterial package";
      description = "The DarkMaterial package (daemon, widgets, patched EWW).";
    };
  };

  config = lib.mkIf cfg.enable {
    programs = lib.optionalAttrs (options.programs ? driftwm) { driftwm.enable = lib.mkDefault true; };
    # The driftwm autostart (idle/lock, clipboard history) and the default
    # terminal binding run outside the dmr wrapper: put them on PATH.
    environment.systemPackages = [ cfg.package ] ++ (with pkgs; [ swayidle wl-clipboard cliphist foot ]);

    fonts.packages = with pkgs; [
      cfg.package # Google Sans Flex (bundled)
      material-symbols
    ];

    # Services the shell talks to (all optional, all event driven).
    networking.networkmanager.enable = lib.mkDefault true;
    hardware.bluetooth.enable = lib.mkDefault true;
    services.power-profiles-daemon.enable = lib.mkDefault true;
    services.upower.enable = lib.mkDefault true;
    services.pipewire = {
      enable = lib.mkDefault true;
      pulse.enable = lib.mkDefault true;
    };
    # Lock screen authentication.
    security.pam.services.hyprlock = { };
    xdg.portal = {
      enable = true;
      extraPortals = [ pkgs.xdg-desktop-portal-gtk ];
    };
  };
}
