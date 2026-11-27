//! Removes tracking query parameters without reformatting retained URL bytes.
//! Tracking rules are based on Sunbird Clear URLs by amaanq.
use std::sync::Mutex;

const BUFFER_SIZE: usize = 65_536;
static BUFFER: Mutex<[u8; BUFFER_SIZE]> = Mutex::new([0; BUFFER_SIZE]);

const TRACKING_PARAMETERS: &[&str] = &[
    "ref_src",
    "ref_url",
    "fbclid",
    "gclid",
    "dclid",
    "gbraid",
    "wbraid",
    "msclkid",
    "yclid",
    "twclid",
    "ttclid",
    "igshid",
    "igsh",
    "mc_eid",
    "mc_cid",
    "_hsenc",
    "_hsmi",
    "mkt_tok",
    "oly_anon_id",
    "oly_enc_id",
    "vero_id",
    "wickedid",
    "__s",
];

fn matches_domain(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn decode_parameter_name(name: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(name.len());
    let mut characters = name.bytes();
    while let Some(character) = characters.next() {
        if character == b'%' {
            let high = char::from(characters.next()?).to_digit(16)?;
            let low = char::from(characters.next()?).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(character);
        }
    }
    String::from_utf8(bytes).ok()
}

fn is_tracker(name: &str, host: &str) -> bool {
    let Some(name) = decode_parameter_name(name) else {
        // Preserve malformed escapes instead of risking a meaningful parameter.
        return false;
    };
    let social = ["twitter.com", "x.com", "tiktok.com"]
        .iter()
        .any(|domain| matches_domain(host, domain));
    let youtube = ["youtube.com", "youtu.be"]
        .iter()
        .any(|domain| matches_domain(host, domain));
    match name.as_str() {
        "s" | "t" => social,
        "feature" | "pp" => youtube,
        "si" => {
            youtube
                || ["spotify.com", "soundcloud.com"]
                    .iter()
                    .any(|domain| matches_domain(host, domain))
        }
        _ => name.starts_with("utm_") || TRACKING_PARAMETERS.contains(&name.as_str()),
    }
}

fn clean_url(url: &str) -> String {
    let Some(query_start) = url.find('?') else {
        return url.into();
    };
    let fragment_start = url.find('#').unwrap_or(url.len());
    if fragment_start < query_start {
        return url.into();
    }

    let authority = url
        .split_once("//")
        .map(|(_, rest)| rest)
        .unwrap_or("")
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let query = &url[query_start + 1..fragment_start];
    let kept: Vec<_> = query
        .split('&')
        .filter(|pair| {
            let name = pair.split('=').next().unwrap_or("");
            !pair.is_empty() && !is_tracker(name, &host)
        })
        .collect();
    let query = if kept.is_empty() {
        String::new()
    } else {
        format!("?{}", kept.join("&"))
    };
    format!("{}{query}{}", &url[..query_start], &url[fragment_start..])
}

fn is_link_boundary(character: char) -> bool {
    character.is_whitespace() || matches!(character, '<' | '>' | '(' | ')' | '`' | '|')
}

fn link_end(text: &str, start: usize) -> usize {
    let mut position = start;
    while position < text.len() {
        let character = text[position..].chars().next().unwrap();
        if character == '(' {
            let rest = &text[position + 1..];
            let nested_end = rest.find(is_link_boundary).unwrap_or(rest.len());
            if rest.as_bytes().get(nested_end) == Some(&b')') {
                position += nested_end + 2;
                continue;
            }
        }
        if is_link_boundary(character) {
            break;
        }
        position += character.len_utf8();
    }
    position
}

fn suffix_start(link: &str) -> usize {
    let mut end = link.len();
    loop {
        let remaining = &link[..end];
        if remaining.ends_with("**") || remaining.ends_with("~~") {
            end -= 2;
        } else if remaining
            .chars()
            .last()
            .is_some_and(|c| matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | ']'))
        {
            end -= 1;
        } else {
            return end;
        }
    }
}

pub fn clean_links(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut position = 0;
    while position < text.len() {
        let rest = &text[position..];
        let bytes = rest.as_bytes();
        let is_url = bytes
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"http://"))
            || bytes
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"https://"));
        if is_url {
            let end = link_end(text, position);
            let link = &text[position..end];
            let suffix = suffix_start(link);
            result.push_str(&clean_url(&link[..suffix]));
            result.push_str(&link[suffix..]);
            position = end;
        } else {
            let character = rest.chars().next().unwrap();
            result.push(character);
            position += character.len_utf8();
        }
    }
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn workshop_abi_version() -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "C" fn workshop_start() {}
#[unsafe(no_mangle)]
pub extern "C" fn workshop_buffer() -> *mut u8 {
    BUFFER.lock().unwrap().as_mut_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn workshop_on_message(_event: i32, length: i32) -> i32 {
    let Ok(length) = usize::try_from(length) else {
        return -1;
    };
    if length > BUFFER_SIZE {
        return -1;
    }
    let mut buffer = BUFFER.lock().unwrap();
    let Ok(text) = std::str::from_utf8(&buffer[..length]) else {
        return -1;
    };
    let cleaned = clean_links(text);
    if cleaned.len() > BUFFER_SIZE {
        return -1;
    }
    buffer[..cleaned.len()].copy_from_slice(cleaned.as_bytes());
    cleaned.len() as i32
}
