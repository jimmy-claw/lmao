find_package(Qt6Test REQUIRED)

# ── test_dashboard: verifies DashboardComponent loads and FFI bindings work ──
add_executable(test_dashboard test_dashboard.cpp)

target_include_directories(test_dashboard PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/../src"
    "${CMAKE_CURRENT_SOURCE_DIR}/../interfaces"
)

target_link_libraries(test_dashboard PRIVATE
    Qt6::Core
    Qt6::Test
    Qt6::Widgets
    Qt6::Quick
    Qt6::Qml
    Qt6::QuickWidgets
    lmao_ffi
    component-interfaces
)

add_test(NAME test_dashboard COMMAND test_dashboard)
