//! Встраивает значок приложения в исполняемый файл Windows.

use std::path::PathBuf;

#[path = "src/logo.rs"]
mod logo;

const SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];

/// Изображение значка в формате DIB: 32 бита на пиксель снизу вверх и пустая маска.
fn dib(size: u32) -> Vec<u8> {
    let rgba = logo::rgba(size);
    let mut out = Vec::new();
    out.extend(40u32.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend((size * 2).to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend([0u8; 24]);
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend([rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    let mask_row = size.div_ceil(32) * 4;
    out.extend(vec![0u8; (mask_row * size) as usize]);
    out
}

/// Запись скомпилированного файла ресурсов (.res).
fn resource(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, language: u16, data: &[u8]) {
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(32u32.to_le_bytes());
    out.extend([0xFF, 0xFF]);
    out.extend(kind.to_le_bytes());
    out.extend([0xFF, 0xFF]);
    out.extend(id.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(flags.to_le_bytes());
    out.extend(language.to_le_bytes());
    out.extend([0u8; 8]);
    out.extend(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/logo.rs");
    let target = |key: &str, value: &str| std::env::var(key).is_ok_and(|v| v == value);
    if !target("CARGO_CFG_TARGET_OS", "windows") || !target("CARGO_CFG_TARGET_ENV", "msvc") {
        return;
    }

    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;
    let mut res = Vec::new();
    resource(&mut res, 0, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend(0u16.to_le_bytes());
    group.extend(1u16.to_le_bytes());
    group.extend((SIZES.len() as u16).to_le_bytes());
    for (i, size) in SIZES.iter().enumerate() {
        let image = dib(*size);
        let id = i as u16 + 1;
        // Сторона 256 записывается нулём.
        group.extend([*size as u8, *size as u8, 0, 0]);
        group.extend(1u16.to_le_bytes());
        group.extend(32u16.to_le_bytes());
        group.extend((image.len() as u32).to_le_bytes());
        group.extend(id.to_le_bytes());
        resource(&mut res, RT_ICON, id, 0x1010, 0x0409, &image);
    }
    resource(&mut res, RT_GROUP_ICON, 1, 0x1030, 0x0409, &group);

    let path = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("lumicut.res");
    std::fs::write(&path, res).expect("не удалось записать файл ресурсов");
    println!("cargo:rustc-link-arg-bins={}", path.display());
}
