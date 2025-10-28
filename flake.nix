{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = {
    nixpkgs,
    rust-overlay,
    flake-utils,
    crane,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        craneLib = crane.mkLib pkgs;
        lib = pkgs.lib;
        workspace.root = ./.;
        src = lib.fileset.toSource {
          inherit (workspace) root;
          # ./samples ./tests ./README.md have to be including for test stages of the build
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
            ./samples
            ./tests
            ./README.md
            (craneLib.fileset.commonCargoSources workspace.root)
          ];
        };
      in {
        # entrypoint for `nix build .`
        packages.default = craneLib.buildPackage {
          inherit src;
        };
        # entrypoint for `nix develop`
        devShells.default = craneLib.devShell {
          inherit src;
        };
      }
    );
}
