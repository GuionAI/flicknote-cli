{
  description = "FlickNote CLI — local-first note management with cloud sync";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rust = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        nativeBuildInputs = with pkgs; [
          rust
          pkg-config
          llvmPackages.libclang
          sqlite
        ];

        buildInputs = with pkgs; [
          cacert
          openssl
          sqlite
        ] ++ lib.optionals stdenv.isDarwin [
          darwin.apple_sdk.frameworks.Security
          darwin.apple_sdk.frameworks.SystemConfiguration
        ];

        flicknote = pkgs.rustPlatform.buildRustPackage {
          pname = "flicknote";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "powersync-0.0.7" = "sha256-HLnbt1qo7C7a0fCELefloSLSXFRg+cOMeWT8suRs4u4=";
            };
          };

          nativeBuildInputs = nativeBuildInputs;
          buildInputs = buildInputs;
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";

          meta = with pkgs.lib; {
            description = "Local-first note management CLI with cloud sync";
            homepage = "https://github.com/GuionAI/flicknote-cli";
            license = licenses.mit;
            mainProgram = "flicknote";
          };
        };

      in
      {
        packages = {
          default = flicknote;
          flicknote = flicknote;
        };

        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs buildInputs;
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";

          shellHook = ''
            echo "flicknote-cli dev shell (rust $(rustc --version | cut -d' ' -f2))"
          '';
        };
      });
}
