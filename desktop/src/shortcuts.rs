use gpui_kit::*;
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
        ClosePanel,
        CopyReaderSelection,
        SelectReaderAll,
        ReaderPageDown,
        ReaderPageUp,
        ReaderStart,
        ReaderEnd
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
        KeyBinding::new("pagedown", ReaderPageDown, Some("BlitzReader")),
        KeyBinding::new("pageup", ReaderPageUp, Some("BlitzReader")),
        KeyBinding::new(
            &format!("{modifier}-home"),
            ReaderStart,
            Some("BlitzReader"),
        ),
        KeyBinding::new(&format!("{modifier}-end"), ReaderEnd, Some("BlitzReader")),
        KeyBinding::new(
            &format!("{modifier}-c"),
            CopyReaderSelection,
            Some("BlitzReader"),
        ),
        KeyBinding::new(
            &format!("{modifier}-a"),
            SelectReaderAll,
            Some("BlitzReader"),
        ),
    ]);
}
