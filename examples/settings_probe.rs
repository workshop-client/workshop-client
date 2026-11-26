use phoenix_sigs::{desktop, file::BinaryFile};
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).ok_or_else(|| {
        anyhow::anyhow!(
            "usage: cargo run --example settings_probe -- /path/to/extracted/discord-phoenix"
        )
    })?;
    let bytes = std::fs::read(path)?;
    let parsed = BinaryFile::parse("phoenix", &bytes)?;
    let image = parsed.image();
    for at in [0x957c2e9, 0x9582554, 0x126756bc, 0x17b45976] {
        println!("bounds {at:#x} {:x?}", image.function_bounds(at));
    }
    println!(
        "LABEL {:x?}",
        desktop::settings_label_parts(image).map(|l| (l.name, l.id))
    );
    println!(
        "HTTP {:x?}",
        phoenix_sigs::http::DesktopHttpTransport::locate(image)
    );
    let gpui = desktop::gpui(image).map_err(anyhow::Error::msg)?;
    println!(
        "SYMBOLS {:x?}",
        [
            &gpui.caller_loc,
            &gpui.div_new,
            &gpui.div_flex,
            &gpui.div_flex_col,
            &gpui.div_style_mut,
            &gpui.div_child_div,
            &gpui.div_child_button,
            &gpui.div_into_any,
            &gpui.text_block,
            &gpui.switch_new,
            &gpui.switch_into_div,
            &gpui.button_new,
            &gpui.alloc_or_oom
        ]
    );
    println!("GPUI {:x?}", desktop::gpui_layouts(image, &gpui));
    println!(
        "SECTION {:x?}",
        desktop::settings_section(image).map(|s| (s.func, s.page_offset))
    );
    Ok(())
}
