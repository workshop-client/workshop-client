# Linux Phoenix research

Inspected November 26, 2026.

## Local build

Input: `~/Applications/discord-desktop-0.1.0-dev.20261123093910.gb71ec80-linux-x64.appimage`

The AppImage was extracted into `/tmp/workshop-appimage/squashfs-root` without launching Discord's GUI. Its `AppRun` is a shell script that resolves its directory and executes `usr/bin/discord-phoenix`, forwarding arguments. The original AppImage was not modified.

Executable naming matches Sunbird's Linux process check. This establishes a plausible loading route, not verified native hook compatibility.

## References

- [Sunbird](https://github.com/sunbird-tools/sunbird), inspected commit `8559cbeeea56a430605217b065a7f020ff72fe4b`.
- [Astral](https://github.com/astral-app/astral), inspected commit `adcbf3d8f81359b84e084f573676cbd445d434ad`.
- [Wasmi API](https://docs.rs/wasmi/2.0.3/wasmi/), used by this prototype.

Sunbird has a QuickJS plugin runtime and separate desktop native integration. Useful files include `scripts/sunbird-linux.sh`, `crates/loader/src/lib.rs`, `crates/runtime-desktop`, `crates/sigs/src/desktop.rs`, and `crates/hook/src/x86.rs`. Its Linux loader uses LD_PRELOAD, recognizes `discord-phoenix` through `/proc/self/exe`, and removes its preload entry before helpers inherit it.

Astral targets iOS and Android and places its Lua implementation under `packages/core/lua`. Its lifecycle and plugin API are useful design references; desktop hooking requires separate work.

## Next implementation milestones

1. Implemented process attachment with a Linux preload loader and a WASM worker. Verified the bundled plugin logs from the actual `discord-phoenix` PID and Phoenix reaches its first frame. This does not yet install native function hooks.
2. Resolve and validate native functions against this exact build. Fail closed when signatures are missing or ambiguous.
3. Deliver one copied event through an event ABI with allocation, payload size limits, and per-callback fuel. Handle traps per plugin and keep WASM work off the UI thread.
4. Add manifests, explicit permissions, enable/disable state, and unload behavior.
5. Add Phoenix settings UI and a reversible local launcher. Test startup, UI responsiveness, plugin traps, and ordinary Discord behavior in the actual client.

Do not equate the existing standalone plugin tests with successful Discord integration.

## Verified Workshop attachment

The loader and bundled hello WASM plugin ran inside `discord-phoenix` PID 53287. The Workshop log reported attachment, `Hello from Workshop WASM!`, and plugin completion. Phoenix startup subsequently reported `window_created` and `first_frame`. The process map contained Workshop's staged shared library. Verification used the November 23 build listed above.

## Independent client access-screen override

Workshop's implementation is in `src/access.rs`. Sunbird was inspected to understand the access-status semantics, so this is not a clean-room implementation. No Sunbird source code or locator implementation was copied. Workshop uses a different patch point and checks the exact local ELF build instead of adopting Sunbird's signature scanner.

Local `objdump` disassembly identified the predicate at ELF virtual address `0x0e3176a1`. It checks a signed-in state, calls the access getter, compares its byte result against 2, and returns whether the result differs. Its alternate branch already returns false. At `0x0e3176af`, Workshop replaces the three-byte `setne al` with `xor eax,eax; nop`. The getter, stack adjustment and return remain intact. Sunbird instead replaces the getter's entry with a constant-return implementation.

The ELF build ID, note layout, complete 25-byte predicate on disk, and mapped predicate bytes must match before the patch applies. The main executable base comes from `dl_iterate_phdr`, so ASLR is supported. The override runs synchronously before the WASM worker starts. There is no cross-version offset guessing.

Verified on November 26 with PID 59541: the loader reported the applied override, the WASM hello plugin completed, and Phoenix reported `window_created` and `first_frame`. The blocking screen's disappearance has not been visually verified. `WORKSHOP_ACCESS_OVERRIDE=0` disables the patch for comparisons. An updated Discord executable needs a newly inspected compatibility entry.
