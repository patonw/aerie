{
  _workspace ? import ./. {},
  pkgs ? _workspace.pkgs,
  libraries ? _workspace.libraries,
  rust-toolchain ? _workspace.rust-toolchain,
  qdrant-network ? "bridge", # For some reason, the default is broken in nix-shell
}:
let
  DATA_DIR = "${builtins.getEnv "HOME"}/.local/state/refoliate";

  mcp-inspect = pkgs.writeShellApplication {
    name = "mcp-inspect";
    runtimeInputs = with pkgs; [
      nodejs_24
    ];
    text = ''
      npx @modelcontextprotocol/inspector "$@"
    '';
  };

  qdrant-serve = pkgs.writeShellScriptBin "qdrant-serve" ''
    QDRANT_NETWORK=${"$"}{QDRANT_NETWORK:-${qdrant-network}}
    ${pkgs.podman}/bin/podman run -it --rm --network=$QDRANT_NETWORK -p 6333:6333 -p 6334:6334 -v "${DATA_DIR}/qdrant:/qdrant/storage:z" qdrant/qdrant
  '';
in pkgs.mkShell {
  inherit DATA_DIR;

  LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath libraries}";
  packages = with pkgs; [
    qdrant-serve
    mcp-inspect
      niv
      cmake
      ninja
      cargo-generate
      mdbook
      mdbook-d2
      moon
      nodejs_22
      pkg-config
      rust-toolchain
      uv
  ] ++ libraries;
}
