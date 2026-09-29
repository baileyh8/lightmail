use gpui::*;
actions!(
    lightmail,
    [
        NewMessage,
        FocusSearch,
        RefreshMail,
        Reply,
        Translate,
        CopyMarkdown,
        Settings,
        ImportMail,
        ClosePanel
    ]
);
pub fn bind(cx: &mut App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{modifier}-n"), NewMessage, Some("Lightmail")),
        KeyBinding::new(&format!("{modifier}-f"), FocusSearch, Some("Lightmail")),
        KeyBinding::new(&format!("{modifier}-r"), RefreshMail, Some("Lightmail")),
        KeyBinding::new(&format!("{modifier}-shift-r"), Reply, Some("Lightmail")),
        KeyBinding::new(&format!("{modifier}-shift-t"), Translate, Some("Lightmail")),
        KeyBinding::new(
            &format!("{modifier}-shift-c"),
            CopyMarkdown,
            Some("Lightmail"),
        ),
        KeyBinding::new(&format!("{modifier}-,"), Settings, Some("Lightmail")),
        KeyBinding::new(&format!("{modifier}-o"), ImportMail, Some("Lightmail")),
        KeyBinding::new("escape", ClosePanel, Some("Lightmail")),
    ]);
}
