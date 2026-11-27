//! Workshop WASM host, usable standalone or from the Linux Phoenix loader.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod access;
#[cfg(target_os = "linux")]
mod loader;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod messages;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod outgoing;
mod plugin_assets;
mod plugin_context;
mod plugin_events;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod plugin_images;
mod plugin_network;
mod plugins;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod settings;
use anyhow::{Context, Result, bail};
use wasmi::{Caller, Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

const FUEL: u64 = 1_000_000;
const MAX_LOG_BYTES: usize = 4096;
const MAX_LOGS: usize = 100;

struct State {
    limits: StoreLimits,
    logs: Vec<String>,
}

/// Run one ABI v1 plugin with no WASI, filesystem, networking, or native pointers.
/// Each invocation gets a fresh store and a bounded instruction budget.
pub fn run_plugin(bytes: &[u8]) -> Result<Vec<String>> {
    if bytes.len() > 4 * 1024 * 1024 {
        bail!("plugin exceeds the 4 MiB module limit");
    }
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, bytes).context("invalid WebAssembly module")?;
    let mut store = Store::new(
        &engine,
        State {
            limits: StoreLimitsBuilder::new()
                .memory_size(16 * 1024 * 1024)
                .memories(1)
                .tables(1)
                .table_elements(4096)
                .instances(1)
                .build(),
            logs: Vec::new(),
        },
    );
    store.limiter(|state| &mut state.limits);
    store.set_fuel(FUEL)?;
    let mut linker = Linker::new(&engine);
    linker.func_wrap(
        "workshop",
        "log",
        |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> i32 {
            let (Ok(ptr), Ok(len)) = (usize::try_from(ptr), usize::try_from(len)) else {
                return -1;
            };
            if len > MAX_LOG_BYTES || caller.data().logs.len() >= MAX_LOGS {
                return -1;
            }
            let Some(memory) = caller
                .get_export("memory")
                .and_then(|export| export.into_memory())
            else {
                return -1;
            };
            let Some(end) = ptr.checked_add(len) else {
                return -1;
            };
            let Some(bytes) = memory.data(&caller).get(ptr..end) else {
                return -1;
            };
            let Ok(message) = std::str::from_utf8(bytes) else {
                return -1;
            };
            let message = message.to_owned();
            caller.data_mut().logs.push(message);
            0
        },
    )?;
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .context("plugin initialization failed")?;
    let version = instance
        .get_typed_func::<(), i32>(&store, "workshop_abi_version")?
        .call(&mut store, ())?;
    if version != 1 {
        bail!("unsupported Workshop ABI version {version}");
    }
    instance
        .get_typed_func::<(), ()>(&store, "workshop_start")?
        .call(&mut store, ())
        .context("plugin trapped or exhausted its instruction budget")?;
    Ok(store.into_data().logs)
}

/// Experimental text transform ABI. The plugin returns output in its input buffer.
#[derive(Clone, Copy)]
#[repr(i32)]
pub enum MessageEvent {
    Send = 0,
    Receive = 1,
    Update = 2,
}

pub fn transform_text(bytes: &[u8], text: &str) -> Result<String> {
    transform_message(bytes, MessageEvent::Send, text)
}

pub fn transform_message(bytes: &[u8], event: MessageEvent, text: &str) -> Result<String> {
    if bytes.len() > 4 * 1024 * 1024 || text.len() > 65536 {
        bail!("plugin or text exceeds transform limits");
    }
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, bytes)?;
    let mut store = Store::new(
        &engine,
        StoreLimitsBuilder::new()
            .memory_size(16 * 1024 * 1024)
            .memories(1)
            .tables(1)
            .table_elements(4096)
            .instances(1)
            .build(),
    );
    store.limiter(|limits| limits);
    store.set_fuel(10_000_000)?;
    let instance = Linker::new(&engine).instantiate_and_start(&mut store, &module)?;
    if instance
        .get_typed_func::<(), i32>(&store, "workshop_abi_version")?
        .call(&mut store, ())?
        != 1
    {
        bail!("unsupported plugin ABI");
    }
    let memory = instance
        .get_memory(&store, "memory")
        .context("plugin memory missing")?;
    let pointer = instance
        .get_typed_func::<(), i32>(&store, "workshop_buffer")?
        .call(&mut store, ())?;
    let pointer = usize::try_from(pointer).context("invalid plugin buffer")?;
    memory.write(&mut store, pointer, text.as_bytes())?;
    let length = if instance.get_export(&store, "workshop_on_message").is_some() {
        instance
            .get_typed_func::<(i32, i32), i32>(&store, "workshop_on_message")?
            .call(&mut store, (event as i32, text.len() as i32))?
    } else {
        instance
            .get_typed_func::<i32, i32>(&store, "workshop_transform")?
            .call(&mut store, text.len() as i32)?
    };
    let length = usize::try_from(length).context("plugin transform failed")?;
    if length > 65536 {
        bail!("invalid transform output length");
    }
    let mut output = vec![0; length];
    memory.read(&store, pointer, &mut output)?;
    String::from_utf8(output).context("plugin returned invalid UTF-8")
}

/// Developer entry point for inspecting a plugin's generic event responses.
pub fn dispatch_plugin_event(id: &str, event: serde_json::Value) -> Result<serde_json::Value> {
    plugin_assets::start()?;
    let plugin = plugins::list()
        .iter()
        .find(|plugin| plugin.id == id)
        .context("unknown plugin")?;
    if let serde_json::Value::Array(events) = event {
        let results = events
            .into_iter()
            .map(|event| plugin_events::dispatch(plugin, event))
            .collect::<Result<Vec<_>>>()?;
        Ok(results.into())
    } else {
        plugin_events::dispatch(plugin, event)
    }
}

/// Resolve and materialize one plugin image for native-adapter diagnostics.
pub fn prepare_plugin_image(id: &str, url: &str, path: &std::path::Path) -> Result<()> {
    let replacement = dispatch_plugin_event(id, serde_json::json!({"type":"image","url":url}))?;
    let url = replacement
        .as_str()
        .context("plugin did not rewrite image")?;
    if !plugin_assets::owns(url) {
        anyhow::bail!("diagnostic expects a prepared local image");
    }
    let output = std::process::Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--noproxy",
            "*",
            "--max-time",
            "30",
            "--",
            url,
        ])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("prepared image request failed");
    }
    std::fs::write(path, output.stdout)?;
    Ok(())
}
