//! Generic persistent WASM events and capability-scoped host services.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use wasmi::{
    Caller, Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder,
    TypedFunc,
};
const MAX: usize = 8 * 1024 * 1024;
struct State {
    id: String,
    hosts: Vec<String>,
    image_hosts: Vec<String>,
    background: bool,
    limits: StoreLimits,
}
struct Vm {
    store: Store<State>,
    memory: Memory,
    buffer: TypedFunc<(), i32>,
    event: TypedFunc<i32, i32>,
}
static VMS: LazyLock<Mutex<BTreeMap<String, Arc<Mutex<Vm>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
fn vm(plugin: &crate::plugins::Plugin) -> Result<Arc<Mutex<Vm>>> {
    let mut vms = VMS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(vm) = vms.get(&plugin.id) {
        return Ok(vm.clone());
    }
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, &plugin.bytes)?;
    if !module
        .exports()
        .any(|export| export.name() == "workshop_event")
    {
        bail!("plugin has no general event handler");
    }
    let mut store = Store::new(
        &engine,
        State {
            id: plugin.id.clone(),
            hosts: plugin.network_hosts.clone(),
            image_hosts: plugin.image_hosts.clone(),
            background: false,
            limits: StoreLimitsBuilder::new()
                .memory_size(128 * 1024 * 1024)
                .memories(1)
                .tables(1)
                .table_elements(4096)
                .instances(1)
                .build(),
        },
    );
    store.limiter(|state| &mut state.limits);
    store.set_fuel(200_000_000)?;
    let mut linker = Linker::new(&engine);
    linker.func_wrap(
        "workshop",
        "call",
        |mut caller: Caller<'_, State>, ptr: i32, len: i32, out: i32, cap: i32| -> i32 {
            let result = (|| -> Result<i32> {
                let ptr = usize::try_from(ptr)?;
                let len = usize::try_from(len)?;
                let out = usize::try_from(out)?;
                let cap = usize::try_from(cap)?;
                if len > MAX || cap > MAX {
                    bail!("host call too large");
                }
                let memory = caller
                    .get_export("memory")
                    .and_then(|m| m.into_memory())
                    .context("missing memory")?;
                let mut input = vec![0; len];
                memory.read(&caller, ptr, &mut input)?;
                let request: Value = serde_json::from_slice(&input)?;
                let result = service(caller.data(), request)?;
                let output = serde_json::to_vec(&result)?;
                if output.len() > cap {
                    bail!("host response too large");
                }
                memory.write(&mut caller, out, &output)?;
                Ok(output.len() as i32)
            })();
            result.unwrap_or(-1)
        },
    )?;
    let instance = linker.instantiate_and_start(&mut store, &module)?;
    if instance
        .get_typed_func::<(), i32>(&store, "workshop_abi_version")?
        .call(&mut store, ())?
        != 1
    {
        bail!("unsupported ABI");
    }
    let memory = instance
        .get_memory(&store, "memory")
        .context("missing memory")?;
    let buffer = instance.get_typed_func::<(), i32>(&store, "workshop_buffer")?;
    let event = instance.get_typed_func::<i32, i32>(&store, "workshop_event")?;
    let vm = Arc::new(Mutex::new(Vm {
        store,
        memory,
        buffer,
        event,
    }));
    vms.insert(plugin.id.clone(), vm.clone());
    Ok(vm)
}
pub fn dispatch(plugin: &crate::plugins::Plugin, event: Value) -> Result<Value> {
    let vm = vm(plugin)?;
    let mut vm = vm.try_lock().map_err(|_| anyhow::anyhow!("plugin busy"))?;
    let input = serde_json::to_vec(&event)?;
    if input.len() > MAX {
        bail!("event too large");
    }
    vm.store.data_mut().background = event["type"] == "tick";
    vm.store.set_fuel(if event["type"] == "tick" {
        5_000_000_000
    } else {
        200_000_000
    })?;
    let buffer = vm.buffer;
    let pointer = usize::try_from(buffer.call(&mut vm.store, ())?)?;
    let memory = vm.memory;
    memory.write(&mut vm.store, pointer, &input)?;
    let event = vm.event;
    let returned = event.call(&mut vm.store, input.len() as i32);
    let len = match returned {
        Ok(length) => usize::try_from(length)?,
        Err(error) => {
            VMS.lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&plugin.id);
            return Err(error.into());
        }
    };
    if len > MAX {
        bail!("plugin output too large");
    }
    let mut output = vec![0; len];
    memory.read(&vm.store, pointer, &mut output)?;
    serde_json::from_slice(&output).context("invalid plugin output")
}
fn directory(id: &str) -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".local/share")
        });
    let id: String = id.bytes().map(|byte| format!("{byte:02x}")).collect();
    base.join("workshop-client/plugins").join(id)
}
fn service(state: &State, request: Value) -> Result<Value> {
    match request["op"].as_str().unwrap_or("") {
        "image_url" => Ok(json!(crate::plugin_assets::register(
            &state.id,
            request["url"].as_str().context("image URL missing")?,
            request["format"].as_str().unwrap_or("png"),
            request["width"].as_u64().unwrap_or(64).try_into()?,
            request["height"].as_u64().unwrap_or(64).try_into()?,
            &state.image_hosts
        )?)),
        "time" => Ok(json!(
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()
        )),
        "settings" => Ok(json!(
            crate::settings::config::get()
                .plugin_settings
                .get(&state.id)
                .cloned()
                .unwrap_or_default()
        )),
        "storage_get" | "storage_set" => {
            let key = request["key"].as_str().context("storage key")?;
            if key.is_empty()
                || key.len() > 64
                || !key
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            {
                bail!("invalid storage key");
            }
            let path = directory(&state.id).join(format!("{key}.json"));
            if request["op"] == "storage_get" {
                use std::io::Read;
                let Ok(file) = std::fs::File::open(path) else {
                    return Ok(Value::Null);
                };
                let mut bytes = Vec::new();
                file.take(MAX as u64 + 1).read_to_end(&mut bytes)?;
                if bytes.len() > MAX {
                    bail!("cache too large");
                }
                Ok(serde_json::from_slice(&bytes)?)
            } else {
                let bytes = serde_json::to_vec(&request["value"])?;
                if bytes.len() > MAX {
                    bail!("cache too large");
                }
                std::fs::create_dir_all(path.parent().unwrap())?;
                let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
                std::fs::write(&tmp, bytes)?;
                std::fs::rename(tmp, path)?;
                Ok(json!(true))
            }
        }
        "http_get" => {
            if !state.background {
                bail!("network calls require a background tick");
            }
            let url = request["url"].as_str().context("URL")?;
            let host = url
                .strip_prefix("https://")
                .context("HTTPS required")?
                .split('/')
                .next()
                .unwrap_or("");
            if !state.hosts.iter().any(|allowed| allowed == host) {
                bail!("plugin lacks permission for HTTP host");
            }
            let output = std::process::Command::new("curl")
                .args([
                    "--silent",
                    "--show-error",
                    "--proto",
                    "=https",
                    "--max-time",
                    "20",
                    "--max-filesize",
                    "8388608",
                    "--write-out",
                    "\n%{http_code}",
                    "--",
                    url,
                ])
                .output()?;
            if !output.status.success() || output.stdout.len() > MAX {
                bail!("plugin HTTP request failed");
            }
            let body = String::from_utf8(output.stdout)?;
            let (body, status) = body.rsplit_once('\n').context("HTTP status missing")?;
            Ok(json!({"status":status.parse::<u16>()?,"body":body}))
        }
        _ => bail!("unknown host operation"),
    }
}
pub fn http_response(method: &str, path: &str, status: u16, body: &[u8]) -> Option<(u16, Vec<u8>)> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    let mut status = status;
    let mut changed = false;
    let config = crate::settings::config::get();
    for plugin in crate::plugins::list()
        .iter()
        .filter(|p| crate::plugins::enabled(p, &config))
    {
        let Ok(result) = dispatch(
            plugin,
            json!({"type":"http_response","method":method,"path":path,"status":status,"body":value}),
        ) else {
            continue;
        };
        if result["body"].is_object() || result["body"].is_array() {
            if let Some(code) = result["status"]
                .as_u64()
                .filter(|code| (100..=599).contains(code))
            {
                status = code as u16;
            }
            eprintln!("[workshop] plugin {} transformed HTTP response", plugin.id);
            value = result["body"].clone();
            changed = true;
        }
    }
    changed.then(|| {
        (
            status,
            serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec()),
        )
    })
}
pub fn image(url: &str) -> Option<String> {
    let config = crate::settings::config::get();
    for plugin in crate::plugins::list()
        .iter()
        .filter(|p| crate::plugins::enabled(p, &config))
    {
        if let Ok(Value::String(url)) = dispatch(plugin, json!({"type":"image","url":url}))
            && (url.starts_with("https://") || crate::plugin_assets::owns(&url))
            && url.len() <= 8192
        {
            return Some(url);
        }
    }
    None
}
pub fn supports(plugin: &crate::plugins::Plugin) -> bool {
    vm(plugin).is_ok()
}
pub fn start() {
    if let Err(error) = crate::plugin_assets::start() {
        eprintln!("[workshop] image service: {error:#}");
    }
    // Initialize metadata before background HTTP work can make a plugin busy.
    let _ = schemas();
    let _ = std::thread::Builder::new()
        .name("workshop-plugin-ticks".into())
        .spawn(|| {
            loop {
                let config = crate::settings::config::get();
                for plugin in crate::plugins::list()
                    .iter()
                    .filter(|p| crate::plugins::enabled(p, &config))
                {
                    if let Err(error) = dispatch(plugin, json!({"type":"tick"}))
                        && !plugin.network_hosts.is_empty()
                    {
                        eprintln!("[workshop] plugin {} tick: {error:#}", plugin.id);
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
        });
}
#[derive(Clone)]
pub struct Setting {
    pub plugin_id: String,
    pub key: String,
    pub title: String,
    pub description: String,
    pub default: bool,
}
pub fn schemas() -> &'static [Setting] {
    static SCHEMAS: std::sync::OnceLock<Vec<Setting>> = std::sync::OnceLock::new();
    SCHEMAS.get_or_init(|| {
        let mut settings = Vec::new();
        for plugin in crate::plugins::list() {
            if let Ok(Value::Array(schema)) = dispatch(plugin, json!({"type":"settings"})) {
                for setting in schema.into_iter().take(32) {
                    if let (Some(key), Some(title), Some(description), Some(default)) = (
                        setting["key"].as_str(),
                        setting["title"].as_str(),
                        setting["description"].as_str(),
                        setting["default"].as_bool(),
                    ) {
                        settings.push(Setting {
                            plugin_id: plugin.id.clone(),
                            key: key.into(),
                            title: title.into(),
                            description: description.into(),
                            default,
                        });
                    }
                }
            }
        }
        settings
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> State {
        State {
            id: "test".into(),
            hosts: Vec::new(),
            image_hosts: Vec::new(),
            background: true,
            limits: StoreLimitsBuilder::new().build(),
        }
    }
    #[test]
    fn rejects_undeclared_hosts_and_storage_traversal() {
        assert!(
            service(
                &state(),
                json!({"op":"http_get","url":"https://example.com/users"})
            )
            .is_err()
        );
        assert!(
            service(
                &state(),
                json!({"op":"storage_get","key":"../other-plugin"})
            )
            .is_err()
        );
        let mut foreground = state();
        foreground.hosts.push("example.com".into());
        foreground.background = false;
        assert!(
            service(
                &foreground,
                json!({"op":"http_get","url":"https://example.com/"})
            )
            .is_err()
        );
    }
}
