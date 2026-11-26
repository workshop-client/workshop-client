use anyhow::{Context, Result, bail};
use std::{
    env,
    ffi::OsString,
    fs,
    io::Read,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        #[cfg(feature = "developer-cli")]
        [command, path] if command == "run-plugin" => {
            let mut bytes = Vec::new();
            fs::File::open(path)
                .context("opening plugin")?
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            for line in workshop_client::run_plugin(&bytes)? {
                println!("[workshop] {line}");
            }
        }
        #[cfg(feature = "developer-cli")]
        [command, id, url, path] if command == "plugin-image" => {
            workshop_client::prepare_plugin_image(
                id.to_str().context("plugin id")?,
                url.to_str().context("image URL")?,
                Path::new(path),
            )?;
        }
        #[cfg(feature = "developer-cli")]
        [command, id, event] if command == "plugin-event" => {
            let id = id.to_str().context("plugin id must be UTF-8")?;
            let event = serde_json::from_str(event.to_str().context("event must be UTF-8")?)?;
            println!("{}", workshop_client::dispatch_plugin_event(id, event)?);
        }
        #[cfg(feature = "developer-cli")]
        [command, path, input] if command == "transform" => {
            let mut bytes = Vec::new();
            fs::File::open(path)?
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            println!(
                "{}",
                workshop_client::transform_text(
                    &bytes,
                    input.to_str().context("text must be UTF-8")?
                )?
            );
        }
        [command, path] if command == "inspect" => inspect(Path::new(path))?,
        [] => launch(None, &[])?,
        [command, rest @ ..] if command == "launch" => {
            let (path, forwarded) = match rest.split_first() {
                Some((separator, forwarded)) if separator == "--" => (None, forwarded),
                Some((path, forwarded)) => (
                    Some(Path::new(path)),
                    forwarded
                        .strip_prefix(&[OsString::from("--")])
                        .unwrap_or(forwarded),
                ),
                None => (None, &[][..]),
            };
            launch(path, forwarded)?;
        }
        [command] if command == "--help" || command == "-h" => {
            println!(
                "Workshop Client\n\n  workshop-client [launch [Discord.AppImage] [-- discord arguments...]]\n  workshop-client inspect <Discord.AppImage>\n\nSet WORKSHOP_DISCORD_APPIMAGE to override the default AppImage.\nWorkshop settings and enabled WASM plugins load inside Phoenix."
            );
            #[cfg(feature = "developer-cli")]
            println!("\nDeveloper commands: run-plugin, transform, plugin-event, plugin-image");
        }
        _ => bail!("invalid command; use --help for available commands"),
    }
    Ok(())
}

fn inspect(path: &Path) -> Result<()> {
    let file = fs::File::open(path).context("opening AppImage")?;
    let size = file.metadata()?.len();
    let mut header = [0u8; 12];
    (&file).read_exact(&mut header)?;
    if &header[..4] != b"\x7fELF" || &header[8..11] != b"AI\x02" {
        bail!("expected a type-2 ELF AppImage");
    }
    println!(
        "AppImage: {}\nSize: {size} bytes\nELF class: {}\nAppImage type: 2\nPhoenix hook compatibility: unverified",
        path.display(),
        header[4]
    );
    Ok(())
}

fn launch(path: Option<&Path>, forwarded: &[OsString]) -> Result<()> {
    let path = match path {
        Some(path) => path.to_owned(),
        None => match env::var_os("WORKSHOP_DISCORD_APPIMAGE") {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from(
                env::var_os("HOME").context("HOME is unset; pass an AppImage path")?,
            )
            .join(
                "Applications/discord-desktop-0.1.0-dev.20261123093910.gb71ec80-linux-x64.appimage",
            ),
        },
    };
    let path = path.canonicalize().with_context(|| {
        format!(
            "finding Discord AppImage at {}; pass a path or set WORKSHOP_DISCORD_APPIMAGE",
            path.display()
        )
    })?;
    inspect(&path)?;
    println!("[workshop] Launching Discord Phoenix");
    let loader = env::current_exe()?
        .with_file_name("libworkshop_client.so")
        .canonicalize()
        .context("finding Workshop loader; run cargo build --lib before launching")?;
    // ld.so treats spaces and colons as separators, even when shell-quoted.
    // Stage our library under a stable, separator-free temporary directory if needed.
    let loader = if loader
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .any(|byte| matches!(byte, b' ' | b':' | b'\t' | b'\n'))
    {
        let home = PathBuf::from(env::var_os("HOME").context("HOME is unset")?);
        let directory = home.join(".local/lib/workshop-client");
        fs::create_dir_all(&directory)?;
        let staged = directory.join("libworkshop_client.so");
        let temporary = directory.join(format!("loader-{}.tmp", std::process::id()));
        fs::copy(&loader, &temporary)?;
        fs::rename(temporary, &staged)?;
        staged
    } else {
        loader
    };
    if loader
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .any(|byte| byte.is_ascii_whitespace() || *byte == b':')
    {
        bail!(
            "loader path cannot contain whitespace or colons: {}",
            loader.display()
        );
    }
    let log = env::var_os("WORKSHOP_LOG")
        .map(PathBuf::from)
        .unwrap_or(env::current_exe()?.with_file_name("workshop.log"));
    let mut preload = loader.as_os_str().to_owned();
    if let Some(existing) = env::var_os("LD_PRELOAD") {
        preload.push(":");
        preload.push(existing);
    }
    println!("[workshop] Plugin log: {}", log.display());
    let error = Command::new(&path)
        .args(forwarded)
        .env("LD_PRELOAD", preload)
        .env("WORKSHOP_LOADER", &loader)
        .env("WORKSHOP_LOG", log)
        .exec();
    Err(error).with_context(|| format!("launching {}", path.display()))
}
