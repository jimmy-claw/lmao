# Builds the messaging-a2a IComponent module (.so)
{ pkgs, common, src }:

let
  cmakeFlags = common.cmakeFlags;

  # Build the Rust FFI library first
  rustFfi = pkgs.rustPlatform.buildRustPackage {
    pname = "waku-a2a-ffi";
    version = "0.1.0";
    inherit src;
    cargoLock.lockFile = "${src}/Cargo.lock";
    buildAndTestSubdir = "crates/waku-a2a-ffi";
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [ pkgs.openssl ];
  };
in
pkgs.stdenv.mkDerivation {
  pname = common.pname;
  version = common.version;
  inherit src;

  nativeBuildInputs = common.nativeBuildInputs;
  buildInputs = common.buildInputs ++ [ rustFfi ];
  inherit (common) meta;

  configurePhase = ''
    runHook preConfigure

    cmake -S module -B build \
      ${pkgs.lib.concatStringsSep " " cmakeFlags} \
      -DLOGOS_CORE_ROOT=''${LOGOS_CORE_ROOT:-/dev/null} \
      -DWAKU_A2A_FFI_LIB=${rustFfi}/lib \
      -DWAKU_A2A_FFI_INCLUDE=include

    runHook postConfigure
  '';

  buildPhase = ''
    runHook preBuild
    cmake --build build
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    mkdir -p $out/lib $out/share/logos-messaging-a2a

    # Install the Qt module
    cp build/libmessaging_a2a_ui.so $out/lib/ || true

    # Install the Rust FFI library alongside
    cp ${rustFfi}/lib/libwaku_a2a_ffi.so $out/lib/ || true

    # Install metadata for Logos Core loader
    cp module/metadata.json $out/share/logos-messaging-a2a/
    cp -r module/icons $out/share/logos-messaging-a2a/ || true

    runHook postInstall
  '';
}
