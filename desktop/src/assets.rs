use gpui::{AssetSource, SharedString};
use std::borrow::Cow;
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(match path {
            "icons/plane.svg" => Some(Cow::Borrowed(include_bytes!("../assets/plane.svg"))),
            "icons/inbox.svg" => Some(Cow::Borrowed(include_bytes!("../assets/inbox.svg"))),
            "icons/clock.svg" => Some(Cow::Borrowed(include_bytes!("../assets/clock.svg"))),
            "icons/file.svg" => Some(Cow::Borrowed(include_bytes!("../assets/file.svg"))),
            "icons/star.svg" => Some(Cow::Borrowed(include_bytes!("../assets/star.svg"))),
            "icons/folder.svg" => Some(Cow::Borrowed(include_bytes!("../assets/folder.svg"))),
            "icons/trash.svg" => Some(Cow::Borrowed(include_bytes!("../assets/trash.svg"))),
            "icons/archive.svg" => Some(Cow::Borrowed(include_bytes!("../assets/archive.svg"))),
            "icons/compose.svg" => Some(Cow::Borrowed(include_bytes!("../assets/compose.svg"))),
            "icons/settings.svg" => Some(Cow::Borrowed(include_bytes!("../assets/settings.svg"))),
            "icons/refresh.svg" => Some(Cow::Borrowed(include_bytes!("../assets/refresh.svg"))),
            "icons/copy.svg" => Some(Cow::Borrowed(include_bytes!("../assets/copy.svg"))),
            "icons/reply.svg" => Some(Cow::Borrowed(include_bytes!("../assets/reply.svg"))),
            "icons/search.svg" => Some(Cow::Borrowed(include_bytes!("../assets/search.svg"))),
            "icons/envelope.svg" => Some(Cow::Borrowed(include_bytes!("../assets/envelope.svg"))),
            "icons/more.svg" => Some(Cow::Borrowed(include_bytes!("../assets/more.svg"))),
            "icons/check.svg" => Some(Cow::Borrowed(include_bytes!("../assets/check.svg"))),
            "icons/chevron-down.svg" => {
                Some(Cow::Borrowed(include_bytes!("../assets/chevron-down.svg")))
            }
            "icons/chevron-right.svg" => {
                Some(Cow::Borrowed(include_bytes!("../assets/chevron-right.svg")))
            }
            "icons/x.svg" => Some(Cow::Borrowed(include_bytes!("../assets/x.svg"))),
            "icons/eye.svg" => Some(Cow::Borrowed(include_bytes!("../assets/eye.svg"))),
            "icons/eye-off.svg" => Some(Cow::Borrowed(include_bytes!("../assets/eye-off.svg"))),
            "icons/plus.svg" => Some(Cow::Borrowed(include_bytes!("../assets/plus.svg"))),
            "icons/arrow-down.svg" => {
                Some(Cow::Borrowed(include_bytes!("../assets/arrow-down.svg")))
            }
            "icons/language.svg" => Some(Cow::Borrowed(include_bytes!("../assets/language.svg"))),
            _ => None,
        })
    }
    fn list(&self, _path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}
