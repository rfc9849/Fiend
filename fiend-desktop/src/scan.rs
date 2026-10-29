use std::path::Path;
use std::process::Command;

fn capture(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let target = path.to_string_lossy().replace('\'', "''");
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
             $b=[System.Windows.Forms.SystemInformation]::VirtualScreen; \
             $i=New-Object System.Drawing.Bitmap $b.Width,$b.Height; \
             $g=[System.Drawing.Graphics]::FromImage($i); \
             $g.CopyFromScreen($b.Left,$b.Top,0,0,$i.Size); \
             $i.Save('{target}',[System.Drawing.Imaging.ImageFormat]::Png); \
             $g.Dispose(); $i.Dispose()"
        );
        let status = Command::new(crate::service::powershell_path())
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .creation_flags(0x0800_0000)
            .status();
        if matches!(status, Ok(status) if status.success()) && path.exists() {
            return Ok(());
        }
        Err("Cannot capture the screen. Try again.".into())
    }
    #[cfg(not(windows))]
    {
        let target = path.to_string_lossy().into_owned();
        let attempts: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
            &[("screencapture", &["-x", "-t", "png"])]
        } else {
            &[
                ("grim", &[]),
                ("gnome-screenshot", &["-f"]),
                ("spectacle", &["-b", "-n", "-f", "-o"]),
                ("import", &["-window", "root"]),
            ]
        };
        for (program, arguments) in attempts {
            let status = Command::new(program).args(*arguments).arg(&target).status();
            if matches!(status, Ok(status) if status.success()) && path.exists() {
                return Ok(());
            }
        }
        Err("Cannot capture the screen. Allow screen recording for Fiend and try again.".into())
    }
}

pub fn decode(image: &image::DynamicImage) -> Vec<String> {
    let gray = image.to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(gray);
    prepared
        .detect_grids()
        .into_iter()
        .filter_map(|grid| grid.decode().ok())
        .map(|(_, content)| content)
        .collect()
}

pub fn find_link(image: &image::DynamicImage) -> Result<String, String> {
    let codes = decode(image);
    if codes.is_empty() {
        return Err("No QR code was found on the screen.".into());
    }
    codes
        .into_iter()
        .find(|text| text.starts_with("fiend://"))
        .ok_or_else(|| "The QR code on the screen is not a Fiend link.".into())
}

pub fn scan_screen() -> Result<String, String> {
    let path = std::env::temp_dir().join(format!("fiend-scan-{}.png", std::process::id()));
    let result = capture(&path).and_then(|()| {
        image::open(&path)
            .map_err(|error| error.to_string())
            .and_then(|image| find_link(&image))
    });
    let _ = std::fs::remove_file(&path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(text: &str) -> image::DynamicImage {
        let code = qrcode::QrCode::new(text.as_bytes()).unwrap();
        let image = code
            .render::<image::Luma<u8>>()
            .quiet_zone(true)
            .module_dimensions(4, 4)
            .build();
        image::DynamicImage::ImageLuma8(image)
    }

    #[test]
    fn a_fiend_link_is_found_in_a_picture() {
        let link = "fiend://v1/eJzLSM3JyQcABiwCFQ";
        assert_eq!(find_link(&render(link)).unwrap(), link);
    }

    #[test]
    fn other_codes_and_empty_pictures_are_refused() {
        assert!(
            find_link(&render("https://example.com"))
                .unwrap_err()
                .contains("not a Fiend link")
        );
        let blank = image::DynamicImage::new_luma8(200, 200);
        assert!(find_link(&blank).unwrap_err().contains("No QR code"));
    }
}
