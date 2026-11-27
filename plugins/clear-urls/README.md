# Clear URLs

Readable Rust source in `src/lib.rs`, compiled to a sandboxed WASM plugin. Tracking rules are based on [Sunbird Clear URLs](https://github.com/sunbird-tools/sunbird/blob/main/plugins/clear-urls/src/index.ts) by amaanq, version 0.1.2. The implementation preserves host-scoped rules, fragments, and retained query bytes.

Build with Rust and the `wasm32-unknown-unknown` target:

```sh
cargo build --manifest-path plugins/clear-urls/Cargo.toml --target wasm32-unknown-unknown --release
cp plugins/clear-urls/target/wasm32-unknown-unknown/release/workshop_clear_urls.wasm plugins/clear-urls/plugin.wasm
cargo run -- transform plugins/clear-urls/plugin.wasm 'https://example.com/?utm_source=test&a=1'
```

The host writes UTF-8 text into `workshop_buffer()`, calls `workshop_on_message(event, length)`, and reads the returned byte count. The same rules run on outgoing messages and incoming message creation/updates. Each call has bounded memory and execution fuel. See `../../docs/plugin-api.md` for the API.
