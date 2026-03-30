{
  description = "RiceVM: A Dis virtual machine implementation in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              # Build
              rustup
              cargo
              rustc

              # Development
              rust-analyzer
              clippy
              rustfmt

              # Testing
              cargo-nextest

              # Git hooks
              pre-commit
            ];
          };
        }
      );

      packages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "ricevm";
            version = "0.1.0-alpha.1";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
          };
        }
      );
    };
}
