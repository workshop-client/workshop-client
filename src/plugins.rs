//! Shared catalog for startup, message dispatch, and settings UI.
use std::{io::Read, sync::LazyLock};
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub description: String,
    pub bytes: Vec<u8>,
    pub network_hosts: Vec<String>,
    pub image_hosts: Vec<String>,
}
fn bundled(manifest: &str, bytes: &[u8]) -> Plugin {
    #[derive(serde::Deserialize)]
    struct Manifest {
        id: String,
        name: String,
        description: String,
        network_hosts: Vec<String>,
        #[serde(default)]
        image_hosts: Vec<String>,
    }
    let manifest: Manifest =
        serde_json::from_str(manifest).expect("bundled manifest is validated during development");
    Plugin {
        id: manifest.id,
        name: manifest.name,
        description: manifest.description,
        bytes: bytes.to_vec(),
        network_hosts: manifest.network_hosts,
        image_hosts: manifest.image_hosts,
    }
}
static PLUGINS: LazyLock<Vec<Plugin>> = LazyLock::new(|| {
    let mut plugins = vec![
        bundled(
            include_str!("../plugins/clear-urls/manifest.json"),
            include_bytes!("../plugins/clear-urls/plugin.wasm"),
        ),
        bundled(
            include_str!("../plugins/global-badges/manifest.json"),
            include_bytes!("../plugins/global-badges/plugin.wasm"),
        ),
    ];
    if let Some(path) = std::env::var_os("WORKSHOP_PLUGIN") {
        let path = std::path::PathBuf::from(path);
        let result = (|| -> std::io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            std::fs::File::open(&path)?
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            Ok(bytes)
        })();
        match result {
            Ok(bytes) => plugins.push(Plugin {
                id: format!(
                    "file:{}",
                    path.canonicalize()
                        .unwrap_or_else(|_| path.clone())
                        .display()
                ),
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                description: "Custom WebAssembly plugin.".into(),
                bytes,
                network_hosts: Vec::new(),
                image_hosts: Vec::new(),
            }),
            Err(error) => eprintln!("[workshop] custom plugin could not load: {error}"),
        }
    }
    plugins
});
pub fn list() -> &'static [Plugin] {
    &PLUGINS
}
pub fn enabled(plugin: &Plugin, config: &crate::settings::config::Config) -> bool {
    config.plugins_enabled
        && config
            .plugin_enabled
            .get(&plugin.id)
            .copied()
            .unwrap_or(true)
}
pub fn transform(event: crate::MessageEvent, text: &str) -> String {
    transform_with_config(event, text, &crate::settings::config::get())
}

pub(crate) fn transform_with_config(
    event: crate::MessageEvent,
    text: &str,
    config: &crate::settings::config::Config,
) -> String {
    let mut result = text.to_owned();
    for plugin in list().iter().filter(|plugin| enabled(plugin, config)) {
        if crate::plugin_events::supports(plugin) {
            continue;
        }
        if let Ok(next) = crate::transform_message(&plugin.bytes, event, &result) {
            result = next;
        }
    }
    result
}
