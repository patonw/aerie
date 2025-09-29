{
  _workspace ? import ../. {},
  _parent ? import ../shell.nix {},
  libraries ? _workspace.libraries,
  pkgs ? _workspace.pkgs,
}:
pkgs.mkShell {
  LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath libraries}";
  inputsFrom = [ _parent ];
  packages = with pkgs; [
      trunk
      cargo-leptos
      tailwindcss
      nodejs_24
      dart-sass
      esbuild
      leptosfmt
  ];
}
