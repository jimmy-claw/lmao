cmake_minimum_required(VERSION 3.20)
project(logos-messaging-a2a-module VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# ── logos-cpp-sdk ─────────────────────────────────────────────────────────────
set(LOGOS_CORE_ROOT "" CACHE PATH "Path to logos-cpp-sdk install (contains include/ and lib/)")
if("${LOGOS_CORE_ROOT}" STREQUAL "")
    message(FATAL_ERROR "LOGOS_CORE_ROOT must be set to the logos-cpp-sdk install prefix")
endif()

# ── Qt6 ───────────────────────────────────────────────────────────────────────
find_package(Qt6Core         REQUIRED)
find_package(Qt6Widgets      REQUIRED)
find_package(Qt6Quick        REQUIRED)
find_package(Qt6Qml          REQUIRED)
find_package(Qt6QuickWidgets REQUIRED)

set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)

# ── waku-a2a-ffi (Rust FFI library) ──────────────────────────────────────────
set(WAKU_A2A_FFI_LIB "" CACHE PATH "Directory containing libwaku_a2a_ffi.so")
if("${WAKU_A2A_FFI_LIB}" STREQUAL "")
    # Default: look for it in the Rust target directory
    set(WAKU_A2A_FFI_LIB "${CMAKE_CURRENT_SOURCE_DIR}/../target/release")
    message(STATUS "WAKU_A2A_FFI_LIB not set — defaulting to ${WAKU_A2A_FFI_LIB}")
endif()

set(WAKU_A2A_FFI_INCLUDE "" CACHE PATH "Directory containing waku_a2a.h")
if("${WAKU_A2A_FFI_INCLUDE}" STREQUAL "")
    set(WAKU_A2A_FFI_INCLUDE "${CMAKE_CURRENT_SOURCE_DIR}/../include")
    message(STATUS "WAKU_A2A_FFI_INCLUDE not set — using ${WAKU_A2A_FFI_INCLUDE}")
endif()

# ── SDK interface targets ─────────────────────────────────────────────────────
add_library(logos_cpp_sdk INTERFACE)
target_include_directories(logos_cpp_sdk INTERFACE
    "${LOGOS_CORE_ROOT}/include"
    "${LOGOS_CORE_ROOT}/include/cpp"
)

add_library(logos_core SHARED IMPORTED)
set_target_properties(logos_core PROPERTIES
    IMPORTED_LOCATION "${LOGOS_CORE_ROOT}/lib/liblogos_core.so"
)

# ── component-interfaces (IComponent.h) ──────────────────────────────────────
find_package(component-interfaces QUIET)
if(NOT component-interfaces_FOUND)
    add_library(component-interfaces INTERFACE)
    target_include_directories(component-interfaces INTERFACE
        "${CMAKE_CURRENT_SOURCE_DIR}/interfaces"
    )
endif()

# ── Rust FFI imported library ─────────────────────────────────────────────────
add_library(waku_a2a_ffi SHARED IMPORTED)
set_target_properties(waku_a2a_ffi PROPERTIES
    IMPORTED_LOCATION "${WAKU_A2A_FFI_LIB}/libwaku_a2a_ffi.so"
)

# ── UI Plugin target (IComponent for logos-app-poc) ──────────────────────────
set(UI_TARGET messaging_a2a_ui)

add_library(${UI_TARGET} SHARED
    src/MessagingA2AUIComponent.cpp
    src/MessagingA2ABackend.cpp
    qml/qml.qrc
)

target_include_directories(${UI_TARGET} PRIVATE
    src
    "${WAKU_A2A_FFI_INCLUDE}"
)

target_compile_definitions(${UI_TARGET} PRIVATE
    MESSAGING_A2A_UI_METADATA_FILE="${CMAKE_CURRENT_SOURCE_DIR}/metadata.json"
)

target_link_libraries(${UI_TARGET} PRIVATE
    Qt6::Core
    Qt6::Widgets
    Qt6::Quick
    Qt6::Qml
    Qt6::QuickWidgets
    logos_cpp_sdk
    logos_core
    waku_a2a_ffi
    component-interfaces
)

# ── RPATH ─────────────────────────────────────────────────────────────────────
if(APPLE)
    set_target_properties(${UI_TARGET} PROPERTIES
        BUILD_RPATH   "@loader_path"
        INSTALL_RPATH "@loader_path"
    )
elseif(UNIX)
    set_target_properties(${UI_TARGET} PROPERTIES
        BUILD_RPATH   "$ORIGIN"
        INSTALL_RPATH "$ORIGIN"
    )
endif()

# ── Install ───────────────────────────────────────────────────────────────────
install(TARGETS ${UI_TARGET}
    LIBRARY DESTINATION lib
)
