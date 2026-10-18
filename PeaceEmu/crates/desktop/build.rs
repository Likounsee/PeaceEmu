use std::{env, fs, io::Cursor, path::PathBuf};

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

// ICO supports PNG payloads on modern Windows. Render the same official bird
// artwork into a centered 256x256 transparent canvas before wrapping it as ICO.
fn write_png_icon(path: &std::path::Path, source_png: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let source = image::load_from_memory(source_png)?;
    let source = source.resize(232, 232, image::imageops::FilterType::Lanczos3).into_rgba8();
    let (width, height) = source.dimensions();
    let mut canvas = image::RgbaImage::from_pixel(256, 256, image::Rgba([0, 0, 0, 0]));
    image::imageops::overlay(
        &mut canvas,
        &source,
        i64::from((256 - width) / 2),
        i64::from((256 - height) / 2),
    );

    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(canvas).write_to(&mut png, image::ImageOutputFormat::Png)?;
    let png = png.into_inner();

    let mut ico = Vec::with_capacity(22 + png.len());
    ico.extend_from_slice(&0u16.to_le_bytes()); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // icon resource
    ico.extend_from_slice(&1u16.to_le_bytes()); // image count
    ico.push(0); // 256 pixels is encoded as zero in ICO directory entries
    ico.push(0);
    ico.push(0); // palette
    ico.push(0); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // color planes
    ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes()); // image offset
    ico.extend_from_slice(&png);
    fs::write(path, ico)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_icon_has_a_valid_ico_header_and_png_payload() {
        let source = include_bytes!("assets/branding/bird.png");
        let path = std::env::temp_dir().join(format!("peaceemu-icon-{}.ico", std::process::id()));
        write_png_icon(&path, source).unwrap();
        let ico = fs::read(&path).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(u32::from_le_bytes(ico[14..18].try_into().unwrap()) as usize, ico.len() - 22);
        assert_eq!(u32::from_le_bytes(ico[18..22].try_into().unwrap()), 22);
        assert_eq!(&ico[22..30], b"\x89PNG\r\n\x1a\n");
        let dimensions = image::load_from_memory(&ico[22..]).unwrap().dimensions();
        assert_eq!(dimensions, (256, 256));
        fs::remove_file(path).unwrap();
    }
}
