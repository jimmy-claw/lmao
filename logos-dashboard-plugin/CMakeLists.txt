cmake_minimum_required(VERSION 3.20)
project(lmao-dashboard-plugin VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)

# ── Qt6 dependencies ──────────────────────────────────────────────────────────
find_package(Qt6 REQUIRED COMPONENTS Core Widgets Quick Qml QuickWidgets)

# ── IComponent interface ──────────────────────────────────────────────────────
add_library(component-interfaces INTERFACE)
target_include_directories(component-interfaces INTERFACE
    ${CMAKE_CURRENT_SOURCE_DIR}/interfaces
)

# ── LMAO FFI library (Rust) ──────────────────────────────────────────────────
if(NOT LMAO_FFI_LIB)
    message(FATAL_ERROR "Set -DLMAO_FFI_LIB=<path to liblmao_ffi.so>")
endif()

add_library(lmao_ffi SHARED IMPORTED)
set_target_properties(lmao_ffi PROPERTIES
    IMPORTED_LOCATION "${LMAO_FFI_LIB}"
)

# ── Dashboard plugin ─────────────────────────────────────────────────────────
add_library(lmao_dashboard MODULE
    src/DashboardComponent.cpp
    src/DashboardBackend.cpp
    qml/qml.qrc
)

target_include_directories(lmao_dashboard PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src
)

target_link_libraries(lmao_dashboard PRIVATE
    Qt6::Core
    Qt6::Widgets
    Qt6::Quick
    Qt6::Qml
    Qt6::QuickWidgets
    component-interfaces
    lmao_ffi
)

# Ensure the plugin can find liblmao_ffi at runtime
set_target_properties(lmao_dashboard PROPERTIES
    INSTALL_RPATH "$ORIGIN"
)
if(APPLE)
    set_target_properties(lmao_dashboard PROPERTIES
        INSTALL_RPATH "@loader_path"
    )
endif()

# ── Tests ────────────────────────────────────────────────────────────────────
enable_testing()
add_subdirectory(tests)
