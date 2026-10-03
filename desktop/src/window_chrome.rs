//! Self-drawn Windows title area. The system caption and the Windows 11 outline
//! are hidden; the three controls sit in the top-right corner of the content.
//! Each control reports its Win32 hit-test code, so the pointer gets native
//! behavior (including Windows 11 snap layouts on maximize), and each click
//! handler serves the keyboard and UI Automation. Views mark empty header space
//! with `WindowControlArea::Drag`; that space must not contain other controls.
use gpui_kit::{
    component::{button::*, Icon, IconName},
    *,
};

pub const HEIGHT: f32 = 40.;
pub const WIDTH: f32 = 138.;
/// Right padding for headers that would otherwise run under the controls.
pub const RESERVE: f32 = if cfg!(windows) { WIDTH + 12. } else { 0. };

fn control(
    id: &'static str,
    label: &'static str,
    icon: IconName,
    area: WindowControlArea,
) -> Button {
    Button::new(id)
        .ghost()
        .w(px(WIDTH / 3.))
        .h(px(HEIGHT))
        .p_0()
        .rounded(px(0.))
        .accessibility_label(label)
        .window_control_area(area)
        .child(Icon::new(icon).size(px(12.)))
}

/// Minimize, maximize or restore, and close, with their element ids. Empty where
/// the system draws the window frame.
pub fn controls(window: &Window, cx: &App) -> Vec<(&'static str, Button)> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let maximized = window.is_maximized();
    vec![
        (
            "window-minimize",
            control(
                "window-minimize",
                "最小化",
                IconName::WindowMinimize,
                WindowControlArea::Min,
            )
            .on_click(|_, window, cx| {
                cx.stop_propagation();
                window.minimize_window();
            }),
        ),
        (
            "window-maximize",
            control(
                "window-maximize",
                if maximized { "还原" } else { "最大化" },
                if maximized {
                    IconName::WindowRestore
                } else {
                    IconName::WindowMaximize
                },
                WindowControlArea::Max,
            )
            .on_click(|_, window, cx| {
                cx.stop_propagation();
                // GPUI's zoom only maximizes; keyboard and UIA need a toggle.
                #[cfg(windows)]
                win32::toggle_maximized(window);
                #[cfg(not(windows))]
                window.zoom_window();
            }),
        ),
        (
            "window-close",
            control(
                "window-close",
                "关闭",
                IconName::WindowClose,
                WindowControlArea::Close,
            )
            .custom(
                ButtonCustomVariant::new(cx)
                    .foreground(rgb(0x9a2920).into())
                    .hover(rgb(0xfde7e9).into())
                    .active(rgb(0xf8c5c8).into()),
            )
            .on_click(|_, window, cx| {
                cx.stop_propagation();
                // The same path as Alt+F4, so closing behaves identically.
                #[cfg(windows)]
                win32::request_close(window);
                #[cfg(not(windows))]
                window.remove_window();
            }),
        ),
    ]
}

/// Removes the Windows 11 outline; the shadow and resize frame remain.
pub fn hide_border(window: &Window) {
    #[cfg(windows)]
    win32::hide_border(window);
    #[cfg(not(windows))]
    let _ = window;
}

#[cfg(windows)]
mod win32 {
    use gpui_kit::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;

    #[link(name = "user32")]
    extern "system" {
        fn IsZoomed(hwnd: *mut c_void) -> i32;
        fn ShowWindowAsync(hwnd: *mut c_void, command: i32) -> i32;
        fn PostMessageW(hwnd: *mut c_void, message: u32, wparam: usize, lparam: isize) -> i32;
    }
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut c_void,
            attribute: u32,
            value: *const c_void,
            size: u32,
        ) -> i32;
    }

    // The window lives for the duration of each call, so its handle stays valid.
    fn hwnd(window: &Window) -> Option<*mut c_void> {
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut c_void),
            _ => None,
        }
    }

    pub fn toggle_maximized(window: &Window) {
        const SW_MAXIMIZE: i32 = 3;
        const SW_RESTORE: i32 = 9;
        if let Some(hwnd) = hwnd(window) {
            unsafe {
                let command = if IsZoomed(hwnd) != 0 {
                    SW_RESTORE
                } else {
                    SW_MAXIMIZE
                };
                ShowWindowAsync(hwnd, command);
            }
        }
    }

    pub fn request_close(window: &Window) {
        const WM_CLOSE: u32 = 0x0010;
        if let Some(hwnd) = hwnd(window) {
            unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        }
    }

    pub fn hide_border(window: &Window) {
        const DWMWA_BORDER_COLOR: u32 = 34;
        const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
        if let Some(hwnd) = hwnd(window) {
            // Windows 10 has no such attribute and keeps its default frame.
            unsafe {
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_BORDER_COLOR,
                    &DWMWA_COLOR_NONE as *const u32 as *const c_void,
                    std::mem::size_of::<u32>() as u32,
                )
            };
        }
    }
}
