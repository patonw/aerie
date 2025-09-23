{
  _workspace ? import ../. {},
  _parent ? import ../shell.nix {},
  pkgs ? _workspace.pkgs,
}:
pkgs.mkShell {
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
