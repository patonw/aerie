{
  sources ? import ./nix/sources.nix,
  pkgs ? import sources.nixpkgs {},
  fenix ? import sources.fenix {},
  gitignore ? import sources."gitignore.nix" {},
  nixgl ? import sources.nixGL {},
  rust-toolchain ? fenix.combine [
    fenix.complete.toolchain
    fenix.targets.wasm32-unknown-unknown.latest.rust-std
  ],
  naersk ? pkgs.callPackage sources.naersk {
    cargo = rust-toolchain;
    rustc = rust-toolchain;
  },
}:
let
  libraries = with pkgs; [
    stdenv.cc.cc.lib
    xorg.libxcb
    libxkbcommon
    fontconfig
    xorg.libX11
    xorg.libXcursor
    xorg.libXrandr
    xorg.libXi
    xorg.libX11.dev
    libGL
    zlib
    openssl
    wayland
    dbus
  ];

  callPackage = pkgs.lib.callPackageWith {
    inherit sources pkgs fenix rust-toolchain naersk gitignore nixgl;
    inherit (gitignore) gitignoreSource;
  };
  nixGL = nixgl.auto.nixGLDefault; # Necessary for running glutin on non-Nixos distros
  build-aerie = { features ? [] } : naersk.buildPackage {
    # Command line launchers
    name = "aerie-bin";
    gitSubmodules = true;
    src = gitignore.gitignoreSource ./.;
    cargoBuildOptions = opts: opts ++ [ "--package aerie" ] ++ (if features == [] then [] else [ "-F" (pkgs.lib.strings.join "," features)]);

    nativeBuildInputs = with pkgs; [
      pkg-config
      cmake
      makeWrapper
    ];

    buildInputs = with pkgs; [
    ] ++ libraries;
  };
  aerie = build-aerie {};
  migrate-aerie = build-aerie { features = ["migration"]; };
in
rec {
  inherit pkgs libraries rust-toolchain;

  aerie-bin = pkgs.writeShellApplication {
    name = "aerie";
    runtimeInputs = [nixGL aerie];
    text = ''
      export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath libraries}
      nixGL ${aerie}/bin/aerie "$@"
    '';
  };

  aerie-runner = pkgs.writeShellApplication {
    name = "aerie-runner";
    runtimeInputs = [aerie];
    text = ''
      export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath libraries}
      simple-runner "$@"
    '';
  };

  migration = pkgs.writeShellApplication {
    name = "aerie-migration";
    runtimeInputs = [migrate-aerie];
    text = ''
      export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath libraries}
      migrate-workflow "$@"
    '';
  };

  desktop = pkgs.makeDesktopItem {
    # Desktop launcher only
    name = "Aerie";
    desktopName = "Aerie Agentic Workflows";
    exec = "${nixGL}/bin/nixGL ${aerie-bin}/bin/aerie";
  };

  aerie-app = pkgs.buildEnv {
    # all launchers
    name = "aerie-app";
    paths = [
      aerie-bin
      aerie-runner
      desktop
    ];
  };
}
