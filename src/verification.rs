//! Explicit `--verify` capture helpers. Never called by the normal overlay lifecycle.
use slint::ComponentHandle;
use std::{error::Error, fs, path::PathBuf};

/// Capture the actual renderer output, check alpha and silhouette geometry, and write
/// an alpha-preserving BMP plus a gray-composited BMP suitable for visual review.
/// These checks exercise rendered pixels; they do not prove desktop hit testing.
pub fn snapshot(ui: &crate::IslandWindow, name: &str) -> Result<PathBuf, Box<dyn Error>> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Snapshot names must contain only letters, digits, '-' or '_'".into());
    }
    let buffer = ui.window().take_snapshot()?;
    let width = buffer.width();
    let height = buffer.height();
    if width < 4 || height < 4 {
        return Err("Snapshot is unexpectedly empty".into());
    }
    let pixels = buffer.as_slice();
    let pixel = |x: u32, y: u32| &pixels[(y * width + x) as usize];
    for (x, y) in [
        (0, 0),
        (width - 1, 0),
        (0, height - 1),
        (width - 1, height - 1),
    ] {
        if pixel(x, y).a != 0 {
            return Err(format!(
                "{name}: transparent canvas corner ({x},{y}) has alpha {}",
                pixel(x, y).a
            )
            .into());
        }
    }
    let scale = width as f64 / f64::from(ui.get_canvas_width());
    let expected_canvas_height = f64::from(ui.get_canvas_height()) * scale;
    if (f64::from(height) - expected_canvas_height).abs() > 2. {
        return Err(format!(
            "{name}: canvas is {width}x{height}; expected logical {}x{} at scale {scale:.3}",
            ui.get_canvas_width(),
            ui.get_canvas_height()
        )
        .into());
    }
    let interior = pixel(width / 2, (3.0 * scale).round() as u32);
    if ui.get_appearance() == 0
        && (interior.a < 250 || interior.r > 3 || interior.g > 3 || interior.b > 3)
    {
        return Err(
            format!("{name}: top-center silhouette is not opaque black: {interior:?}").into(),
        );
    }

    let mut bounds = (width, height, 0, 0);
    let mut nontransparent = 0_u64;
    let mut partial_alpha = 0_u64;
    for y in 0..height {
        for x in 0..width {
            let alpha = pixel(x, y).a;
            if alpha > 0 {
                bounds.0 = bounds.0.min(x);
                bounds.1 = bounds.1.min(y);
                bounds.2 = bounds.2.max(x);
                bounds.3 = bounds.3.max(y);
                nontransparent += 1;
                partial_alpha += u64::from(alpha < 255);
            }
        }
    }
    let expected_width = f64::from(ui.get_shape_width() + 2. * ui.get_shoulder()) * scale;
    let expected_height = f64::from(ui.get_shape_height()) * scale;
    let actual_width = bounds.2 + 1 - bounds.0;
    let actual_height = bounds.3 + 1 - bounds.1;
    if (f64::from(actual_width) - expected_width).abs() > 3.0
        || (f64::from(actual_height) - expected_height).abs() > 3.0
        || bounds.1 != 0
    {
        return Err(format!("{name}: silhouette bounds {bounds:?}, expected {expected_width:.2}x{expected_height:.2} anchored at y=0").into());
    }
    let directory = PathBuf::from("verification");
    fs::create_dir_all(&directory)?;
    // BITMAPV4HEADER explicitly specifies alpha masks; no encoder dependency needed.
    let mut alpha_bmp = Vec::with_capacity(122 + pixels.len() * 4);
    alpha_bmp.extend_from_slice(b"BM");
    put_u32(&mut alpha_bmp, 122 + width * height * 4);
    put_u32(&mut alpha_bmp, 0);
    put_u32(&mut alpha_bmp, 122);
    put_u32(&mut alpha_bmp, 108);
    put_u32(&mut alpha_bmp, width);
    put_u32(&mut alpha_bmp, height);
    alpha_bmp.extend_from_slice(&1_u16.to_le_bytes());
    alpha_bmp.extend_from_slice(&32_u16.to_le_bytes());
    put_u32(&mut alpha_bmp, 3); // BI_BITFIELDS
    put_u32(&mut alpha_bmp, width * height * 4);
    for value in [
        2835, 2835, 0, 0, 0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000, 0x73524742,
    ] {
        put_u32(&mut alpha_bmp, value);
    }
    alpha_bmp.resize(122, 0);
    for y in (0..height).rev() {
        for x in 0..width {
            let p = pixel(x, y);
            alpha_bmp.extend_from_slice(&[p.b, p.g, p.r, p.a]);
        }
    }
    fs::write(directory.join(format!("{name}-alpha.bmp")), alpha_bmp)?;

    // Standard 24-bit BMP can be opened by common image inspection tools. It shows
    // alpha on a neutral gray backdrop, while the separate file retains raw alpha.
    let stride = (width * 3 + 3) & !3;
    let mut review_bmp = Vec::with_capacity((54 + stride * height) as usize);
    review_bmp.extend_from_slice(b"BM");
    put_u32(&mut review_bmp, 54 + stride * height);
    put_u32(&mut review_bmp, 0);
    put_u32(&mut review_bmp, 54);
    put_u32(&mut review_bmp, 40);
    put_u32(&mut review_bmp, width);
    put_u32(&mut review_bmp, height);
    review_bmp.extend_from_slice(&1_u16.to_le_bytes());
    review_bmp.extend_from_slice(&24_u16.to_le_bytes());
    put_u32(&mut review_bmp, 0);
    put_u32(&mut review_bmp, stride * height);
    for value in [2835, 2835, 0, 0] {
        put_u32(&mut review_bmp, value);
    }
    for y in (0..height).rev() {
        for x in 0..width {
            let p = pixel(x, y);
            let composite = |v: u8| {
                ((u32::from(v) * u32::from(p.a) + 210 * (255 - u32::from(p.a)) + 127) / 255) as u8
            };
            review_bmp.extend_from_slice(&[composite(p.b), composite(p.g), composite(p.r)]);
        }
        review_bmp.resize(review_bmp.len() + (stride - width * 3) as usize, 0);
    }
    let output = directory.join(format!("{name}.bmp"));
    fs::write(&output, review_bmp)?;
    fs::write(
        directory.join(format!("{name}.txt")),
        format!(
            "renderer snapshot: {width}x{height}\nsilhouette bounds: {bounds:?}\nnontransparent pixels: {nontransparent}\npartially transparent edge pixels: {partial_alpha}\ntransparent corners: PASS\nmaterial-aware interior check: PASS\nsilhouette size and top anchoring: PASS\nDesktop composition and click-through are separate native checks.\n"
        ),
    )?;
    Ok(output)
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

pub fn snapshot_preferences(
    ui: &crate::SettingsWindow,
) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    use std::io::Write;
    let buffer = ui.window().take_snapshot()?;
    std::fs::create_dir_all("verification")?;
    let path = std::path::PathBuf::from("verification/settings.ppm");
    let mut file = std::fs::File::create(&path)?;
    write!(file, "P6\n{} {}\n255\n", buffer.width(), buffer.height())?;
    let mut pixels = Vec::with_capacity(buffer.as_slice().len() * 3);
    for p in buffer.as_slice() {
        pixels.extend_from_slice(&[p.r, p.g, p.b]);
    }
    file.write_all(&pixels)?;
    Ok(path)
}
