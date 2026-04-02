{
  description = "RiceVM: A Dis virtual machine and Limbo compiler in Rust";

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
              # Rust toolchain
              rustup

              # Build dependencies
              pkg-config
              openssl

              # Optional: GUI support (SDL2)
              SDL2
              SDL2_ttf

              # Optional: audio support
              alsa-lib

              # Development tools
              cargo-nextest
              cargo-tarpaulin
              cargo-audit

              # Documentation
              python3Packages.mkdocs-material

              # Git hooks
              pre-commit
            ];

            # Ensure the linker can find SDL2 and OpenSSL
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
              SDL2
              SDL2_ttf
              openssl
              alsa-lib
            ]);
          };
        }
      );

      packages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.rustPlatform.buildRustPackage rec {
            pname = "ricevm";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = with pkgs; [
              pkg-config
            ];

            buildInputs = with pkgs; [
              openssl
            ];
          };
        }
      );
    };
}
