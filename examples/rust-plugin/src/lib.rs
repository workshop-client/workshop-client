#[link(wasm_import_module = "workshop")]
unsafe extern "C" {
    fn log(ptr: *const u8, len: usize) -> i32;
}

#[unsafe(no_mangle)]
pub extern "C" fn workshop_abi_version() -> i32 { 1 }

#[unsafe(no_mangle)]
pub extern "C" fn workshop_start() {
    let message = "Hello from a Rust Workshop plugin!";
    // SAFETY: the host copies these valid UTF-8 bytes during this call.
    unsafe { log(message.as_ptr(), message.len()); }
}
