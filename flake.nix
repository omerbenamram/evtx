{
  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.inputs.flake-utils.follows = "flake-utils";
    nixpkgs.url = "github:nixos/nixpkgs?ref=release-21.05";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        cargo2nixOverlay = import ./overlay;
        overlays = [
          cargo2nixOverlay
          rust-overlay.overlay
        ];

        # 1. Setup nixpkgs with rust and cargo2nix overlays.
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustPkgs = pkgs.rustBuilder.makePackageSet' {
          packageFun = import ./Cargo.nix;
          rustChannel = "1.56.1";
          packageOverrides = pkgs: pkgs.rustBuilder.overrides.all;
          localPatterns = [ ''^(src|tests|templates)(/.*)?'' ''[^/]*\.(rs|toml)$'' ];
        };

        devShell = pkgs.mkShell {
          inputsFrom = pkgs.lib.mapAttrsToList (_: pkg: pkg { }) rustPkgs.noBuild.workspace;
          nativeBuildInputs = [ rustPkgs.rustChannel ] ++ (with pkgs; [cacert]);
          RUST_SRC_PATH = "${rustPkgs.rustChannel}/lib/rustlib/src/rust/library";
        };

      in rec {

        # nix develop
        inherit devShell;

        # the packages in:
        # nix build .#packages.x86_64-linux.cargo2nix
        packages = {

          evtx = rustPkgs.workspace.evtx {};

          ci = pkgs.rustBuilder.runTests rustPkgs.workspace.evtx { };

          shell = devShell;
        };

        # nix build
        defaultPackage = packages.cargo2nix;

        # nix run
        defaultApp = { type = "app"; program = "${defaultPackage}/bin/cargo2nix";};

        # for downstream importer who wants to provide rust themselves
        overlay = cargo2nixOverlay;

        # for downstream importer to create nixpkgs the same way
        inherit overlays;
      }
    );
}
