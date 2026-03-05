cmake_minimum_required(VERSION 3.20)
project(lmao-core-module VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# ── logos-module-builder (optional — used in Nix builds) ─────────────────────
if(DEFINED ENV{LOGOS_MODULE_BUILDER_ROOT})
    include($ENV{LOGOS_MODULE_BUILDER_ROOT}/cmake/LogosModule.cmake)
    set(USE_MODULE_BUILDER TRUE)
else()
    set(USE_MODULE_BUILDER FALSE)
endif()

# ── Qt6 ──────────────────────────────────────────────────────────────────────
find_package(Qt6Core         REQUIRED)
find_package(Qt6Widgets      REQUIRED)
find_package(Qt6Quick        REQUIRED)
find_package(Qt6Qml          REQUIRED)
find_package(Qt6QuickWidgets REQUIRED)

set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)

# ── IComponent interface ─────────────────────────────────────────────────────
add_library(component-interfaces INTERFACE)
target_include_directories(component-interfaces INTERFACE
    "${CMAKE_CURRENT_SOURCE_DIR}/interfaces"
)

# ── lmao-ffi (Rust FFI library) ─────────────────────────────────────────────
set(LMAO_FFI_LIB "" CACHE PATH "Directory containing liblmao_ffi.so")
if("${LMAO_FFI_LIB}" STREQUAL "")
    # Default: look in the Cargo target directory
    set(LMAO_FFI_LIB "${CMAKE_CURRENT_SOURCE_DIR}/../target/release")
    message(STATUS "LMAO_FFI_LIB not set — defaulting to ${LMAO_FFI_LIB}")
endif()

add_library(lmao_ffi SHARED IMPORTED)
set_target_properties(lmao_ffi PROPERTIES
    IMPORTED_LOCATION "${LMAO_FFI_LIB}/liblmao_ffi.so"
    IMPORTED_NO_SONAME TRUE
)

# ── Module plugin target ────────────────────────────────────────────────────
set(MODULE_TARGET lmao_module)

add_library(${MODULE_TARGET} SHARED
    src/LmaoComponent.cpp
    src/LmaoBackend.cpp
    src/AgentListModel.cpp
    qml/qml.qrc
)

target_include_directories(${MODULE_TARGET} PRIVATE
    src
)

target_compile_definitions(${MODULE_TARGET} PRIVATE
    QT_PLUGIN
)

target_link_libraries(${MODULE_TARGET} PRIVATE
    Qt6::Core
    Qt6::Widgets
    Qt6::Quick
    Qt6::Qml
    Qt6::QuickWidgets
    lmao_ffi
    component-interfaces
)

# ── RPATH ────────────────────────────────────────────────────────────────────
if(APPLE)
    set_target_properties(${MODULE_TARGET} PROPERTIES
        BUILD_RPATH   "@loader_path"
        INSTALL_RPATH "@loader_path"
    )
elseif(UNIX)
    set_target_properties(${MODULE_TARGET} PROPERTIES
        BUILD_RPATH   "$ORIGIN"
        INSTALL_RPATH "$ORIGIN"
    )
endif()

# ── Install ──────────────────────────────────────────────────────────────────
install(TARGETS ${MODULE_TARGET}
    LIBRARY DESTINATION lib
)

# ── Tests ────────────────────────────────────────────────────────────────────
option(BUILD_TESTS "Build module tests" ON)
if(BUILD_TESTS)
    enable_testing()
    add_subdirectory(tests)
endif()
