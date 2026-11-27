//! Phoenix HTTP adapter, restricted to message-create JSON requests.
use phoenix_hook::{image::Image, x86::Abi};
use phoenix_sigs::{
    desktop,
    http::{DesktopHttpTransport, RequestLayout},
};
use std::{
    ffi::c_void,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicPtr, Ordering},
    },
};
#[repr(C)]
struct Future {
    data: *mut c_void,
    vtable: *const c_void,
}
#[repr(C)]
struct PollResult {
    headers: [u64; 12],
    body: Vector,
    status: u16,
    padding: [u8; 6],
}
type Poll = unsafe fn(*mut c_void, *mut c_void) -> PollResult;
type Dealloc = unsafe extern "C" fn(*mut u8, usize, usize);
static REQUESTS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, (String, String)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
type Send = unsafe fn(*const c_void, *mut c_void) -> Future;
type Alloc = unsafe extern "C" fn(usize, usize) -> *mut u8;
struct Host {
    send: Send,
    layout: RequestLayout,
    alloc: Alloc,
    dealloc: Dealloc,
    poll: Poll,
}
static HOST: OnceLock<Host> = OnceLock::new();
#[repr(C)]
struct Vector {
    cap: usize,
    ptr: *mut u8,
    len: usize,
}
#[repr(C)]
struct Bytes {
    vtable: *const Vtable,
    ptr: *const u8,
    len: usize,
    data: AtomicPtr<()>,
}
#[repr(C)]
struct Vtable {
    clone: unsafe extern "C" fn(&AtomicPtr<()>, *const u8, usize) -> Bytes,
    vector: unsafe extern "C" fn(*mut (), *const u8, usize) -> Vector,
    mutable: unsafe extern "C" fn(*mut (), *const u8, usize) -> Bytes,
    unique: unsafe extern "C" fn(&AtomicPtr<()>) -> bool,
    drop: unsafe extern "C" fn(*mut (), *const u8, usize),
}
static TABLE: Vtable = Vtable {
    clone: clone_bytes,
    vector: vector_bytes,
    mutable: mutable_bytes,
    unique: unique_bytes,
    drop: drop_bytes,
};
fn owned(bytes: Vec<u8>) -> Bytes {
    let owner = Arc::new(bytes);
    let ptr = owner.as_ptr();
    let len = owner.len();
    Bytes {
        vtable: &TABLE,
        ptr,
        len,
        data: AtomicPtr::new(Arc::into_raw(owner).cast_mut().cast()),
    }
}
unsafe extern "C" fn clone_bytes(data: &AtomicPtr<()>, ptr: *const u8, len: usize) -> Bytes {
    let raw = data.load(Ordering::Relaxed);
    unsafe {
        Arc::increment_strong_count(raw.cast::<Vec<u8>>());
    }
    Bytes {
        vtable: &TABLE,
        ptr,
        len,
        data: AtomicPtr::new(raw),
    }
}
unsafe extern "C" fn drop_bytes(data: *mut (), _: *const u8, _: usize) {
    unsafe {
        drop(Arc::from_raw(data.cast::<Vec<u8>>()));
    }
}
unsafe extern "C" fn unique_bytes(_: &AtomicPtr<()>) -> bool {
    false
}
unsafe extern "C" fn mutable_bytes(data: *mut (), ptr: *const u8, len: usize) -> Bytes {
    Bytes {
        vtable: &TABLE,
        ptr,
        len,
        data: AtomicPtr::new(data),
    }
}
unsafe extern "C" fn vector_bytes(data: *mut (), ptr: *const u8, len: usize) -> Vector {
    let output = unsafe { (HOST.get().unwrap().alloc)(len.max(1), 1) };
    if output.is_null() {
        std::process::abort();
    }
    unsafe {
        std::ptr::copy_nonoverlapping(ptr, output, len);
        drop_bytes(data, ptr, len);
    }
    Vector {
        cap: len.max(1),
        ptr: output,
        len,
    }
}
pub fn install() -> anyhow::Result<()> {
    crate::access::supported_build()?;
    let image = unsafe { Image::loaded() }
        .into_iter()
        .find(|i| i.file_name() == "discord-phoenix")
        .ok_or_else(|| anyhow::anyhow!("Phoenix image missing"))?;
    let found = DesktopHttpTransport::locate(&image).map_err(anyhow::Error::msg)?;
    let alloc = found
        .host_alloc
        .ok_or_else(|| anyhow::anyhow!("host allocator missing"))?;
    let dealloc = found
        .host_dealloc
        .ok_or_else(|| anyhow::anyhow!("host deallocator missing"))?;
    // The extent scanner cannot follow this build's stack-spilled sret pointer.
    // In that case validate the observed 128-byte copy and status-at-0x78 stores.
    let stack_layout = image
        .function_bounds(found.poll_fn)
        .and_then(|(start, end)| image.read(start, end - start))
        .is_some_and(|code| {
            code.windows(10)
                .any(|bytes| bytes == [0xb9, 0x80, 0, 0, 0, 0x48, 0x89, 0xc7, 0xf3, 0xa4])
                && code
                    .windows(5)
                    .any(|bytes| bytes == [0x66, 0x44, 0x89, 0x42, 0x78])
        });
    if !(122..=size_of::<PollResult>()).contains(&found.poll_result)
        && !(found.poll_result == 0 && stack_layout)
    {
        anyhow::bail!("HTTP poll result layout is unverified");
    }
    let pro = desktop::Prologues::of(Abi::of(&image)).frame;
    let trampoline = unsafe {
        image.hook_entry(
            found.send_fn,
            pro.pattern,
            pro.len,
            send as *const () as usize,
        )?
    };
    HOST.set(Host {
        send: unsafe { std::mem::transmute::<usize, Send>(trampoline) },
        layout: found.request,
        alloc: unsafe { std::mem::transmute::<usize, Alloc>(alloc) },
        dealloc: unsafe { std::mem::transmute::<usize, Dealloc>(dealloc) },
        poll: unsafe { std::mem::transmute::<usize, Poll>(found.poll_fn) },
    })
    .map_err(|_| anyhow::anyhow!("send hook already installed"))?;
    let writes = found
        .poll_slots
        .iter()
        .map(|&slot| (slot, poll as *const () as usize))
        .collect::<Vec<_>>();
    unsafe {
        image.swap_words(&writes)?;
    }
    Ok(())
}
unsafe fn send(this: *const c_void, request: *mut c_void) -> Future {
    let host = HOST.get().unwrap();
    let _ = std::panic::catch_unwind(|| unsafe {
        if request.is_null() || !crate::settings::config::get().plugins_enabled {
            return;
        }
        if request.byte_add(host.layout.method).cast::<u8>().read() != 2 {
            return;
        }
        let url = &*request.byte_add(host.layout.url).cast::<Vector>();
        if url.len > 8192 || url.ptr.is_null() {
            return;
        }
        let Ok(url) = std::str::from_utf8(std::slice::from_raw_parts(url.ptr, url.len)) else {
            return;
        };
        // Only exact create-message endpoints, never auth, uploads, or other API calls.
        let path = url.split('?').next().unwrap_or(url);
        let Some((_, tail)) = path.rsplit_once("/channels/") else {
            return;
        };
        let Some(channel) = tail.strip_suffix("/messages") else {
            return;
        };
        if channel.is_empty() || !channel.bytes().all(|c| c.is_ascii_digit()) {
            return;
        }
        let body = request.byte_add(host.layout.body).cast::<u64>();
        if body.read() != 1 {
            return;
        }
        let bytes = &mut *body.add(1).cast::<Bytes>();
        if bytes.ptr.is_null() || bytes.len > 1024 * 1024 {
            return;
        }
        let raw = std::slice::from_raw_parts(bytes.ptr, bytes.len);
        let Some(rewritten) = crate::messages::rewrite_json(crate::MessageEvent::Send, raw) else {
            return;
        };
        let replacement = owned(rewritten.into_bytes());
        let old = std::ptr::replace(bytes, replacement);
        ((*old.vtable).drop)(old.data.load(Ordering::Relaxed), old.ptr, old.len);
    });
    let observed = std::panic::catch_unwind(|| unsafe {
        if request.is_null() {
            return None;
        }
        let tag = request.byte_add(host.layout.method).cast::<u8>().read();
        let method = *[
            "OPTIONS", "GET", "POST", "PUT", "DELETE", "HEAD", "TRACE", "CONNECT", "PATCH", "QUERY",
        ]
        .get(tag as usize)?;
        let url = &*request.byte_add(host.layout.url).cast::<Vector>();
        if url.ptr.is_null() || url.len > 8192 {
            return None;
        }
        let url = std::str::from_utf8(std::slice::from_raw_parts(url.ptr, url.len)).ok()?;
        let path = if url.starts_with('/') {
            url
        } else {
            let rest = url.strip_prefix("https://")?;
            let (host, path) = rest.split_once('/')?;
            if !["discord.com", "canary.discord.com", "ptb.discord.com"].contains(&host) {
                return None;
            }
            &url[url.len() - path.len() - 1..]
        };
        let path = if let Some(rest) = path.strip_prefix("/api/") {
            if rest.starts_with('v') {
                rest.find('/').map(|at| &rest[at..]).unwrap_or(path)
            } else {
                &path[4..]
            }
        } else {
            path
        };
        Some((method.to_owned(), path.to_owned()))
    })
    .ok()
    .flatten();
    if let Some((method, path)) = &observed {
        let _ = std::panic::catch_unwind(|| unsafe {
            if request.is_null() {
                return;
            }
            let body = request.byte_add(host.layout.body).cast::<u64>();
            if body.read() != 1 {
                return;
            }
            let bytes = &mut *body.add(1).cast::<Bytes>();
            if bytes.ptr.is_null() || bytes.len > 1024 * 1024 {
                return;
            }
            let raw = std::slice::from_raw_parts(bytes.ptr, bytes.len);
            if let Some(rewritten) = crate::plugin_events::request_cleanup(method, path, raw) {
                let old = std::ptr::replace(bytes, owned(rewritten));
                ((*old.vtable).drop)(old.data.load(Ordering::Relaxed), old.ptr, old.len);
            }
        });
    }
    let future = unsafe { (host.send)(this, request) };
    if let Some(observed) = observed
        && !future.data.is_null()
    {
        let mut requests = REQUESTS.lock().unwrap_or_else(|e| e.into_inner());
        if requests.len() >= 4096 {
            requests.clear();
        }
        requests.insert(future.data as usize, observed);
    }
    future
}

