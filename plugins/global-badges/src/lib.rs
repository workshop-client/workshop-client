//! Global Badges provider. All Equibadges behavior lives in this WASM module.
use serde_json::{Value, json};
use std::sync::{LazyLock, Mutex};
const API: &str = "https://badges.equicord.org/users";
const LIMIT: usize = 8 * 1024 * 1024;
static INPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
struct Cache {
    users: Value,
    refresh_at: u64,
    loaded: bool,
    virtual_ids: std::collections::HashSet<String>,
}
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| {
    Mutex::new(Cache {
        users: json!({}),
        refresh_at: 0,
        loaded: false,
        virtual_ids: Default::default(),
    })
});
#[link(wasm_import_module = "workshop")]
unsafe extern "C" {
    fn call(ptr: *const u8, len: i32, out: *mut u8, capacity: i32) -> i32;
}
fn host(request: Value) -> Value {
    let bytes = serde_json::to_vec(&request).unwrap_or_default();
    let capacity = match request["op"].as_str() {
        Some("storage_get" | "http_get") => LIMIT,
        _ => 65536,
    };
    let mut output = vec![0u8; capacity];
    let length = unsafe {
        call(
            bytes.as_ptr(),
            bytes.len() as i32,
            output.as_mut_ptr(),
            output.len() as i32,
        )
    };
    if length < 0 || length as usize > output.len() {
        return Value::Null;
    }
    serde_json::from_slice(&output[..length as usize]).unwrap_or(Value::Null)
}
#[unsafe(no_mangle)]
pub extern "C" fn workshop_abi_version() -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "C" fn workshop_start() {}
#[unsafe(no_mangle)]
pub extern "C" fn workshop_buffer() -> *mut u8 {
    let mut input = INPUT.lock().unwrap();
    input.resize(LIMIT, 0);
    input.as_mut_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn workshop_event(length: i32) -> i32 {
    if length < 0 || length as usize > LIMIT {
        return -1;
    }
    let mut input = INPUT.lock().unwrap();
    let Ok(request) = serde_json::from_slice::<Value>(&input[..length as usize]) else {
        return -1;
    };
    let result = handle(request);
    let Ok(bytes) = serde_json::to_vec(&result) else {
        return -1;
    };
    if bytes.len() > LIMIT {
        return -1;
    }
    input[..bytes.len()].copy_from_slice(&bytes);
    bytes.len() as i32
}
fn handle(request: Value) -> Value {
    match request["type"].as_str().unwrap_or("") {
        "settings" => json!([
 {"key":"show_source","title":"Badge source","description":"Show the community that provided each badge.","default":true},
 {"key":"badge_vault","title":"BadgeVault badges","description":"Include community badges that any user can create.","default":false}]),
        "initialize" => {
            load_cache();
            Value::Null
        }
        "http_request_cleanup" => clean_badge_save(request),
        "tick" => {
            refresh();
            Value::Null
        }
        "image" => {
            let Some(request_url) = request["url"].as_str() else {
                return Value::Null;
            };
            let Some(uri) = decode_image(request_url) else {
                return Value::Null;
            };
            host(
                json!({"op":"image_url","url":uri,"format":if request_url.split('?').next().is_some_and(|url|url.ends_with(".gif")){"gif"}else{"png"},"width":64,"height":64}),
            )
        }
        "http_response" => response(request),
        _ => Value::Null,
    }
}
fn load_cache() {
    let mut cache = CACHE.lock().unwrap();
    if !cache.loaded {
        let saved = host(json!({"op":"storage_get","key":"badge-cache"}));
        if saved["users"]
            .as_object()
            .is_some_and(|users| !users.is_empty())
        {
            cache.users = saved["users"].clone();
            cache.refresh_at = saved["at"].as_u64().unwrap_or(0) + 21600;
        }
        cache.loaded = true;
    }
}
fn refresh() {
    load_cache();
    let now = host(json!({"op":"time"})).as_u64().unwrap_or(0);
    let mut cache = CACHE.lock().unwrap();
    if now < cache.refresh_at {
        return;
    }
    cache.refresh_at = now + 30;
    let response = host(json!({"op":"http_get","url":API}));
    if response["pending"] == true {
        cache.refresh_at = now + 1;
        return;
    }
    if response["status"] != 200 {
        return;
    }
    let Some(body) = response["body"]
        .as_str()
        .and_then(|body| serde_json::from_str::<Value>(body).ok())
    else {
        return;
    };
    if !body["users"]
        .as_object()
        .is_some_and(|users| !users.is_empty())
    {
        return;
    }
    cache.users = body["users"].clone();
    cache.refresh_at = now + 21600;
    host(json!({"op":"storage_set","key":"badge-cache","value":{"at":now,"users":cache.users}}));
}
fn source(mod_name: &str) -> String {
    match mod_name {
        "vencord" => "Vencord",
        "equicord" => "Equicord",
        "betterdiscord" => "BetterDiscord",
        "aliucord" => "Aliucord",
        "reviewdb" => "ReviewDB",
        "replugged" => "Replugged",
        "enmity" => "Enmity",
        "revenge" => "Revenge",
        "bunny" => "Bunny",
        "vendroidenhanced" => "Vendroid Enhanced",
        "goosemod" => "GooseMod",
        "record" => "ReCord",
        "badgevault" => "BadgeVault",
        _ => return mod_name.to_owned(),
    }
    .to_owned()
}
fn image_key(uri: &str) -> String {
    format!(
        "workshop_global_v3_{}",
        uri.as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
fn decode_image(url: &str) -> Option<String> {
    let tail = url.split_once("/badge-icons/workshop_global_")?.1;
    let tail = tail
        .strip_prefix("v3_")
        .or_else(|| tail.strip_prefix("v2_"))
        .unwrap_or(tail);
    let hex = tail.split('.').next()?;
    if hex.len() > 4096 || !hex.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = hex
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            Some((pair[0] as char).to_digit(16)? as u8 * 16 + (pair[1] as char).to_digit(16)? as u8)
        })
        .collect();
    let result = String::from_utf8(bytes?).ok()?;
    valid_image(&result).then_some(result)
}
fn valid_image(uri: &str) -> bool {
    uri.starts_with("https://")
        && uri.len() < 2048
        && !uri.chars().any(char::is_whitespace)
        && uri[8..]
            .split('/')
            .next()
            .is_some_and(|authority| !authority.is_empty() && !authority.contains('@'))
}
fn badges(entries: &Value, vault: bool, show_source: bool, directory: bool) -> Vec<Value> {
    let Some(entries) = entries.as_array() else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut output = Vec::new();
    for entry in entries {
        let (Some(mod_name), Some(tooltip), Some(uri)) = (
            entry["mod"].as_str(),
            entry["tooltip"].as_str(),
            entry["badge"].as_str(),
        ) else {
            continue;
        };
        let mod_name = mod_name.trim().to_lowercase();
        let tooltip = tooltip.trim();
        if mod_name.is_empty()
            || mod_name == "discord"
            || (mod_name == "badgevault" && !vault)
            || tooltip.is_empty()
        {
            continue;
        }
        let uri = if mod_name == "vencord" && tooltip.eq_ignore_ascii_case("vencord contributor") {
            "https://cdn.discordapp.com/emojis/1562267648857997356.webp?size=128"
        } else {
            uri.trim()
        };
        if !valid_image(uri) || !seen.insert(uri.to_owned()) {
            continue;
        }
        let provider = source(&mod_name);
        let role = tooltip.split(|c: char| !c.is_alphanumeric()).any(|word| {
            [
                "contributor",
                "early",
                "developer",
                "dev",
                "donor",
                "donator",
                "staff",
                "team",
            ]
            .contains(&word.to_lowercase().as_str())
        });
        let name = if role && !tooltip.to_lowercase().starts_with(&provider.to_lowercase()) {
            format!("{provider} {tooltip}")
        } else {
            tooltip.to_owned()
        };
        let hash = uri.bytes().fold(2166136261u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16777619)
        });
        let id = 900000 + (hash % 1000000000);
        CACHE.lock().unwrap().virtual_ids.insert(id.to_string());
        let icon = image_key(uri);
        // Keep the native DTO URL in its expected HTTPS namespace. The media
        // adapter resolves the marker to prepared artwork at fetch time.
        let url = format!("https://cdn.discordapp.com/badge-icons/{icon}.png");
        let prepared =
            host(json!({"op":"image_url","url":uri,"format":"png","width":64,"height":64}));
        if prepared.is_string() {
            host(json!({"op":"image_alias","source":url,"target":prepared}));
        }
        let badge = if directory {
            let mut badge = json!({"badge_id":id,"name":name,"description":if show_source{format!("Badge from {provider}.")}else{String::new()},"rarity":5,"simple_icon_url":url,"simple_icon_raster_url":url,"complex_icon_static_url":url,"is_earnable":true,"owned":true,"hidden":false});
            if show_source {
                badge["info_label"] = provider.into();
            }
            if uri
                .split('?')
                .next()
                .is_some_and(|p| p.ends_with(".gif") || p.ends_with(".webp"))
            {
                let animation = host(
                    json!({"op":"image_url","url":uri,"format":"gif","width":256,"height":256}),
                );
                if animation.is_string() {
                    let animated_url = format!("https://cdn.discordapp.com/badge-icons/{icon}.gif");
                    host(json!({"op":"image_alias","source":animated_url,"target":animation}));
                    badge["simple_icon_url"] = animated_url.clone().into();
                    badge["simple_icon_raster_url"] = animated_url.clone().into();
                    badge["complex_icon_animated_url"] = animated_url.into();
                }
            }
            badge
        } else {
            json!({"id":id.to_string(),"description":name,"icon":icon,"simple_icon_url":url})
        };
        output.push(badge);
        if output.len() == 64 {
            break;
        }
    }
    output
}
fn response(request: Value) -> Value {
    if request["method"] != "GET" {
        return Value::Null;
    }
    let Some(path) = request["path"].as_str() else {
        return Value::Null;
    };
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let Some(rest) = path.strip_prefix("/users/") else {
        return Value::Null;
    };
    let Some((user, endpoint)) = rest.split_once('/') else {
        return Value::Null;
    };
    if !["profile", "badges"].contains(&endpoint) || user.is_empty() {
        return Value::Null;
    }

    let directory = endpoint == "badges";
    let status = request["status"].as_u64().unwrap_or(0);
    let missing = status == 404 && directory;
    if !(missing || (200..300).contains(&status)) {
        return Value::Null;
    }
    let mut body = if missing {
        json!({})
    } else {
        request["body"].clone()
    };
    if !body.is_object() {
        return Value::Null;
    }
    let identity = host(json!({"op":"current_user"}));
    let user = if user == "@me" {
        body["user"]["id"]
            .as_str()
            .or_else(|| body["user_id"].as_str())
            .or_else(|| identity["id"].as_str())
            .unwrap_or("")
    } else {
        user
    };
    if !user.bytes().all(|c| c.is_ascii_digit()) || user.is_empty() {
        return Value::Null;
    }
    let settings = host(json!({"op":"settings"}));
    let entries = CACHE.lock().unwrap().users[user].clone();
    let mut extra = badges(
        &entries,
        settings["badge_vault"].as_bool().unwrap_or(false),
        settings["show_source"].as_bool().unwrap_or(true),
        directory,
    );
    if extra.is_empty() {
        return Value::Null;
    }
    if let Some(existing) = body["badges"].as_array() {
        extra.extend(existing.iter().cloned());
    }
    body["badges"] = extra.into();
    json!({"status":if missing{200}else{status},"body":body})
}
/// Community IDs are display-only. Strip them from badge ordering/hiding saves.
fn clean_badge_save(request: Value) -> Value {
    let path = request["path"].as_str().unwrap_or("");
    if !path.starts_with("/users/@me")
        || !matches!(request["method"].as_str(), Some("PATCH" | "PUT" | "POST"))
    {
        return Value::Null;
    }
    let mut body = request["body"].clone();
    let cache = CACHE.lock().unwrap();
    fn virtual_id(value: &Value, ids: &std::collections::HashSet<String>) -> bool {
        value
            .as_str()
            .map(str::to_owned)
            .or_else(|| value.as_u64().map(|id| id.to_string()))
            .is_some_and(|id| ids.contains(&id))
    }
    fn strip(
        value: &mut Value,
        ids: &std::collections::HashSet<String>,
        badge_field: bool,
    ) -> bool {
        let mut changed = false;
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    changed |= strip(
                        value,
                        ids,
                        badge_field || key.to_ascii_lowercase().contains("badge"),
                    );
                }
            }
            Value::Array(values) => {
                if badge_field {
                    let before = values.len();
                    values.retain(|value| {
                        !virtual_id(value, ids)
                            && !virtual_id(&value["id"], ids)
                            && !virtual_id(&value["badge_id"], ids)
                    });
                    changed |= values.len() != before;
                }
                for value in values {
                    changed |= strip(value, ids, badge_field);
                }
            }
            _ => {}
        }
        changed
    }
    if strip(&mut body, &cache.virtual_ids, false) {
        json!({"body":body})
    } else {
        Value::Null
    }
}
