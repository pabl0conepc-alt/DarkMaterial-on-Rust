{
  lib,
  rustPlatform,
  makeWrapper,
  pkg-config,
  eww,
  pulseaudio,
  brightnessctl,
  networkmanager,
  curl,
  wl-clipboard,
  cliphist,
  grim,
  slurp,
  hyprlock,
  hyprpicker,
  fuzzel,
  swayidle,
  wlsunset,
  libqalculate,
  power-profiles-daemon,
  xdg-utils,
  ddcutil,
  procps,
  systemd,
  material-symbols,
  adwaita-icon-theme,
  hicolor-icon-theme,
}:
let
  # EWW + the DarkMaterial patch (same as Sheels'): `:onkeypress` and
  # `:autofocus` on `input`, memoised props and `for` loops (~8x less CPU per
  # update), `:visible` fixes and single-line pasted text.
  eww' = eww.overrideAttrs (old: {
    patches = (old.patches or [ ]) ++ [ ./eww-dmr.patch ];
    # gtk-rs crates need a lot of memory per rustc: at most two at a time so
    # the build also works on machines with little RAM.
    preBuild = (old.preBuild or "") + ''
      export NIX_BUILD_CORES=$(( NIX_BUILD_CORES > 2 ? 2 : NIX_BUILD_CORES ))
    '';
  });
  runtimeDeps = [
    eww'
    pulseaudio # pactl (works with pipewire-pulse)
    brightnessctl
    networkmanager # nmcli
    curl
    wl-clipboard
    cliphist
    grim
    slurp
    hyprlock
    hyprpicker
    fuzzel
    swayidle
    wlsunset # night mode
    libqalculate # launcher calculator
    power-profiles-daemon # powerprofilesctl
    xdg-utils
    ddcutil
    procps # pidof
    systemd # systemd-inhibit, systemctl
  ];
in
rustPlatform.buildRustPackage {
  pname = "darkmaterial";
  version = "1.0.0";

  src = lib.cleanSourceWith {
    src = ../.;
    filter = path: _type: !(lib.hasInfix "/target" path) && !(lib.hasInfix "/docs/" path);
  };
  cargoRoot = "daemon";
  buildAndTestSubdir = "daemon";
  cargoLock.lockFile = ../daemon/Cargo.lock;

  nativeBuildInputs = [ makeWrapper pkg-config ];

  postInstall = ''
    share=$out/share/dmr
    mkdir -p $share $out/share/fonts/truetype
    cp -r eww schemes templates config $share/
    cp fonts/*.ttf $out/share/fonts/truetype/
    install -Dm755 bin/dmr-emoji bin/dmr-clipboard -t $out/bin
    for b in $out/bin/dmr $out/bin/dmr-emoji $out/bin/dmr-clipboard; do
      wrapProgram $b \
        --prefix PATH : ${lib.makeBinPath runtimeDeps}:$out/bin \
        --set-default DMR_SHARE $share \
        --set-default DMR_FONT_DIRS $out/share/fonts:${material-symbols}/share/fonts \
        --suffix XDG_DATA_DIRS : ${adwaita-icon-theme}/share:${hicolor-icon-theme}/share
    done
  '';

  meta = {
    description = "DankMaterialShell recreated for the driftwm compositor: EWW widgets driven by a Rust daemon";
    homepage = "https://github.com/pabl0conepc-alt/DarkMaterial-on-Rust";
    license = lib.licenses.gpl3Only;
    platforms = lib.platforms.linux;
    mainProgram = "dmr";
  };
}
