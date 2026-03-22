# Common build configuration for logos-messaging-a2a module
{ pkgs }:

{
  pname = "logos-messaging-a2a";
  version = "0.1.0";

  nativeBuildInputs = [
    pkgs.cmake
    pkgs.ninja
    pkgs.pkg-config
    pkgs.qt6.wrapQtAppsHook
    pkgs.rustPlatform.cargoSetupHook
    pkgs.cargo
    pkgs.rustc
  ];

  buildInputs = [
    pkgs.qt6.qtbase
    pkgs.qt6.qtdeclarative
    pkgs.zstd
    pkgs.krb5
    pkgs.abseil-cpp
    pkgs.zlib
    pkgs.icu
    pkgs.openssl
  ];

  cmakeFlags = [
    "-GNinja"
    "-DCMAKE_BUILD_TYPE=Release"
  ];

  meta = with pkgs.lib; {
    description = "A2A agent-to-agent messaging module for Logos Core";
    homepage = "https://github.com/jimmy-claw/logos-messaging-a2a";
    platforms = platforms.unix;
  };
}
