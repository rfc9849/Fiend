use std::borrow::Cow;

use gpui_kit::{AssetSource, SharedString};

pub struct Assets;

fn find(path: &str) -> Option<&'static [u8]> {
    match path {
        "icons/arc.svg" => Some(include_bytes!("../assets/icons/arc.svg").as_slice()),
        "icons/arrow.svg" => Some(include_bytes!("../assets/icons/arrow.svg").as_slice()),
        "icons/block.svg" => Some(include_bytes!("../assets/icons/block.svg").as_slice()),
        "icons/brand.svg" => Some(include_bytes!("../assets/icons/brand.svg").as_slice()),
        "icons/brand-face.svg" => Some(include_bytes!("../assets/icons/brand-face.svg").as_slice()),
        "icons/check.svg" => Some(include_bytes!("../assets/icons/check.svg").as_slice()),
        "icons/chevron.svg" => Some(include_bytes!("../assets/icons/chevron.svg").as_slice()),
        "icons/clock.svg" => Some(include_bytes!("../assets/icons/clock.svg").as_slice()),
        "icons/close.svg" => Some(include_bytes!("../assets/icons/close.svg").as_slice()),
        "icons/copy.svg" => Some(include_bytes!("../assets/icons/copy.svg").as_slice()),
        "icons/data.svg" => Some(include_bytes!("../assets/icons/data.svg").as_slice()),
        "icons/down.svg" => Some(include_bytes!("../assets/icons/down.svg").as_slice()),
        "icons/edit.svg" => Some(include_bytes!("../assets/icons/edit.svg").as_slice()),
        "icons/eye.svg" => Some(include_bytes!("../assets/icons/eye.svg").as_slice()),
        "icons/file.svg" => Some(include_bytes!("../assets/icons/file.svg").as_slice()),
        "icons/globe.svg" => Some(include_bytes!("../assets/icons/globe.svg").as_slice()),
        "icons/home.svg" => Some(include_bytes!("../assets/icons/home.svg").as_slice()),
        "icons/info.svg" => Some(include_bytes!("../assets/icons/info.svg").as_slice()),
        "icons/key.svg" => Some(include_bytes!("../assets/icons/key.svg").as_slice()),
        "icons/link.svg" => Some(include_bytes!("../assets/icons/link.svg").as_slice()),
        "icons/log.svg" => Some(include_bytes!("../assets/icons/log.svg").as_slice()),
        "icons/plus.svg" => Some(include_bytes!("../assets/icons/plus.svg").as_slice()),
        "icons/power.svg" => Some(include_bytes!("../assets/icons/power.svg").as_slice()),
        "icons/refresh.svg" => Some(include_bytes!("../assets/icons/refresh.svg").as_slice()),
        "icons/route.svg" => Some(include_bytes!("../assets/icons/route.svg").as_slice()),
        "icons/server.svg" => Some(include_bytes!("../assets/icons/server.svg").as_slice()),
        "icons/settings.svg" => Some(include_bytes!("../assets/icons/settings.svg").as_slice()),
        "icons/share.svg" => Some(include_bytes!("../assets/icons/share.svg").as_slice()),
        "icons/shield.svg" => Some(include_bytes!("../assets/icons/shield.svg").as_slice()),
        "icons/speed.svg" => Some(include_bytes!("../assets/icons/speed.svg").as_slice()),
        "icons/swap.svg" => Some(include_bytes!("../assets/icons/swap.svg").as_slice()),
        "icons/up.svg" => Some(include_bytes!("../assets/icons/up.svg").as_slice()),
        _ => None,
    }
}

const ICONS: &[&str] = &[
    "icons/arc.svg",
    "icons/arrow.svg",
    "icons/block.svg",
    "icons/brand.svg",
    "icons/brand-face.svg",
    "icons/check.svg",
    "icons/chevron.svg",
    "icons/clock.svg",
    "icons/close.svg",
    "icons/copy.svg",
    "icons/data.svg",
    "icons/down.svg",
    "icons/edit.svg",
    "icons/eye.svg",
    "icons/file.svg",
    "icons/globe.svg",
    "icons/home.svg",
    "icons/info.svg",
    "icons/key.svg",
    "icons/link.svg",
    "icons/log.svg",
    "icons/plus.svg",
    "icons/power.svg",
    "icons/refresh.svg",
    "icons/route.svg",
    "icons/server.svg",
    "icons/settings.svg",
    "icons/share.svg",
    "icons/shield.svg",
    "icons/speed.svg",
    "icons/swap.svg",
    "icons/up.svg",
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        Ok(find(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|icon| icon.starts_with(path))
            .map(|icon| SharedString::from(*icon))
            .collect())
    }
}
