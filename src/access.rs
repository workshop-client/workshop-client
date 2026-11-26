//! In-memory access-screen predicate override for one inspected Linux build.
//! Derived from the local binary; deliberately does not patch the access getter.
use anyhow::{Context, Result, bail};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

const BUILD_ID: &[u8] = &[
    0x2b, 0x07, 0xb1, 0xd5, 0x2b, 0x4e, 0xd6, 0x81, 0x29, 0x3b, 0x55, 0x1b, 0x72, 0x6c, 0x3e, 0x18,
    0xd5, 0x1e, 0x8c, 0x92,
];
const PREDICATE: usize = 0x0e3176a1;
const ORIGINAL: &[u8] = &[
    0x83, 0x7f, 0x78, 0x03, 0x75, 0x10, 0x50, 0xe8, 0x6d, 0xe3, 0xff, 0xff, 0x3c, 0x02, 0x0f, 0x95,
    0xc0, 0x48, 0x83, 0xc4, 0x08, 0xc3, 0x31, 0xc0, 0xc3,
];
// Replace `setne al` with `xor eax,eax; nop`. Keep the original status call,
// comparison, stack restoration and return. The predicate then reports false.
const RESULT_OFFSET: usize = 14;
const REPLACEMENT: &[u8] = &[0x31, 0xc0, 0x90];

fn validate(header: &[u8], code: &[u8]) -> Result<()> {
    if header.get(..6) != Some(b"\x7fELF\x02\x01")
        || header.get(0x31c..0x32c)
            != Some(&[4, 0, 0, 0, 20, 0, 0, 0, 3, 0, 0, 0, b'G', b'N', b'U', 0])
        || header.get(0x32c..0x340) != Some(BUILD_ID)
    {
        bail!("unsupported Phoenix ELF build; access override skipped");
    }
    if code != ORIGINAL {
        bail!("access predicate bytes differ; override skipped");
    }
    Ok(())
}

pub(super) fn supported_build() -> Result<()> {
    let mut header = [0u8; 0x340];
    File::open("/proc/self/exe")?.read_exact(&mut header)?;
    validate(&header, ORIGINAL)
}

unsafe extern "C" fn main_image(
    info: *mut libc::dl_phdr_info,
    _: usize,
    result: *mut libc::c_void,
) -> libc::c_int {
    // SAFETY: dl_iterate_phdr supplies a live record and our usize out pointer.
    unsafe {
        if !(*info).dlpi_name.is_null() && *(*info).dlpi_name == 0 {
            *result.cast::<usize>() = (*info).dlpi_addr as usize;
            return 1;
        }
    }
    0
}

/// Called only by the startup constructor, before Phoenix main or WASM workers.
pub(super) fn apply() -> Result<String> {
    let mut executable = File::open("/proc/self/exe")?;
    let mut header = [0u8; 0x340];
    executable.read_exact(&mut header)?;
    executable.seek(SeekFrom::Start(PREDICATE as u64))?;
    let mut code = [0u8; ORIGINAL.len()];
    executable.read_exact(&mut code)?;
    validate(&header, &code)?;
    let mut base = 0usize;
    // SAFETY: callback only reads loader-provided records and sets base.
    unsafe {
        libc::dl_iterate_phdr(Some(main_image), (&mut base as *mut usize).cast());
    }
    if base == 0 {
        bail!("could not resolve Phoenix load address");
    }
    let address = base
        .checked_add(PREDICATE)
        .context("predicate address overflow")?;
    // SAFETY: build identity and file bytes establish this executable mapping.
    // Check mapped bytes too, refusing other mods or unexpected relocation.
    unsafe {
        if std::slice::from_raw_parts(address as *const u8, ORIGINAL.len()) != ORIGINAL {
            bail!("mapped access predicate differs; override skipped");
        }
        let page_size = libc::sysconf(libc::_SC_PAGESIZE);
        if page_size <= 0 {
            bail!("could not determine page size");
        }
        let page_size = page_size as usize;
        let target = address + RESULT_OFFSET;
        let page = target / page_size * page_size;
        let length = (target + REPLACEMENT.len()).div_ceil(page_size) * page_size - page;
        if libc::mprotect(page as *mut _, length, libc::PROT_READ | libc::PROT_WRITE) != 0 {
            return Err(std::io::Error::last_os_error()).context("making predicate writable");
        }
        std::ptr::copy_nonoverlapping(REPLACEMENT.as_ptr(), target as *mut u8, REPLACEMENT.len());
        // x86-64 instruction/data caches are coherent; this runs before callers.
        if libc::mprotect(page as *mut _, length, libc::PROT_READ | libc::PROT_EXEC) != 0 {
            return Err(std::io::Error::last_os_error()).context("restoring predicate protection");
        }
    }
    Ok(format!(
        "client access-screen override applied at {:#x}",
        address + RESULT_OFFSET
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_builds_and_modified_code() {
        let mut header = [0u8; 0x340];
        assert!(validate(&header, ORIGINAL).is_err());
        header[..6].copy_from_slice(b"\x7fELF\x02\x01");
        header[0x31c..0x32c]
            .copy_from_slice(&[4, 0, 0, 0, 20, 0, 0, 0, 3, 0, 0, 0, b'G', b'N', b'U', 0]);
        header[0x32c..].copy_from_slice(BUILD_ID);
        assert!(validate(&header, ORIGINAL).is_ok());
        let mut changed = ORIGINAL.to_vec();
        changed[0] ^= 1;
        assert!(validate(&header, &changed).is_err());
    }
}
