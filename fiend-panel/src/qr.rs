use qrcode::{Color, EcLevel, QrCode};

pub fn svg(text: &str) -> Result<String, String> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
        .map_err(|_| "The text is too large for a QR code".to_owned())?;
    let width = code.width();
    let quiet = 4;
    let size = width + quiet * 2;
    let mut path = String::new();
    for (index, color) in code.to_colors().into_iter().enumerate() {
        if color == Color::Dark {
            let x = index % width + quiet;
            let y = index / width + quiet;
            path.push_str(&format!("M{x} {y}h1v1h-1z"));
        }
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" shape-rendering=\"crispEdges\" role=\"img\" aria-label=\"QR code\"><rect width=\"{size}\" height=\"{size}\" fill=\"#ffffff\"/><path fill=\"#000000\" d=\"{path}\"/></svg>"
    ))
}

pub fn version(text: &str) -> Option<i16> {
    QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
        .ok()
        .and_then(|code| match code.version() {
            qrcode::Version::Normal(version) => Some(version),
            qrcode::Version::Micro(_) => None,
        })
}
