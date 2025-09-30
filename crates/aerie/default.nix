{
  sources,
  pkgs,
  fenix,
  nixgl,
  gitignoreSource,
  rust-toolchain,
  naersk,
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
  ];
  nixGL = nixgl.auto.nixGLDefault; # Necessary for running glutin on non-Nixos distros
  aerie = naersk.buildPackage {
    # Command line launchers
    name = "aerie-bin";
    src = gitignoreSource ./.;
    cargoBuildOptions = opts: opts ++ [ "--package aerie" ];

    nativeBuildInputs = with pkgs; [
      pkg-config
      cmake
      makeWrapper
    ];

    buildInputs = with pkgs; [
    ] ++ libraries;

    # postFixup = ''
    #   if [[ -f $out/bin/simple-runner ]]; then
    #     wrapProgram $out/bin/simple-runner \
    #       --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath libraries}
    #   fi
    #
    #   if [[ -f $out/bin/aerie ]]; then
    #     wrapProgram $out/bin/aerie \
    #       --run ${nixGL}/bin/nixGL \
    #       --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath libraries}
    #   fi
    # '';
  };
in
rec {
  inherit libraries;

  bin = pkgs.writeShellApplication {
    name = "aerie";
    runtimeInputs = [nixGL aerie];
    text = ''
      export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath libraries}
      nixGL ${aerie}/bin/aerie "$@"
    '';
  };

  runner = pkgs.writeShellApplication {
    name = "aerie-runner";
    runtimeInputs = [aerie];
    text = ''
      export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath libraries}
      simple-runner "$@"
    '';
  };

  desktop = pkgs.makeDesktopItem {
    # Desktop launcher only
    name = "Aerie";
    desktopName = "Aerie Agentic Workflows";
    exec = "${nixGL}/bin/nixGL ${bin}/bin/aerie";
  };

  app = pkgs.buildEnv {
    # all launchers
    name = "aerie-app";
    paths = [
      bin
      runner
      desktop
    ];
  };
}


