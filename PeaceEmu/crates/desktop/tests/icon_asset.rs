#[test]
fn bundled_brand_icon_is_a_valid_rgba_image() {
    let decoded = image::load_from_memory(include_bytes!("../assets/branding/bird.png"))
        .expect("the bundled brand icon must be a valid image");
    let rgba = decoded.into_rgba8();

    assert!(rgba.width() > 0, "icon width must be non-zero");
    assert!(rgba.height() > 0, "icon height must be non-zero");
    assert_eq!(
        rgba.as_raw().len(),
        rgba.width() as usize * rgba.height() as usize * 4,
        "RGBA buffer length must match the decoded dimensions"
    );
}
