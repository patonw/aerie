{
  _workspace ? import ../. {},
  _parent ? import ../shell.nix {},
  pkgs ? _workspace.pkgs,
  aerie ? _workspace.aerie,
}:
let
  libraries = aerie.libraries;
in
pkgs.mkShell {
  inputsFrom = [ _parent ];
  LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath libraries}";
  packages = with pkgs; [
  ] ++ libraries;
}

