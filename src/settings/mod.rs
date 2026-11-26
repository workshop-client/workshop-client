//! Native settings integration. New page IDs are intercepted before native dispatch.
pub mod config;
mod widgets;
use anyhow::{Context, Result, bail};
use phoenix_hook::{image::Image, pattern::Pattern, x86};
use phoenix_sigs::desktop;
use std::{ffi::c_void, sync::OnceLock};
use widgets::{Any, Text};
const WORKSHOP: u8 = 30;
const PLUGINS: u8 = 31;
type Section = unsafe extern "C" fn(
    *mut Any,
    *mut u8,
    *mut c_void,
    *const c_void,
    *mut c_void,
    *mut c_void,
) -> *mut Any;
type Label = unsafe extern "C" fn(*mut Text, u8) -> *mut Text;
type Descriptor = unsafe extern "C" fn(*mut u8, *mut u8, u8, *mut c_void, *mut c_void) -> *mut u8;
type Select = unsafe extern "C" fn(*mut u8, u8, *mut c_void);
type Visible = unsafe extern "C" fn(u8, *mut c_void, *mut c_void) -> u8;
static SECTION: OnceLock<Section> = OnceLock::new();
static LABEL: OnceLock<Label> = OnceLock::new();
static DESCRIPTOR: OnceLock<Descriptor> = OnceLock::new();
static SELECT: OnceLock<Select> = OnceLock::new();
static VISIBLE: OnceLock<Visible> = OnceLock::new();
static PAGE_OFFSET: OnceLock<usize> = OnceLock::new();
fn custom(page: u8) -> bool {
    page == WORKSHOP || page == PLUGINS
}

