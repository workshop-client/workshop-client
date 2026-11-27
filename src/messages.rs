//! Message receive adapter. Transform failures preserve the original event.
use phoenix_hook::image::Image;
use phoenix_sigs::{
    GATEWAY_HANDLER_FIELD, GatewayEvent, GatewayHandler, HandlerResult, RawStr,
    registry::GatewayRegistry,
};
use std::{ffi::c_void, sync::OnceLock};
static CREATE: OnceLock<GatewayHandler> = OnceLock::new();
static READY: OnceLock<GatewayHandler> = OnceLock::new();
static UPDATE: OnceLock<GatewayHandler> = OnceLock::new();
pub fn install() -> anyhow::Result<()> {
    crate::access::supported_build()?;
    let image = unsafe { Image::loaded() }
        .into_iter()
        .find(|i| i.file_name() == "discord-phoenix")
        .ok_or_else(|| anyhow::anyhow!("Phoenix image missing"))?;
    let registry = GatewayRegistry::locate(&image)?;
    let mut writes = Vec::new();
    for (name, original, detour) in [
        ("MESSAGE_CREATE", &CREATE, created as *const () as usize),
        ("MESSAGE_UPDATE", &UPDATE, updated as *const () as usize),
        ("READY", &READY, ready as *const () as usize),
    ] {
        let entry = registry.entry(
            registry
                .id(name)
                .ok_or_else(|| anyhow::anyhow!("missing {name}"))?,
        );
        original
            .set(unsafe { std::mem::transmute::<usize, GatewayHandler>(entry.handler) })
            .map_err(|_| anyhow::anyhow!("message hooks already installed"))?;
        for slot in &entry.slots {
            writes.push((slot + GATEWAY_HANDLER_FIELD, detour));
        }
    }
    unsafe {
        image.swap_words(&writes)?;
    }
    Ok(())
}
unsafe extern "C" fn created(
    store: *mut c_void,
    event: *const GatewayEvent,
    cx: *mut c_void,
) -> HandlerResult {
    unsafe { receive(CREATE.get().unwrap(), store, event, cx) }
}
unsafe extern "C" fn updated(
    store: *mut c_void,
    event: *const GatewayEvent,
    cx: *mut c_void,
) -> HandlerResult {
    unsafe { receive(UPDATE.get().unwrap(), store, event, cx) }
}
unsafe fn receive(
    original: &GatewayHandler,
    store: *mut c_void,
    event: *const GatewayEvent,
    cx: *mut c_void,
) -> HandlerResult {
    let rewritten = std::panic::catch_unwind(|| -> Option<String> {
        if !crate::settings::config::get().plugins_enabled {
            return None;
        }
        let event = unsafe { &*event };
        if event.data.len > 1024 * 1024 {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(event.data.ptr, event.data.len) };
        rewrite_json(
            if std::ptr::eq(original, CREATE.get().unwrap()) {
                crate::MessageEvent::Receive
            } else {
                crate::MessageEvent::Update
            },
            bytes,
        )
    })
    .ok()
    .flatten();
    if let Some(data) = rewritten {
        let mut copied = unsafe { *event };
        copied.data = RawStr {
            ptr: data.as_ptr(),
            len: data.len(),
        };
        unsafe { original(store, &copied, cx) }
    } else {
        unsafe { original(store, event, cx) }
    }
}

pub(crate) fn rewrite_json(event: crate::MessageEvent, bytes: &[u8]) -> Option<String> {
    rewrite_json_using(bytes, |text| crate::plugins::transform(event, text))
}

fn rewrite_json_using(bytes: &[u8], transform: impl FnOnce(&str) -> String) -> Option<String> {
    let mut message: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let content = message.get("content")?.as_str()?;
    let cleaned = transform(content);
    if cleaned == content {
        return None;
    }
    message["content"] = cleaned.into();
    serde_json::to_string(&message).ok()
}

unsafe extern "C" fn ready(
    store: *mut c_void,
    event: *const GatewayEvent,
    cx: *mut c_void,
) -> HandlerResult {
    let _ = std::panic::catch_unwind(|| unsafe {
        let event = &*event;
        if event.data.len > 8 * 1024 * 1024 {
            return;
        }
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(std::slice::from_raw_parts(
            event.data.ptr,
            event.data.len,
        )) {
            crate::plugin_context::observe(&value["user"]);
        }
    });
    unsafe { READY.get().unwrap()(store, event, cx) }
}
