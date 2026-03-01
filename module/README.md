# Logos Core IComponent Module

Qt plugin that wraps the `waku-a2a-ffi` Rust library as a Logos Core IComponent module,
loadable by `logos-app-poc`.

## Architecture

```
┌─────────────────────────────────┐
│  logos-app-poc (Qt host app)    │
│  loads IComponent plugins       │
├─────────────────────────────────┤
│  messaging_a2a_ui.so            │  ← This module
│  MessagingA2AUIComponent (C++)  │
│  MessagingA2ABackend (C++)      │
│  MessagingA2AView.qml           │
├─────────────────────────────────┤
│  libwaku_a2a_ffi.so             │  ← Rust FFI crate
│  (C API: init/announce/send/…)  │
├─────────────────────────────────┤
│  Rust core crates               │
│  (crypto, transport, node)      │
└─────────────────────────────────┘
```

## Build

### Prerequisites
- Qt6 (Core, Widgets, Quick, Qml, QuickWidgets)
- logos-cpp-sdk installed
- `libwaku_a2a_ffi.so` built (`cargo build --release -p waku-a2a-ffi` from repo root)

### Steps

```bash
# Build the Rust FFI library first
cd /path/to/logos-messaging-a2a
cargo build --release -p waku-a2a-ffi

# Build the Qt module
cd module
mkdir build && cd build
cmake .. \
    -DLOGOS_CORE_ROOT=/path/to/logos-cpp-sdk/install \
    -DWAKU_A2A_FFI_LIB=../../target/release \
    -DWAKU_A2A_FFI_INCLUDE=../../include
make -j$(nproc)
```

### Install
Copy `libmessaging_a2a_ui.so` + `libwaku_a2a_ffi.so` to the logos-app-poc plugin directory.

## QML UI

The module provides a dark-themed QML UI with:
- Initialize / Announce / Discover controls
- Agent discovery list
- Direct messaging panel
- Connection status indicator