unsafe fn poll(future: *mut c_void, cx: *mut c_void) -> PollResult {
    let host = HOST.get().unwrap();
    let mut result = unsafe { (host.poll)(future, cx) };
    if result.headers[0] == u64::MAX - 1 {
        return result;
    }
    let request = REQUESTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(future as usize));
    if result.headers[0] == u64::MAX {
        return result;
    }
    if let Some((method, path)) = request {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            if result.body.len > 8 * 1024 * 1024
                || result.body.len > result.body.cap
                || result.body.ptr.is_null()
            {
                return;
            }
            let bytes = std::slice::from_raw_parts(result.body.ptr, result.body.len);
            let Some((status, body)) =
                crate::plugin_events::http_response(&method, &path, result.status, bytes)
            else {
                return;
            };
            if body.len() > 8 * 1024 * 1024 {
                return;
            }
            if body.len() <= result.body.cap {
                std::ptr::copy_nonoverlapping(body.as_ptr(), result.body.ptr, body.len());
                result.body.len = body.len();
            } else {
                let ptr = (host.alloc)(body.len(), 1);
                if ptr.is_null() {
                    return;
                }
                std::ptr::copy_nonoverlapping(body.as_ptr(), ptr, body.len());
                if result.body.cap > 0 {
                    (host.dealloc)(result.body.ptr, result.body.cap, 1);
                }
                result.body = Vector {
                    cap: body.len(),
                    ptr,
                    len: body.len(),
                };
            }
            result.status = status;
        }));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_bytes_can_be_cloned_and_released() {
        let original = owned(b"message body".to_vec());
        let cloned = unsafe { clone_bytes(&original.data, original.ptr, original.len) };
        unsafe {
            drop_bytes(
                original.data.load(Ordering::Relaxed),
                original.ptr,
                original.len,
            );
        }
        assert_eq!(
            unsafe { std::slice::from_raw_parts(cloned.ptr, cloned.len) },
            b"message body"
        );
        unsafe {
            drop_bytes(cloned.data.load(Ordering::Relaxed), cloned.ptr, cloned.len);
        }
    }
}
