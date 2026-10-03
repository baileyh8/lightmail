use gpui_kit::{AssetSource, SharedString};
use std::borrow::Cow;

/// Lightmail's own icons first, then Kit's bundled set for component chrome
/// such as input and menu glyphs.
pub struct Assets;

fn own(path: &str) -> Option<&'static [u8]> {
    Some(match path {
        "icons/plane.svg" => include_bytes!("../assets/plane.svg"),
        "icons/inbox.svg" => include_bytes!("../assets/inbox.svg"),
        "icons/clock.svg" => include_bytes!("../assets/clock.svg"),
        "icons/file.svg" => include_bytes!("../assets/file.svg"),
        "icons/star.svg" => include_bytes!("../assets/star.svg"),
        "icons/folder.svg" => include_bytes!("../assets/folder.svg"),
        "icons/trash.svg" => include_bytes!("../assets/trash.svg"),
        "icons/archive.svg" => include_bytes!("../assets/archive.svg"),
        "icons/compose.svg" => include_bytes!("../assets/compose.svg"),
        "icons/settings.svg" => include_bytes!("../assets/settings.svg"),
        "icons/refresh.svg" => include_bytes!("../assets/refresh.svg"),
        "icons/copy.svg" => include_bytes!("../assets/copy.svg"),
        "icons/reply.svg" => include_bytes!("../assets/reply.svg"),
        "icons/search.svg" => include_bytes!("../assets/search.svg"),
        "icons/envelope.svg" => include_bytes!("../assets/envelope.svg"),
        "icons/more.svg" => include_bytes!("../assets/more.svg"),
        "icons/check.svg" => include_bytes!("../assets/check.svg"),
        "icons/chevron-down.svg" => include_bytes!("../assets/chevron-down.svg"),
        "icons/chevron-right.svg" => include_bytes!("../assets/chevron-right.svg"),
        "icons/x.svg" => include_bytes!("../assets/x.svg"),
        "icons/eye.svg" => include_bytes!("../assets/eye.svg"),
        "icons/eye-off.svg" => include_bytes!("../assets/eye-off.svg"),
        "icons/plus.svg" => include_bytes!("../assets/plus.svg"),
        "icons/arrow-down.svg" => include_bytes!("../assets/arrow-down.svg"),
        "icons/language.svg" => include_bytes!("../assets/language.svg"),
        "icons/image-off.svg" => include_bytes!("../assets/image-off.svg"),
        _ => return None,
    })
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        match own(path) {
            Some(bytes) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit::assets::AllAssets.load(path),
        }
    }
    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        gpui_kit::assets::AllAssets.list(path)
    }
}
