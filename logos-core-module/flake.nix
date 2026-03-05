{
  description = "logos-core-module — Logos Core IComponent plugin for LMAO (A2A over Waku)";

  inputs = {
    logos-module-builder.url = "github:logos-co/logos-module-builder";
    nixpkgs.follows = "logos-module-builder/nixpkgs";
  };

  outputs = { self, logos-module-builder, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };

      # Build the lmao-ffi Rust crate from the parent workspace.
      lmao-ffi = pkgs.rustPlatform.buildRustPackage {
        pname = "lmao-ffi";
        version = "0.1.0";
        src = ./..;
        cargoLock.lockFile = ./../Cargo.lock;
        buildAndTestSubdir = "crates/lmao-ffi";
        # We only need the cdylib, skip tests (they need network).
        doCheck = false;
        postInstall = ''
          mkdir -p $out/lib
          cp target/*/release/liblmao_ffi.so $out/lib/ 2>/dev/null || true
        '';
      };

      # Build the C++ IComponent module via logos-module-builder.
      module = logos-module-builder.lib.mkLogosModule {
        src = ./.;
        configFile = ./module.yaml;
      };
    in
    {
      packages.${system} = {
        # The main module library (C++ IComponent plugin).
        lib = module.packages.${system}.default or module.packages.${system}.lib or
          (pkgs.stdenv.mkDerivation {
            pname = "lmao-core-module";
            version = "0.1.0";
            src = ./.;
            nativeBuildInputs = with pkgs; [ cmake pkg-config qt6.wrapQtAppsHook ];
            buildInputs = with pkgs; [
              qt6.qtbase
              qt6.qtdeclarative
              qt6.qtquick3d
              lmao-ffi
            ];
            cmakeFlags = [
              "-DLMAO_FFI_LIB=${lmao-ffi}/lib"
            ];
          });

        # The Rust FFI library (for standalone use).
        ffi = lmao-ffi;

        default = self.packages.${system}.lib;
      };
    };
}
