{
  sources ? import ./nix/sources.nix,
  pkgs ? import sources.nixpkgs {},
  fenix ? import sources.fenix {},
}:
let
  PLAYWRIGHT_ENV = ''
      export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
      export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
  '';
  libraries = with pkgs; [
    openssl
    wasmtime
  ];
  rustifer = fenix.combine [
    # https://jordankaye.dev/posts/rust-wasm-nix/ but latest instead of stable
    fenix.complete.toolchain
    fenix.targets.wasm32-unknown-unknown.latest.rust-std
  ];
in
{
  shell = pkgs.mkShell {
    LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath libraries}";
    packages = with pkgs; [
      niv
      cmake
      ninja
      cargo-generate
      moon
      nodejs_22
      pkg-config
      rustifer
      uv
    ] ++ libraries;
    shellHook = ''
      ${PLAYWRIGHT_ENV}
    '';
  };
}
