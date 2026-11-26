# Global Badges

A WebAssembly plugin for Workshop that displays community badges from Equibadges. It is inspired by Sunbird's Global Badges plugin by iFlex0x and wolfieeeeeeee. Reference: https://github.com/sunbird-tools/sunbird/blob/main/plugins/global-badges/src/index.ts. Sunbird declares MIT licensing in its workspace.

All provider behavior lives in `src/lib.rs`: the Equibadges endpoint, six-hour disk cache, thirty-second retry schedule, parsing, provider labels, artwork override, URL encoding, profile and badge-list merging, and settings. Workshop supplies generic events and scoped host services. The plugin's HTTPS permission is declared in `manifest.json`.

The plugin preserves Discord's existing badges, skips official Discord entries from community data, filters duplicate and invalid images, and excludes BadgeVault by default. It does not modify the editable `/users/@me/badges` response or send synthetic badge IDs back to Discord. A 404 for another user's badge list can become a 200 with community badges. Badge source and BadgeVault are independently configurable under its plugin card.

Fetching runs on a background tick, never on a profile-render callback. A cold download may mean reopening a profile once the cache is ready. Warm starts read the cached map. Disabling the plugin prevents new profile and image rewrites; an already displayed profile can retain its rendered badges until it is reopened.

Build with a Rust toolchain that includes `wasm32-unknown-unknown`:

```sh
cargo test --manifest-path plugins/global-badges/Cargo.toml
cargo build --manifest-path plugins/global-badges/Cargo.toml --target wasm32-unknown-unknown --release
cp plugins/global-badges/target/wasm32-unknown-unknown/release/workshop_global_badges.wasm plugins/global-badges/plugin.wasm
cargo build --release
```

Verified: plugin policy tests, actual WASM schema, real Equibadges fetch/cache, real provider data producing profile badges while preserving existing badges, and image URL round trips. Native adapters install on the pinned Linux Phoenix build. Visual profile rendering still requires confirmation in Phoenix.

Artwork is prepared through the host's generic image service. The plugin requests 64px PNG previews and GIF animation for animated badge URLs, allowing WebP sources to render without pretending their original bytes are PNG/GIF. Real PNG, WebP, and GIF provider fixtures were downloaded and converted successfully; the GIF fixture retained eight frames. Actual profile/animation rendering is being verified in Phoenix.
