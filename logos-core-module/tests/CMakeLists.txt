find_package(Qt6Test REQUIRED)

# ── test_module: verifies LmaoComponent loads, name() and version() work ──
add_executable(test_module test_module.cpp)

target_include_directories(test_module PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/../src"
    "${CMAKE_CURRENT_SOURCE_DIR}/../interfaces"
)

target_link_libraries(test_module PRIVATE
    Qt6::Core
    Qt6::Test
    Qt6::Widgets
    Qt6::Quick
    Qt6::Qml
    Qt6::QuickWidgets
    lmao_ffi
    component-interfaces
)

add_test(NAME test_module COMMAND test_module)
