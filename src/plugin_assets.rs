//! Generic bounded image conversion/cache, served only on authenticated loopback URLs.
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{IpAddr, TcpListener, TcpStream, ToSocketAddrs},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
struct Asset {
    url: String,
    format: String,
    width: u32,
    height: u32,
    path: PathBuf,
    lock: Mutex<()>,
}
struct Server {
    port: u16,
    token: String,
    assets: Mutex<HashMap<String, Arc<Asset>>>,
}
static SERVER: OnceLock<Server> = OnceLock::new();
pub fn start() -> Result<()> {
    if SERVER.get().is_some() {
        return Ok(());
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let mut random = [0; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let token = random.iter().map(|byte| format!("{byte:02x}")).collect();
    SERVER
        .set(Server {
            port: listener.local_addr()?.port(),
            token,
            assets: Mutex::new(HashMap::new()),
        })
        .map_err(|_| anyhow::anyhow!("asset server already initialized"))?;
    std::thread::Builder::new()
        .name("workshop-assets".into())
        .spawn(move || {
            static ACTIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            for stream in listener.incoming().flatten() {
                if ACTIVE.fetch_add(1, std::sync::atomic::Ordering::AcqRel) >= 8 {
                    ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                    continue;
                }
                let _ = std::thread::Builder::new()
                    .name("workshop-image".into())
                    .spawn(move || {
                        let _ = serve(stream);
                        ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                    });
            }
        })?;
    Ok(())
}
pub fn register(
    id: &str,
    url: &str,
    format: &str,
    width: u32,
    height: u32,
    allowed: &[String],
) -> Result<String> {
    validate_url(url, allowed)?;
    if !["png", "gif", "mp4"].contains(&format)
        || !(1..=128).contains(&width)
        || !(1..=128).contains(&height)
    {
        bail!("unsupported image conversion");
    }
    let server = SERVER.get().context("image service unavailable")?;
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (id, url, format, width, height).hash(&mut hash);
    let key = format!("{:016x}.{format}", hash.finish());
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        })
        .join("workshop-client/images");
    let mut assets = server.assets.lock().unwrap_or_else(|e| e.into_inner());
    if assets.len() >= 4096 && !assets.contains_key(&key) {
        bail!("image registry full");
    }
    assets.entry(key.clone()).or_insert_with(|| {
        Arc::new(Asset {
            url: url.into(),
            format: format.into(),
            width,
            height,
            path: base.join(&key),
            lock: Mutex::new(()),
        })
    });
    Ok(format!(
        "http://127.0.0.1:{}/{}/{}",
        server.port, server.token, key
    ))
}
pub fn owns(url: &str) -> bool {
    SERVER.get().is_some_and(|server| {
        url.starts_with(&format!(
            "http://127.0.0.1:{}/{}/",
            server.port, server.token
        ))
    })
}
fn validate_url<'a>(url: &'a str, allowed: &[String]) -> Result<&'a str> {
    let authority = url
        .strip_prefix("https://")
        .context("image HTTPS required")?
        .split('/')
        .next()
        .unwrap_or("");
    if authority.is_empty()
        || authority.contains(['@', ':'])
        || !authority
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
        || authority == "localhost"
        || authority.ends_with(".local")
        || authority.parse::<IpAddr>().is_ok()
    {
        bail!("invalid image host");
    }
    if !allowed.iter().any(|host| host == "*" || host == authority) {
        bail!("image host permission missing");
    }
    Ok(authority)
}
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && ip.octets()[0] != 0
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
                && ip.to_ipv4_mapped().is_none()
        }
    }
}
fn convert(asset: &Asset) -> Result<Vec<u8>> {
    let _guard = asset.lock.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(bytes) = std::fs::read(&asset.path)
        && !bytes.is_empty()
        && bytes.len() <= 1024 * 1024
    {
        return Ok(bytes);
    }
    let host = validate_url(&asset.url, &["*".into()])?;
    let address = (host, 443)
        .to_socket_addrs()?
        .find(|address| public(address.ip()))
        .context("image host has no public address")?;
    let pin = match address.ip() {
        IpAddr::V4(ip) => format!("{host}:443:{ip}"),
        IpAddr::V6(ip) => format!("{host}:443:[{ip}]"),
    };
    std::fs::create_dir_all(asset.path.parent().unwrap())?;
    let input = asset
        .path
        .with_extension(format!("{}.input", std::process::id()));
    let output = asset
        .path
        .with_extension(format!("{}.tmp.{}", std::process::id(), asset.format));
    let result = (|| -> Result<Vec<u8>> {
        let status = std::process::Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--max-time",
                "12",
                "--max-filesize",
                "8388608",
                "--resolve",
                &pin,
                "--output",
            ])
            .arg(&input)
            .arg("--")
            .arg(&asset.url)
            .status()?;
        if !status.success() {
            bail!("image download failed");
        }
        let conversion = if asset.format == "mp4" {
            std::process::Command::new("timeout").args(["15","ffmpeg","-v","error","-nostdin","-threads","1","-filter_threads","1","-i"]).arg(&input)
                .args(["-an","-vf",&format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=0x202024,format=yuv420p",asset.width,asset.height,asset.width,asset.height),"-t","12","-frames:v","120","-c:v","libx264","-preset","veryfast","-movflags","+faststart","-y"]).arg(&output).output()?
        } else {
            let source = format!(
                "{}{}",
                input.display(),
                if asset.format == "gif" {
                    "[0-119]"
                } else {
                    "[0]"
                }
            );
            let mut command = std::process::Command::new("timeout");
            command.args([
                "15", "magick", "-limit", "memory", "64MiB", "-limit", "disk", "64MiB", "-limit",
                "area", "4MP",
            ]);
            command.arg(source);
            if asset.format == "gif" {
                command.arg("-coalesce");
            }
            command.args([
                "-resize",
                &format!("{}x{}", asset.width, asset.height),
                "-strip",
            ]);
            if asset.format == "gif" {
                command.args(["-layers", "Optimize"]);
            }
            command.arg(&output).output()?
        };
        if !conversion.status.success() {
            bail!("image conversion failed");
        }
        let bytes = std::fs::read(&output)?;
        if bytes.is_empty() || bytes.len() > 1024 * 1024 {
            bail!("converted image too large");
        }
        std::fs::rename(&output, &asset.path)?;
        Ok(bytes)
    })();
    let _ = std::fs::remove_file(input);
    let _ = std::fs::remove_file(output);
    result
}
fn serve(mut stream: TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut input = Vec::new();
    while input.len() < 4096 && !input.windows(2).any(|bytes| bytes == b"\r\n") {
        let mut chunk = [0; 512];
        let length = stream.read(&mut chunk)?;
        if length == 0 {
            break;
        }
        input.extend_from_slice(&chunk[..length]);
    }
    let request = std::str::from_utf8(&input)?;
    let server = SERVER.get().context("server missing")?;
    let (method, path) = request
        .lines()
        .next()
        .and_then(|line| line.split_once(' '))
        .and_then(|(method, rest)| rest.split_once(' ').map(|(path, _)| (method, path)))
        .unwrap_or(("", ""));
    if !["GET", "HEAD"].contains(&method) {
        stream.write_all(
            b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
        return Ok(());
    }
    let prefix = format!("/{}/", server.token);
    let key = path.strip_prefix(&prefix).unwrap_or("");
    let asset = server
        .assets
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .cloned();
    if let Some(asset) = asset {
        match convert(&asset) {
            Ok(body) => {
                let range = request.lines().skip(1).find_map(|line| {
                    line.split_once(':')
                        .filter(|(key, _)| key.eq_ignore_ascii_case("range"))
                        .map(|(_, value)| value.trim())
                });
                let selected = range.and_then(|range| byte_range(range, body.len()));
                if range.is_some() && selected.is_none() {
                    stream.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
                    return Ok(());
                }
                let (start, end) = selected.unwrap_or((0, body.len() - 1));
                let content_type = if asset.format == "mp4" {
                    "video/mp4"
                } else if asset.format == "gif" {
                    "image/gif"
                } else {
                    "image/png"
                };
                write!(
                    stream,
                    "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n",
                    if selected.is_some() {
                        "206 Partial Content"
                    } else {
                        "200 OK"
                    },
                    content_type,
                    end - start + 1
                )?;
                if selected.is_some() {
                    write!(
                        stream,
                        "Content-Range: bytes {start}-{end}/{}\r\n",
                        body.len()
                    )?;
                }
                stream.write_all(b"\r\n")?;
                if method == "GET" {
                    stream.write_all(&body[start..=end])?;
                }
            }
            Err(error) => {
                eprintln!("[workshop] plugin image preparation failed: {error:#}");
                stream.write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )?;
            }
        }
    } else {
        stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
    }
    Ok(())
}
fn byte_range(value: &str, len: usize) -> Option<(usize, usize)> {
    if len == 0 {
        return None;
    }
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<usize>().ok()?;
        return (suffix > 0).then_some((len.saturating_sub(suffix), len - 1));
    }
    let start = start.parse::<usize>().ok()?;
    let end = if end.is_empty() {
        len - 1
    } else {
        end.parse::<usize>().ok()?.min(len - 1)
    };
    (start <= end && start < len).then_some((start, end))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supports_video_byte_ranges() {
        assert_eq!(byte_range("bytes=0-31", 100), Some((0, 31)));
        assert_eq!(byte_range("bytes=80-", 100), Some((80, 99)));
        assert_eq!(byte_range("bytes=-12", 100), Some((88, 99)));
        assert!(byte_range("bytes=200-", 100).is_none());
    }
    #[test]
    fn restricts_origins_and_loopback() {
        assert!(validate_url("https://example.com/a.png", &["*".into()]).is_ok());
        for url in [
            "http://example.com/a.png",
            "https://127.0.0.1/a",
            "https://localhost/a",
            "https://a@b/a",
        ] {
            assert!(validate_url(url, &["*".into()]).is_err());
        }
        assert!(!public("192.168.1.1".parse().unwrap()));
    }
}