/// Install before Phoenix main. Strict build identity remains required while
/// Workshop's extra page IDs and native ABI are experimental.
pub fn install() -> Result<()> {
    crate::access::supported_build()?;
    let image = unsafe { Image::loaded() }
        .into_iter()
        .find(|image| image.file_name() == "discord-phoenix")
        .context("Phoenix image missing")?;
    let section = desktop::settings_section(&image).map_err(anyhow::Error::msg)?;
    let (_, section_end) = image
        .function_bounds(section.func)
        .context("section bounds")?;
    let (root, end) = image
        .function_bounds(section_end)
        .context("settings host bounds")?;
    let code = image.read(root, end - root).context("settings host code")?;
    let one = |pattern: Pattern| -> Result<usize> {
        let matches = pattern.find_in(code);
        if matches.len() != 1 {
            bail!("settings host pattern has {} matches", matches.len());
        }
        Ok(root + matches[0])
    };
    let table_load = one(Pattern::new("48 8D 0D ?? ?? ?? ?? 49 8B 44 0C F0"))?;
    let table_load2 = one(Pattern::new("48 8D 05 ?? ?? ?? ?? 49 8D 34 04 48 83 C6 C0"))?;
    let limit = one(Pattern::new("49 81 FC C0 01 00 00"))?;
    let relative = |at: usize| -> Result<usize> {
        let displacement = i32::from_le_bytes(
            image
                .read(at + 3, 4)
                .context("table displacement")?
                .try_into()?,
        );
        Ok((at + 7).wrapping_add_signed(displacement as isize))
    };
    let table = relative(table_load)?;
    if relative(table_load2)? != table {
        bail!("settings category table references disagree");
    }
    let old = image
        .read(table, 6 * 64)
        .context("settings category table")?;
    let pages = image
        .read_usize(table + 5 * 64 + 48)
        .context("utility pages")?;
    if image.read(pages, 2) != Some(&[16, 24]) {
        bail!("utility category layout changed");
    }
    let labels = desktop::settings_label_parts(&image).map_err(anyhow::Error::msg)?;
    let label_site = image
        .rip_refs(labels.id..labels.id + 1)
        .into_iter()
        .map(|(site, _)| site)
        .filter(|&site| image.is_code(site))
        .collect::<Vec<_>>();
    if label_site.len() != 1 {
        bail!("page label function is ambiguous");
    }
    let label = image
        .function_bounds(label_site[0])
        .context("label function bounds")?
        .0;
    let descriptor = unique(
        &image,
        Pattern::new(
            "55 41 57 41 56 41 55 41 54 53 48 81 EC ?? ?? ?? ?? 49 89 FC 0F B6 C2 6A FF 41 5D 6A 20 5B FF C8 83 F8 1C",
        ),
    )?;
    let select = unique(
        &image,
        Pattern::new(
            "41 57 41 56 53 48 89 D3 49 89 FE 40 88 B7 ?? ?? ?? ?? C6 87 ?? ?? ?? ?? 00 C6 87 ?? ?? ?? ?? 01",
        ),
    )?;
    let visibility = desktop::settings_visibility(&image).map_err(anyhow::Error::msg)? - 11;
    widgets::locate(&image)?;
    PAGE_OFFSET
        .set(section.page_offset)
        .map_err(|_| anyhow::anyhow!("settings installed twice"))?;
    // A nearby immutable copy preserves all existing categories and adds ours.
    let memory = nearby(root, 4096)?;
    unsafe {
        std::ptr::copy_nonoverlapping(old.as_ptr(), memory as *mut u8, old.len());
        let words = (memory + old.len()) as *mut usize;
        let title = "Workshop Settings";
        let key = "WORKSHOP_SETTINGS";
        let page_list = memory + 7 * 64;
        (page_list as *mut [u8; 2]).write([WORKSHOP, PLUGINS]);
        for (i, value) in [
            0,
            key.as_ptr() as usize,
            key.len(),
            title.as_ptr() as usize,
            title.len(),
            0,
            page_list,
            2,
        ]
        .into_iter()
        .enumerate()
        {
            words.add(i).write(value);
        }
        // The startup hook consumes each original function's complete instructions.
        LABEL
            .set(std::mem::transmute::<usize, Label>(hook(
                &image,
                label,
                label_detour as *const () as usize,
            )?))
            .ok();
        DESCRIPTOR
            .set(std::mem::transmute::<usize, Descriptor>(hook(
                &image,
                descriptor,
                descriptor_detour as *const () as usize,
            )?))
            .ok();
        SELECT
            .set(std::mem::transmute::<usize, Select>(hook(
                &image,
                select,
                select_detour as *const () as usize,
            )?))
            .ok();
        VISIBLE
            .set(std::mem::transmute::<usize, Visible>(hook(
                &image,
                visibility,
                visible_detour as *const () as usize,
            )?))
            .ok();
        SECTION
            .set(std::mem::transmute::<usize, Section>(hook(
                &image,
                section.func,
                section_detour as *const () as usize,
            )?))
            .ok();
        for at in [table_load, table_load2] {
            let disp = i32::try_from(memory as isize - (at + 7) as isize)?;
            image.write_code(at + 3, &disp.to_le_bytes())?;
        }
        image.write_code(limit + 3, &0x200u32.to_le_bytes())?;
        if libc::mprotect(memory as *mut _, 4096, libc::PROT_READ) != 0 {
            bail!(
                "protecting category table: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    Ok(())
}
fn unique(image: &Image<'_>, pattern: Pattern) -> Result<usize> {
    let matches = image.scan(pattern);
    if let [address] = matches.as_slice() {
        Ok(*address)
    } else {
        bail!("native settings function has {} matches", matches.len())
    }
}
fn nearby(address: usize, len: usize) -> Result<usize> {
    for distance in (0x2000_0000..0x6000_0000).step_by(0x100_0000) {
        let hint = (address + distance) & !4095;
        let memory = unsafe {
            libc::mmap(
                hint as *mut _,
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED_NOREPLACE,
                -1,
                0,
            )
        };
        if memory != libc::MAP_FAILED {
            return Ok(memory as usize);
        }
    }
    bail!("no nearby memory for settings categories")
}
/// Twelve-byte absolute entry jump, with a checked relocatable trampoline.
/// Called exclusively from the loader before the affected functions execute.
unsafe fn hook(image: &Image<'static>, address: usize, detour: usize) -> Result<usize> {
    let bytes = image.read(address, 32).context("hook code")?;
    let count = (12..=32)
        .find(|&n| x86::relocatable(&bytes[..n], address))
        .context("settings prologue cannot relocate")?;
    let memory = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if memory == libc::MAP_FAILED {
        bail!("allocating native trampoline");
    }
    let mut original = bytes[..count].to_vec();
    original.extend([0xff, 0x25, 0, 0, 0, 0]);
    original.extend((address + count).to_le_bytes());
    unsafe {
        std::ptr::copy_nonoverlapping(original.as_ptr(), memory.cast::<u8>(), original.len());
        if libc::mprotect(memory, 4096, libc::PROT_READ | libc::PROT_EXEC) != 0 {
            libc::munmap(memory, 4096);
            bail!("protecting native trampoline");
        }
        let mut jump = vec![0x48, 0xb8];
        jump.extend(detour.to_le_bytes());
        jump.extend([0xff, 0xe0]);
        jump.resize(count, 0x90);
        image.write_code(address, &jump)?;
    }
    Ok(memory as usize)
}
unsafe extern "C" fn label_detour(out: *mut Text, page: u8) -> *mut Text {
    if custom(page) {
        unsafe {
            out.write(Text::new(if page == WORKSHOP {
                "Workshop"
            } else {
                "Plugins List"
            }));
        }
        out
    } else {
        unsafe { LABEL.get().unwrap()(out, page) }
    }
}
unsafe extern "C" fn descriptor_detour(
    out: *mut u8,
    state: *mut u8,
    page: u8,
    account: *mut c_void,
    cx: *mut c_void,
) -> *mut u8 {
    let result = unsafe {
        DESCRIPTOR.get().unwrap()(
            out,
            state,
            if custom(page) { 24 } else { page },
            account,
            cx,
        )
    };
    if custom(page) {
        let id = if page == WORKSHOP {
            "workshop-client-settings"
        } else {
            "workshop-client-plugins"
        };
        unsafe {
            out.add(16)
                .cast::<usize>()
                .write_unaligned(id.as_ptr() as usize);
            out.add(24).cast::<usize>().write_unaligned(id.len());
        }
    }
    result
}
unsafe extern "C" fn select_detour(state: *mut u8, page: u8, cx: *mut c_void) {
    unsafe {
        SELECT.get().unwrap()(state, if custom(page) { 24 } else { page }, cx);
        if custom(page) {
            state.add(*PAGE_OFFSET.get().unwrap()).write(page);
        }
    }
}
unsafe extern "C" fn visible_detour(page: u8, a: *mut c_void, b: *mut c_void) -> u8 {
    if custom(page) {
        1
    } else {
        unsafe { VISIBLE.get().unwrap()(page, a, b) }
    }
}
unsafe extern "C" fn section_detour(
    out: *mut Any,
    state: *mut u8,
    data: *mut c_void,
    vtable: *const c_void,
    window: *mut c_void,
    cx: *mut c_void,
) -> *mut Any {
    let page = unsafe { *state.add(*PAGE_OFFSET.get().unwrap()) };
    if custom(page) {
        let rendered = std::panic::catch_unwind(|| render(page, out));
        if rendered.is_ok() {
            return out;
        }
        // Do not feed Workshop's custom page IDs into Phoenix's section switch.
        unsafe {
            state.add(*PAGE_OFFSET.get().unwrap()).write(24);
        }
    }
    unsafe { SECTION.get().unwrap()(out, state, data, vtable, window, cx) }
}
fn render(page: u8, out: *mut Any) {
    let config = config::get();
    let mut root = widgets::stack();
    if page == WORKSHOP {
        root = widgets::push(root, widgets::text("Workshop", "Make this client yours."));
        root = widgets::push(
            root,
            widgets::text(
                "Client settings",
                "Changes below take effect after restarting Discord Phoenix.",
            ),
        );
        for (id, title, description, on, index) in [
            (
                "workshop-enable-plugins",
                "Enable plugins",
                "Run the selected WebAssembly plugin when Phoenix starts.",
                config.plugins_enabled,
                0,
            ),
            (
                "workshop-plugin-logging",
                "Plugin logging",
                "Write plugin messages to the Workshop log and terminal.",
                config.plugin_logging,
                1,
            ),
            (
                "workshop-access-override",
                "Skip the client access screen",
                "Open Phoenix past its local account-availability screen.",
                config.access_override,
                2,
            ),
        ] {
            root = widgets::push(root, widgets::toggle(id, title, description, on, index));
        }
        root = widgets::push(root, widgets::text("Workshop Client", "Version 0.1.0"));
    } else {
        root = widgets::push(
            root,
            widgets::card(
                "Plugin Management",
                "Enable or disable plugins individually. Message hooks update immediately.",
            ),
        );
        let plugins = crate::plugins::list();
        let enabled = plugins
            .iter()
            .filter(|plugin| crate::plugins::enabled(plugin, &config))
            .count();
        static COUNTS: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
            let total = crate::plugins::list().len();
            (0..=total)
                .map(|enabled| format!("Enabled plugins: {enabled}     Total plugins: {total}"))
                .collect()
        });
        let summary = COUNTS[enabled].as_str();
        root = widgets::push(root, widgets::card("Plugins", summary));
        if !config.plugins_enabled {
            root = widgets::push(
                root,
                widgets::text(
                    "Plugins are paused",
                    "Turn on Enable plugins in Workshop to run your enabled plugins.",
                ),
            );
        }
        for (index, plugin) in plugins.iter().enumerate() {
            let card = widgets::panel();
            let card = widgets::push(
                card,
                widgets::toggle(
                    &plugin.id,
                    &plugin.name,
                    &plugin.description,
                    config
                        .plugin_enabled
                        .get(&plugin.id)
                        .copied()
                        .unwrap_or(true),
                    100 + index as u64,
                ),
            );
            let mut card = card;
            for (setting_index, setting) in crate::plugin_events::schemas()
                .iter()
                .enumerate()
                .filter(|(_, setting)| setting.plugin_id == plugin.id)
            {
                let checked = config
                    .plugin_settings
                    .get(&plugin.id)
                    .and_then(|values| values.get(&setting.key))
                    .copied()
                    .unwrap_or(setting.default);
                card = widgets::push(
                    card,
                    widgets::toggle(
                        &setting.key,
                        &setting.title,
                        &setting.description,
                        checked,
                        1000 + setting_index as u64,
                    ),
                );
            }
            root = widgets::push(root, card);
        }
    }

    unsafe {
        widgets::finish(root, out);
    }
}
