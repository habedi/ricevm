{
  description = "RiceVM: A Dis virtual machine and Limbo compiler in Rust";

  inputs = {
    self.submodules = true;
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      forAllSystems = f: nixpkgs.lib.genAttrs supportedSystems (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        in
        f pkgs rustToolchain
      );
    in
    {
      devShells = forAllSystems (pkgs: rustToolchain:
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              rustToolchain
              gnumake
              pkg-config
              SDL2
              cargo-audit
              cargo-nextest
              pre-commit
              uv
              (python3.withPackages (ps: with ps; [ mkdocs mkdocs-material mkdocstrings-python ]))
            ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              pkgs.alsa-lib
              pkgs.cargo-tarpaulin
              pkgs.gdb
            ];
          };
        }
      );

      packages = forAllSystems (pkgs: rustToolchain:
        let
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };
        in
        {
          default = rustPlatform.buildRustPackage {
            pname = "ricevm";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
            src = pkgs.lib.cleanSource self;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [ "-p" "ricevm-cli" ];

            meta = {
              description = "A Dis virtual machine and Limbo compiler";
              homepage = "https://github.com/habedi/ricevm";
              license = with pkgs.lib.licenses; [ mit asl20 ];
              mainProgram = "ricevm-cli";
              platforms = supportedSystems;
            };
          };
        }
      );
    };
}
