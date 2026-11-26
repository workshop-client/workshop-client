use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{LazyLock, Mutex},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub plugins_enabled: bool,
    pub plugin_enabled: std::collections::BTreeMap<String, bool>,
    pub plugin_settings:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, bool>>,
    pub plugin_logging: bool,
    pub access_override: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            plugins_enabled: true,
            plugin_enabled: Default::default(),
            plugin_settings: Default::default(),
            plugin_logging: true,
            access_override: true,
        }
    }
}
fn path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
        .join("workshop-client/settings.json")
}
static CONFIG: LazyLock<Mutex<Config>> = LazyLock::new(|| {
    Mutex::new(
        std::fs::read(path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default(),
    )
});
pub fn get() -> Config {
    CONFIG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
pub fn set(index: u64, checked: bool) -> anyhow::Result<()> {
    let mut config = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    let mut next = config.clone();
    match index {
        0 => next.plugins_enabled = checked,
        1 => next.plugin_logging = checked,
        2 => next.access_override = checked,
        index if index >= 1000 => {
            let setting = crate::plugin_events::schemas()
                .get((index - 1000) as usize)
                .ok_or_else(|| anyhow::anyhow!("unknown plugin setting"))?;
            next.plugin_settings
                .entry(setting.plugin_id.clone())
                .or_default()
                .insert(setting.key.clone(), checked);
        }
        index if index >= 100 => {
            let plugin = crate::plugins::list()
                .get((index - 100) as usize)
                .ok_or_else(|| anyhow::anyhow!("unknown plugin"))?;
            next.plugin_enabled.insert(plugin.id.clone(), checked);
        }
        _ => anyhow::bail!("unknown setting"),
    }
    let path = path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, serde_json::to_vec_pretty(&next)?)?;
    std::fs::rename(temporary, path)?;
    *config = next;
    Ok(())
}
