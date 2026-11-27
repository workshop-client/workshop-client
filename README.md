# Workshop Client

A Linux-first Phoenix client mod experiment with WebAssembly plugins.

Workshop launches Discord Phoenix with a Linux native loader and runs a WASM plugin inside its process on a worker thread. It includes native Workshop settings, message transforms, HTTP-response transforms, and badge-image rewriting on the inspected Phoenix build. Plugin installation from the UI is not implemented yet.

## Status

Development preview for Linux x64 Phoenix build `0.1.0-dev.20261123093910.gb71ec80`. Native compatibility is restricted to the inspected build. Global Badges animation and profile-loading fixes are still being verified. This repository is not a stable public release.

## Try it

```sh
cargo run
# Or choose another build and forward Discord arguments:
cargo run -- launch /path/to/Discord.AppImage -- --help
cargo run -- run-plugin examples/hello/plugin.wat
cargo run -- inspect /path/to/Discord.AppImage
```

With no arguments, Workshop launches your November 23 Phoenix AppImage from `~/Applications`. Set `WORKSHOP_DISCORD_APPIMAGE` or pass a path to choose another build. The launcher replaces its process with the AppImage and inherits your desktop environment and terminal output. The launcher preloads `libworkshop_client.so`. The loader activates only in `discord-phoenix`, removes itself from `LD_PRELOAD` before helpers launch, and starts the enabled bundled and custom plugins inside the Phoenix process. Output goes to the terminal and `target/debug/workshop.log`. Close an existing Phoenix instance before launching Workshop so Discord does not hand off to the old process.

The example prints `[workshop] Hello from Workshop WASM!`.

## Client access screen

Workshop overrides the local predicate that displays the account-availability blocking screen. It applies automatically at startup for the inspected Linux x64 build, ELF ID `2b07b1d52b4ed681293b551b726c3e18d51e8c92`. Unknown builds or changed code are left alone and produce a diagnostic. This changes a local UI decision; server-side permissions and authentication still apply.

Disable the override for comparison:

```sh
WORKSHOP_ACCESS_OVERRIDE=0 cargo run
```

The patch changes three bytes in process memory before Phoenix main starts and restores the code page to read/execute. Closing the process removes it. The AppImage on disk is unchanged.

## Load your own plugin inside Phoenix

```sh
WORKSHOP_PLUGIN="$PWD/examples/hello/plugin.wat" cargo run
```

`WORKSHOP_PLUGIN` accepts a `.wasm` or `.wat` file. Clear URLs and Global Badges are bundled. `WORKSHOP_PLUGIN` adds a custom module alongside them. Set `WORKSHOP_LOG` to change the log path. A plugin trap is reported by the worker; it does not unwind through Phoenix's entry point. Text-transform plugins receive send/receive/edit callbacks in fresh instances. General event plugins retain an instance and can handle HTTP responses and image URLs. See [plugin API](docs/plugin-api.md).

`cargo build` builds both the launcher and shared library. Keep them together when copying a build. The launcher stages the library at `~/.local/lib/workshop-client` when the project path contains spaces, because `LD_PRELOAD` cannot quote spaces in library paths. The original AppImage is unchanged.

## Write plugins in Rust

This requires a Rust toolchain with the `wasm32-unknown-unknown` target. The commands below assume a rustup-managed toolchain.

```sh
rustup target add wasm32-unknown-unknown
cargo build --manifest-path examples/rust-plugin/Cargo.toml --target wasm32-unknown-unknown --release
cargo run -- run-plugin examples/rust-plugin/target/wasm32-unknown-unknown/release/workshop_hello.wasm
```

The runtime uses core WebAssembly with an explicit host ABI. Languages that can emit compatible modules can target this API. WASI imports and wasm-bindgen JavaScript glue are not supported.

## Experimental ABI v1

Plugins export `workshop_abi_version() -> i32`, returning `1`, and `workshop_start()`. They may import `workshop.log(ptr: i32, len: i32) -> i32`. Logging requires an exported linear memory named `memory` and UTF-8 bytes. The host copies the bytes and returns `0`, or `-1` for an invalid pointer, invalid UTF-8, or a log limit.

Each invocation gets a fresh instance, one million units of execution fuel, one memory limited to 16 MiB, one table limited to 4096 elements, and at most 100 log entries of 4096 bytes each. Module input is capped at 4 MiB. These are prototype limits, not a complete defense against every compilation or host resource exhaustion case. There are no filesystem, network, Discord token, or native pointer imports.

## Phoenix integration

Keep the WASM runtime separate from version-dependent native hooks. A Linux loader should recognize the actual Phoenix process, validate the build and signatures, and feed copied events into the plugin host. UI changes should use explicit host commands. WASM plugins should never receive raw Phoenix pointers.

See [research notes](docs/phoenix-linux.md) for the inspected build and reference code. Workshop uses pinned Sunbird hook and symbol-location crates. See [attribution](NOTICE.md).

Global Badges is documented in [its plugin README](plugins/global-badges/README.md). Each plugin has its own saved enable switch; plugin-defined boolean settings appear under its card.

## Compact Linux package

Run `scripts/package-linux.sh` to build the public preview. It disables developer CLI commands and text-format WAT support, enables size optimization and link-time optimization, and strips symbols. The script fails if the launcher, runtime library, and package README exceed 5,000,000 bytes uncompressed. The bundled WASM plugins are already embedded in the runtime library, so they are included in that budget.

Output: `target/package/workshop-client-linux-x64.tar.gz`. Keep the launcher and shared library together. Discord and external image tools are excluded; current image preparation requires system curl, ImageMagick, FFmpeg, and timeout. This size target does not mean the entire Discord AppImage or those system dependencies fit within 5 MB.

Normal `cargo run` keeps the developer commands. `cargo run --profile dist --no-default-features` uses the compact public configuration.
