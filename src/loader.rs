//! Linux process attachment only. Native Phoenix event/UI hooks are separate work.
use std::{
    env,
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

#[used]
#[unsafe(link_section = ".init_array")]
static INITIALIZE: extern "C" fn() = initialize;

extern "C" fn initialize() {
    // The same rlib is linked into the CLI, so require both launcher opt-in and
    // the real executable name. AppImage shells and media helpers do no work.
    if env::var_os("WORKSHOP_LOADER").is_none()
        || !fs::read_link("/proc/self/exe")
            .is_ok_and(|path| path.file_name() == Some(OsStr::new("discord-phoenix")))
    {
        return;
    }
    let _ = std::panic::catch_unwind(|| {
        let loader = env::var_os("WORKSHOP_LOADER").unwrap_or_default();
        let remaining = env::var_os("LD_PRELOAD")
            .unwrap_or_default()
            .as_bytes()
            .split(|byte| matches!(byte, b':' | b' '))
            .filter(|entry| !entry.is_empty() && *entry != loader.as_bytes())
            .map(<[u8]>::to_vec)
            .collect::<Vec<_>>()
            .join(&b':');
        // SAFETY: this startup constructor runs before Phoenix main and before
        // Workshop creates its worker. No plugin code has run at this point.
        unsafe {
            if remaining.is_empty() {
                env::remove_var("LD_PRELOAD");
            } else {
                env::set_var("LD_PRELOAD", OsStr::from_bytes(&remaining));
            }
            env::remove_var("WORKSHOP_LOADER");
        }
        let log = env::var_os("WORKSHOP_LOG").map(PathBuf::from);
        write_log(log.as_deref(), "attached to discord-phoenix");
        #[cfg(target_arch = "x86_64")]
        if env::var_os("WORKSHOP_ACCESS_OVERRIDE").as_deref() != Some(OsStr::new("0"))
            && crate::settings::config::get().access_override
        {
            match crate::access::apply() {
                Ok(message) => write_log(log.as_deref(), &message),
                Err(error) => write_log(log.as_deref(), &format!("access override: {error:#}")),
            }
        } else {
            write_log(log.as_deref(), "client access-screen override disabled");
        }
        #[cfg(target_arch = "x86_64")]
        match crate::settings::install() {
            Ok(()) => write_log(log.as_deref(), "Workshop Settings installed in Phoenix"),
            Err(error) => write_log(
                log.as_deref(),
                &format!("settings UI unavailable: {error:#}"),
            ),
        }
        match crate::messages::install() {
            Ok(()) => write_log(log.as_deref(), "message receive hooks installed"),
            Err(error) => write_log(
                log.as_deref(),
                &format!("message receive hooks unavailable: {error:#}"),
            ),
        }
        match crate::outgoing::install() {
            Ok(()) => write_log(log.as_deref(), "HTTP send and response hooks installed"),
            Err(error) => write_log(
                log.as_deref(),
                &format!("message send hook unavailable: {error:#}"),
            ),
        }
        match crate::plugin_images::install() {
            Ok(()) => write_log(log.as_deref(), "plugin image adapter installed"),
            Err(error) => write_log(
                log.as_deref(),
                &format!("plugin image adapter unavailable: {error:#}"),
            ),
        }
        crate::plugin_events::start();
        let plugin = env::var_os("WORKSHOP_PLUGIN").map(PathBuf::from);
        if let Err(error) = std::thread::Builder::new()
            .name("workshop-wasm".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| run(plugin.as_deref(), log.as_deref()));
                match result {
                    Ok(Ok(())) => write_log(log.as_deref(), "plugin finished"),
                    Ok(Err(error)) => {
                        write_log(log.as_deref(), &format!("plugin failed: {error:#}"))
                    }
                    Err(_) => write_log(log.as_deref(), "plugin worker panicked"),
                }
            })
        {
            eprintln!("[workshop] could not start plugin worker: {error}");
        }
    });
}

fn run(plugin: Option<&Path>, log: Option<&Path>) -> anyhow::Result<()> {
    if !crate::settings::config::get().plugins_enabled {
        write_log(log, "startup plugins disabled");
        return Ok(());
    }
    let _ = plugin;
    let config = crate::settings::config::get();
    for plugin in crate::plugins::list()
        .iter()
        .filter(|plugin| crate::plugins::enabled(plugin, &config))
    {
        if crate::plugin_events::supports(plugin) {
            continue;
        }
        match crate::run_plugin(&plugin.bytes) {
            Ok(messages) => {
                for message in messages {
                    if config.plugin_logging {
                        write_log(log, &message);
                    }
                }
            }
            Err(error) => write_log(log, &format!("plugin {} failed: {error:#}", plugin.name)),
        }
    }

    Ok(())
}

fn write_log(path: Option<&Path>, message: &str) {
    let line = format!("[workshop pid={}] {message}", std::process::id());
    eprintln!("{line}");
    if let Some(path) = path
        && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}
