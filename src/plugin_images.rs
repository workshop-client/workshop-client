//! Generic plugin image-URL rewrite adapter for Phoenix's media transport.
use phoenix_hook::{image::Image, x86::Abi};
use phoenix_sigs::desktop;
use std::{
    collections::HashSet,
    ffi::c_void,
    sync::{LazyLock, Mutex, OnceLock},
};
#[repr(C)]
struct Future {
    data: *mut c_void,
    vtable: *const c_void,
}
type Fetch = unsafe fn(*const c_void, *const u8, usize, usize, usize, usize) -> Future;
static FETCH: OnceLock<Fetch> = OnceLock::new();
static URLS: LazyLock<Mutex<HashSet<&'static str>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
pub fn install() -> anyhow::Result<()> {
    crate::access::supported_build()?;
    let image = unsafe { Image::loaded() }
        .into_iter()
        .find(|image| image.file_name() == "discord-phoenix")
        .ok_or_else(|| anyhow::anyhow!("Phoenix image missing"))?;
    let function = desktop::media_fetch(&image).map_err(anyhow::Error::msg)?;
    let prologue = desktop::Prologues::of(Abi::of(&image)).media_fetch;
    let original = unsafe {
        image.hook_entry(
            function,
            prologue.pattern,
            prologue.len,
            fetch as *const () as usize,
        )?
    };
    FETCH
        .set(unsafe { std::mem::transmute::<usize, Fetch>(original) })
        .map_err(|_| anyhow::anyhow!("image adapter installed twice"))?;
    Ok(())
}
unsafe fn fetch(
    this: *const c_void,
    ptr: *const u8,
    len: usize,
    max: usize,
    extra0: usize,
    extra1: usize,
) -> Future {
    let rewritten = std::panic::catch_unwind(|| {
        if ptr.is_null() || len > 8192 {
            return None;
        }
        let url = std::str::from_utf8(unsafe { std::slice::from_raw_parts(ptr, len) }).ok()?;
        // Ordinary images never enter plugin code. This namespace is used by custom badge icons.
        if !url.contains("/badge-icons/") {
            return None;
        }
        let rewritten = crate::plugin_events::image(url);
        eprintln!(
            "[workshop] badge image request: rewritten={}",
            rewritten.is_some()
        );
        let rewritten = rewritten?;
        let mut urls = URLS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(url) = urls.get(rewritten.as_str()) {
            return Some(*url);
        }
        if urls.len() >= 4096 {
            return None;
        }
        let url: &'static str = Box::leak(rewritten.into_boxed_str());
        urls.insert(url);
        Some(url)
    })
    .ok()
    .flatten();
    let (ptr, len) = rewritten.map_or((ptr, len), |url| (url.as_ptr(), url.len()));
    unsafe { FETCH.get().unwrap()(this, ptr, len, max, extra0, extra1) }
}
