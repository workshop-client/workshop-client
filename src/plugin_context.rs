//! Client identity context, excluding session credentials.
use serde_json::Value;
use std::sync::{LazyLock, Mutex};
static USER: LazyLock<Mutex<Value>> = LazyLock::new(|| Mutex::new(Value::Null));
fn stored_user(files: &std::path::Path) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Preferences {
        #[serde(rename = "desktop.main_account")]
        account: Option<String>,
    }
    let valid = |id: &str| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit());
    if let Some(id) = std::fs::read(files.join("device_prefs.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Preferences>(&bytes).ok())
        .and_then(|prefs| prefs.account)
        .filter(|id| valid(id))
    {
        return Some(id);
    }
    let mut accounts = std::fs::read_dir(files.join("store"))
        .ok()?
        .flatten()
        .filter(|entry| entry.path().join("cache.sqlite").is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|id| valid(id));
    let first = accounts.next()?;
    accounts.next().is_none().then_some(first)
}
pub fn current_user() -> Value {
    let files = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/share"))
        })
        .map(|root| root.join("discord-phoenix"));
    if let Some(id) = files.and_then(|files| stored_user(&files)) {
        return serde_json::json!({"id":id});
    }
    USER.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
pub fn observe(user: &Value) {
    if user["id"]
        .as_str()
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
    {
        *USER.lock().unwrap_or_else(|e| e.into_inner()) = serde_json::json!({"id":user["id"]});
    }
}
