use std::{env, fs, path::PathBuf};

const PRODUCT_NAME: &str = "PeaceEmu";

fn main() {
    println!("cargo:rerun-if-changed=assets/branding/bird.png");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");

    // Keep non-Windows builds independent of Windows resource tooling.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let png = include_bytes!("assets/branding/bird.png");
    let icon_path = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set"))
        .join("PeaceEmu.ico");
    write_png_icon(&icon_path, png).expect("generate PeaceEmu Windows icon");

    let mut resource = winres::WindowsResource::new();
    resource
        .set_icon(icon_path.to_str().expect("icon path is valid UTF-8"))
        .set("FileDescription", PRODUCT_NAME)
        .set("ProductName", PRODUCT_NAME)
        .set("InternalName", PRODUCT_NAME)
        .set("OriginalFilename", "PeaceEmu.exe")
        .set("CompanyName", "Likounsee");
    resource.compile().expect("embed PeaceEmu icon and version metadata");
}

// ICO supports PNG payloads on modern Windows. This wraps the existing official
// PNG without re-encoding it, so the executable resource and installer share
// the same source artwork.
fn write_png_icon(path: &std::path::Path, png: &[u8]) -> std::io::Result<()> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "branding image is not a PNG",
        ));
    }
    let width = u32::from_be_bytes(png[16..20].try_into().expect("PNG width bytes"));
    let height = u32::from_be_bytes(png[20..24].try_into().expect("PNG height bytes"));
    if width == 0 || height == 0 || width > 256 || height > 256 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Windows icon dimensions must be between 1 and 256 pixels",
        ));
    }

    let mut ico = Vec::with_capacity(22 + png.len());
    ico.extend_from_slice(&0u16.to_le_bytes()); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // icon resource
    ico.extend_from_slice(&1u16.to_le_bytes()); // image count
    ico.push(if width == 256 { 0 } else { width as u8 });
    ico.push(if height == 256 { 0 } else { height as u8 });
    ico.push(0); // palette
    ico.push(0); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // color planes
    ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes()); // image offset
    ico.extend_from_slice(png);
    fs::write(path, ico)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ico_wraps_the_bundled_png_with_a_valid_header() {
        let png = include_bytes!("assets/branding/bird.png");
        let path = std::env::temp_dir().join(format!("peaceemu-icon-{}.ico", std::process::id()));
        write_png_icon(&path, png).unwrap();
        let ico = fs::read(&path).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(u32::from_le_bytes(ico[14..18].try_into().unwrap()) as usize, png.len());
        assert_eq!(u32::from_le_bytes(ico[18..22].try_into().unwrap()), 22);
        assert_eq!(&ico[22..], png);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_non_png_and_oversized_icon_sources() {
        assert!(write_png_icon(std::path::Path::new("unused.ico"), b"not png").is_err());
    }
}
