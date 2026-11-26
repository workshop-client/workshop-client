//! Narrow bindings to the native widgets resolved by phoenix-sigs.
use phoenix_hook::image::Image;
use phoenix_sigs::desktop::{self, Gpui, GpuiLayout};
use std::{ffi::c_void, sync::OnceLock};

#[repr(C, align(16))]
pub struct Buffer<const N: usize>([u8; N]);
impl<const N: usize> Buffer<N> {
    pub fn new() -> Self {
        Self([0; N])
    }
    pub fn put<T>(&mut self, offset: usize, value: T) {
        assert!(offset + size_of::<T>() <= N);
        unsafe {
            self.0
                .as_mut_ptr()
                .add(offset)
                .cast::<T>()
                .write_unaligned(value);
        }
    }
}
pub type Div = Buffer<256>;
pub type Any = Buffer<32>;
type Switch = Buffer<2048>;
#[repr(C)]
pub struct Text {
    tag: usize,
    pointer: *const u8,
    length: usize,
}
impl Text {
    pub fn new(text: &'static str) -> Self {
        Self {
            tag: 24,
            pointer: text.as_ptr(),
            length: text.len(),
        }
    }
}
struct Widgets {
    functions: Gpui,
    layout: GpuiLayout,
}
static WIDGETS: OnceLock<Widgets> = OnceLock::new();
pub fn locate(image: &Image<'static>) -> anyhow::Result<()> {
    let functions = desktop::gpui(image).map_err(anyhow::Error::msg)?;
    let layout = desktop::gpui_layouts(image, &functions).map_err(anyhow::Error::msg)?;
    WIDGETS
        .set(Widgets { functions, layout })
        .map_err(|_| anyhow::anyhow!("widgets already initialized"))
}
fn w() -> &'static Widgets {
    WIDGETS
        .get()
        .expect("native widgets initialized before hooks")
}
macro_rules! function {
    ($field:ident,$ty:ty) => {
        unsafe { std::mem::transmute::<usize, $ty>(w().functions.$field) }
    };
}
pub fn stack() -> Div {
    let mut div = Div::new();
    function!(div_new, extern "C" fn(*mut Div, *const c_void) -> *mut Div)(
        &mut div,
        w().functions.caller_loc as *const _,
    );
    for address in [w().functions.div_flex, w().functions.div_flex_col] {
        let mut out = Div::new();
        unsafe {
            std::mem::transmute::<usize, extern "C" fn(*mut Div, *mut Div) -> *mut Div>(address)(
                &mut out, &mut div,
            );
        }
        div = out;
    }
    let style = function!(div_style_mut, extern "C" fn(*mut Div) -> *mut u8)(&mut div);
    for offset in w().layout.style.gap {
        unsafe {
            style
                .add(offset)
                .cast::<u64>()
                .write_unaligned(u64::from(16f32.to_bits()) << 32);
        }
    }
    div
}
pub fn text(title: &'static str, description: &'static str) -> Div {
    let mut div = Div::new();
    function!(
        text_block,
        extern "C" fn(*mut Div, *const Text, *const Text) -> *mut Div
    )(&mut div, &Text::new(title), &Text::new(description));
    div
}
pub fn push(mut parent: Div, mut child: Div) -> Div {
    let mut out = Div::new();
    function!(
        div_child_div,
        extern "C" fn(*mut Div, *mut Div, *mut Div) -> *mut Div
    )(&mut out, &mut parent, &mut child);
    out
}
pub unsafe fn finish(mut div: Div, out: *mut Any) {
    function!(div_into_any, extern "C" fn(*mut Any, *mut Div) -> *mut Any)(out, &mut div);
}

#[repr(C)]
struct Callback {
    drop: usize,
    size: usize,
    align: usize,
    once: unsafe fn(*const u64, u8, *mut c_void, *mut c_void),
    mutable: unsafe fn(*const u64, u8, *mut c_void, *mut c_void),
    call: unsafe fn(*const u64, u8, *mut c_void, *mut c_void),
    extra: usize,
}
static CALLBACK: Callback = Callback {
    drop: 0,
    size: 8,
    align: 8,
    once: changed,
    mutable: changed,
    call: changed,
    extra: 0,
};
unsafe fn changed(data: *const u64, on: u8, _window: *mut c_void, _cx: *mut c_void) {
    let _ = std::panic::catch_unwind(|| {
        if let Err(error) = super::config::set(unsafe { *data }, on != 0) {
            eprintln!("[workshop] saving settings failed: {error:#}");
        }
    });
}
pub fn toggle(
    id: &'static str,
    label: &'static str,
    description: &'static str,
    on: bool,
    index: u64,
) -> Div {
    let mut switch = Switch::new();
    function!(
        switch_new,
        extern "C" fn(*mut Switch, *const u8, usize, u8) -> *mut Switch
    )(&mut switch, id.as_ptr(), id.len(), u8::from(on));
    let at = w().layout.switch;
    switch.put(at.label, Text::new(label));
    switch.put(at.description, Text::new(description));
    let allocate = function!(alloc_or_oom, extern "C" fn(usize, usize) -> *mut u64);
    let action = allocate(8, 24);
    unsafe {
        action.write(1);
        action.add(1).write(1);
        action.add(2).write(index);
    }
    switch.put(at.on_change, action);
    switch.put(at.on_change + 8, &CALLBACK as *const Callback);
    let mut div = Div::new();
    function!(
        switch_into_div,
        extern "C" fn(*mut Div, *mut Switch) -> *mut Div
    )(&mut div, &mut switch);
    div
}

/// Native bordered settings card, inheriting Phoenix's text and switch theme.
pub fn card(title: &'static str, description: &'static str) -> Div {
    push(panel(), text(title, description))
}

pub fn panel() -> Div {
    let mut div = stack();
    let style = function!(div_style_mut, extern "C" fn(*mut Div) -> *mut u8)(&mut div);
    let offsets = &w().layout.style;
    // The locator validates all field offsets against the native style size.
    unsafe {
        for (fields, pixels) in [
            (&offsets.padding[..], 18f32),
            (&offsets.corner_radii[..], 8f32),
            (&offsets.border_widths[..], 1f32),
        ] {
            for &offset in fields {
                style
                    .add(offset)
                    .cast::<u64>()
                    .write_unaligned(u64::from(pixels.to_bits()) << 32);
            }
        }
        style
            .add(offsets.border_color)
            .cast::<u32>()
            .write_unaligned(1);
        style
            .add(offsets.border_color + 4)
            .cast::<[f32; 4]>()
            .write_unaligned([0.62, 0.02, 0.6, 0.18]);
        style
            .add(offsets.size[0])
            .cast::<u64>()
            .write_unaligned(0x3f80_0000_0000_0002);
    }
    div
}
