{
  description = "agent-fw framework development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs @ {
    nixpkgs,
    flake-parts,
    fenix,
    ...
  }:
    flake-parts.lib.mkFlake {inherit inputs;} {
      systems = ["aarch64-darwin" "x86_64-linux" "aarch64-linux"];

      perSystem = {system, ...}: let
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfree = true;
        };

        toolchain = with fenix.packages.${system};
          combine [
            stable.cargo
            stable.rustc
            stable.clippy
            stable.rustfmt
            stable.rust-analyzer
            stable.rust-src
          ];
      in {
        devShells.default = pkgs.mkShell {
          name = "agent-fw-dev";

          packages = with pkgs; [
            toolchain
            pkg-config
            openssl
            cargo-nextest
            cargo-watch
            cargo-expand
            taplo
            git
            python312
            uv
          ];

          PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
          OPENSSL_DIR = "${pkgs.openssl.dev}";
          OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
          OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";
          SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
          RUST_SRC_PATH =
            "${fenix.packages.${system}.stable.rust-src}/lib/rustlib/src/rust/library";

          shellHook = ''
            echo "agent-fw development shell"
            echo "rustc: $(rustc --version)"
            echo "cargo: $(cargo --version)"
            echo "python: $(python --version 2>&1)"
            echo ""
            echo "Common commands:"
            echo "  cargo test -p agent-fw-cli"
            echo "  cargo nextest run"
            echo "  cargo fmt --all"
            echo "  cargo clippy --workspace --all-targets"
          '';
        };
      };
    };
}
