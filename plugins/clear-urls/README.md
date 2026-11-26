# Clear URLs

Experimental core WASM plugin. Tracking rules are based on Sunbird Clear URLs by amaanq, version 0.1.2, from https://github.com/sunbird-tools/sunbird/blob/main/plugins/clear-urls/src/index.ts. Sunbird declares MIT licensing in its Cargo workspace. Workshop's C implementation preserves host-scoped rules, fragments, and retained query bytes.

Build with Clang:

```sh
clang --target=wasm32 -O2 -nostdlib -fno-builtin -Wl,--no-entry -Wl,--export-memory -Wl,-z,stack-size=65536 -Wl,--max-memory=16777216 plugins/clear-urls/plugin.c -o plugins/clear-urls/plugin.wasm
cargo run -- transform plugins/clear-urls/plugin.wasm 'https://example.com/?utm_source=test&a=1'
```

The module exports the startup ABI and an experimental text-transform ABI. The host writes up to 65536 UTF-8 bytes to the pointer returned by `workshop_buffer()`, calls `workshop_transform(length)`, then reads the returned byte count at that pointer. Each call has bounded memory and execution fuel.

Phoenix hooks now invoke the plugin for outgoing message-create JSON requests and incoming message creation/updates. See ../../docs/plugin-api.md for the event API and limitations. Native hook installation is verified at startup; end-to-end chat behavior requires live verification.
