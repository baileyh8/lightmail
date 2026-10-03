//! Windows shell integration. All callbacks enqueue commands onto GPUI's UI
//! executor; no mail logic or GPUI borrowing runs inside the native window proc.
use crate::app::MailDesktop;
use gpui_kit::{App, Entity, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::Cell,
    hash::{Hash, Hasher},
    path::Path,
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};

const CALLBACK: u32 = WM_APP + 71;
const ACTIVATE: u32 = WM_APP + 72;
const MENU_COMMAND: u32 = WM_APP + 73;
const SUBCLASS: usize = 0x4c4d;
const OPEN: usize = 1;
const RECEIVE: usize = 2;
const AUTOMATIC: usize = 3;
const EXIT: usize = 4;
// NIN_KEYSELECT is a C macro (NIN_SELECT | NINF_KEY), absent from windows-sys.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

#[derive(Clone, Copy)]
enum Command {
    Open,
    Receive,
    Automatic,
    Exit,
    Active(bool),
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn identity(directory: &Path) -> Vec<u16> {
    let path = directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf());
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.to_string_lossy().to_lowercase().hash(&mut hash);
    wide(&format!("Lightmail.Directory.{:016x}", hash.finish()))
}

fn hwnd(window: &Window) -> anyhow::Result<HWND> {
    match HasWindowHandle::window_handle(window)
        .map_err(|e| anyhow::anyhow!("Window handle unavailable: {e:?}"))?
        .as_raw()
    {
        RawWindowHandle::Win32(raw) => Ok(raw.hwnd.get() as HWND),
        _ => anyhow::bail!("Not a Windows window"),
    }
}

/// A second launch restores the instance holding this exact data directory.
pub fn activate_existing(directory: &Path) -> bool {
    struct Search {
        key: Vec<u16>,
        found: bool,
    }
    unsafe extern "system" fn visit(window: HWND, data: LPARAM) -> i32 {
        let search = &mut *(data as *mut Search);
        if !GetPropW(window, search.key.as_ptr()).is_null() {
            search.found = PostMessageW(window, ACTIVATE, 0, 0) != 0;
            return 0;
        }
        1
    }
    let mut search = Search {
        key: identity(directory),
        found: false,
    };
    unsafe {
        EnumWindows(Some(visit), &mut search as *mut _ as LPARAM);
    }
    search.found
}

struct Tray {
    hwnd: HWND,
    key: Vec<u16>,
    icon: HICON,
    restart: u32,
    installed: Cell<bool>,
    automatic: Cell<bool>,
    attached: Cell<bool>,
    sender: async_channel::Sender<Command>,
}

impl Tray {
    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: CALLBACK,
            hIcon: self.icon,
            ..Default::default()
        };
        let tip = wide(if self.automatic.get() {
            "轻邮 · 自动收信"
        } else {
            "轻邮 · 手动收信"
        });
        data.szTip[..tip.len()].copy_from_slice(&tip);
        data
    }

    fn add(&self) -> bool {
        let mut data = self.data();
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) != 0 };
        self.installed.set(added);
        if added {
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            unsafe {
                Shell_NotifyIconW(NIM_SETVERSION, &data);
            }
        }
        added
    }

    fn remove(&self) {
        if self.installed.replace(false) {
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &self.data());
            }
        }
        if self.attached.replace(false) {
            unsafe {
                RemovePropW(self.hwnd, self.key.as_ptr());
                RemoveWindowSubclass(self.hwnd, Some(subclass), SUBCLASS);
            }
        }
    }

    fn menu(&self, location: WPARAM) {
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            for (id, label, flags) in [
                (OPEN, "打开轻邮", MF_STRING),
                (RECEIVE, "立即收信", MF_STRING),
                (
                    AUTOMATIC,
                    "自动收信",
                    MF_STRING
                        | if self.automatic.get() {
                            MF_CHECKED
                        } else {
                            MF_UNCHECKED
                        },
                ),
                (0, "", MF_SEPARATOR),
                (EXIT, "退出轻邮", MF_STRING),
            ] {
                AppendMenuW(menu, flags, id, wide(label).as_ptr());
            }
            SetMenuDefaultItem(menu, OPEN as u32, 0);
            // Required by the shell for dismissal when clicking outside a menu.
            SetForegroundWindow(self.hwnd);
            let selected = TrackPopupMenuEx(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                location as u16 as i16 as i32,
                (location >> 16) as u16 as i16 as i32,
                self.hwnd,
                std::ptr::null(),
            ) as usize;
            DestroyMenu(menu);
            PostMessageW(self.hwnd, WM_NULL, 0, 0);
            Shell_NotifyIconW(NIM_SETFOCUS, &self.data());
            if selected != 0 {
                PostMessageW(self.hwnd, MENU_COMMAND, selected, 0);
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

unsafe extern "system" fn subclass(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // The Rc lives in both the GPUI task and shutdown hook. Removal precedes
    // release of those owners; callbacks and drop all run on the UI thread.
    let tray = &*(data as *const Tray);
    if message == CALLBACK {
        match lparam as u32 & 0xffff {
            NIN_SELECT | NIN_KEYSELECT => {
                let _ = tray.sender.try_send(Command::Open);
            }
            WM_CONTEXTMENU => tray.menu(wparam),
            _ => {}
        }
        return 0;
    }
    if message == ACTIVATE {
        let _ = tray.sender.try_send(Command::Open);
        return 0;
    }
    if message == MENU_COMMAND {
        let command = match wparam {
            OPEN => Some(Command::Open),
            RECEIVE => Some(Command::Receive),
            AUTOMATIC => Some(Command::Automatic),
            EXIT => Some(Command::Exit),
            _ => None,
        };
        if let Some(command) = command {
            let _ = tray.sender.try_send(command);
        }
        return 0;
    }
    if message == tray.restart {
        if !tray.add() {
            // If Explorer cannot restore the tray, never strand a hidden app.
            let _ = tray.sender.try_send(Command::Open);
        }
    } else if message == WM_ACTIVATE {
        let _ = tray
            .sender
            .try_send(Command::Active(wparam & 0xffff != WA_INACTIVE as usize));
    } else if message == WM_NCDESTROY {
        tray.remove();
    }
    DefSubclassProc(window, message, wparam, lparam)
}

pub fn attach(
    view: &Entity<MailDesktop>,
    window: &mut Window,
    directory: &Path,
    cx: &mut App,
) -> anyhow::Result<()> {
    let hwnd = hwnd(window)?;
    unsafe {
        // Shared module resources live as long as the process; do not DestroyIcon.
        let module = GetModuleHandleW(std::ptr::null());
        for (kind, size) in [(ICON_BIG, 32), (ICON_SMALL, 16)] {
            let icon = LoadImageW(
                module,
                1usize as *const u16,
                IMAGE_ICON,
                size,
                size,
                LR_SHARED,
            );
            anyhow::ensure!(!icon.is_null(), "Application icon resource is missing");
            SendMessageW(hwnd, WM_SETICON, kind as usize, icon as isize);
        }
    }
    let (sender, receiver) = async_channel::unbounded();
    let tray = Rc::new(Tray {
        hwnd,
        key: identity(directory),
        icon: unsafe {
            LoadImageW(
                GetModuleHandleW(std::ptr::null()),
                1usize as *const u16,
                IMAGE_ICON,
                32,
                32,
                LR_SHARED,
            )
        },
        restart: unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) },
        installed: Cell::new(false),
        attached: Cell::new(false),
        automatic: Cell::new(view.read(cx).service.automatic_receiving()),
        sender,
    });
    unsafe {
        anyhow::ensure!(
            SetWindowSubclass(hwnd, Some(subclass), SUBCLASS, Rc::as_ptr(&tray) as usize) != 0,
            "Cannot attach tray callbacks"
        );
        tray.attached.set(true);
        anyhow::ensure!(
            SetPropW(hwnd, tray.key.as_ptr(), 1usize as HANDLE) != 0,
            "Cannot register running instance"
        );
    }
    anyhow::ensure!(tray.add(), "Windows could not create the notification icon");
    let weak = view.downgrade();
    let closing = tray.clone();
    window.on_window_should_close(cx, move |_, cx| {
        if !closing.installed.get() {
            return true;
        }
        let _ = weak.update(cx, |s, cx| {
            s.persist_compose(cx);
            s.service.set_active(false);
            s.record("tray-hide");
        });
        unsafe {
            ShowWindowAsync(closing.hwnd, SW_HIDE);
        }
        false
    });
    let shutdown = tray.clone();
    let weak = view.downgrade();
    cx.on_app_quit(move |cx| {
        shutdown.remove();
        let _ = weak.update(cx, |s, cx| {
            s.persist_compose(cx);
            s.service.stop();
        });
        async {}
    })
    .detach();
    let view = view.downgrade();
    cx.spawn(async move |cx| {
        while let Ok(command) = receiver.recv().await {
            if view
                .update(cx, |s, cx| match command {
                    Command::Open => {
                        unsafe {
                            // This command runs on the window's own UI thread.
                            ShowWindow(
                                tray.hwnd,
                                if IsIconic(tray.hwnd) != 0 {
                                    SW_RESTORE
                                } else {
                                    SW_SHOW
                                },
                            );
                            SetForegroundWindow(tray.hwnd);
                        }
                        s.service.set_active(true);
                        s.record("tray-open");
                    }
                    Command::Receive => s.refresh(cx),
                    Command::Automatic => {
                        let enabled = !s.service.automatic_receiving();
                        match s.set_automatic_receiving(enabled) {
                            Ok(()) => {
                                tray.automatic.set(enabled);
                                unsafe {
                                    Shell_NotifyIconW(NIM_MODIFY, &tray.data());
                                }
                                s.status = if enabled {
                                    "已开启自动收信"
                                } else {
                                    "已切换为手动收信"
                                }
                                .into();
                            }
                            Err(error) => s.status = error.to_string(),
                        }
                        cx.notify();
                    }
                    Command::Exit => {
                        s.persist_compose(cx);
                        s.service.stop();
                        tray.remove();
                        cx.quit();
                    }
                    Command::Active(active) => s.service.set_active(active),
                })
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
    Ok(())
}

#[cfg(feature = "acceptance")]
pub mod acceptance {
    use super::*;

    pub fn verify(window: &Window) -> anyhow::Result<()> {
        let hwnd = hwnd(window)?;
        unsafe {
            for kind in [ICON_SMALL, ICON_BIG] {
                anyhow::ensure!(
                    SendMessageW(hwnd, WM_GETICON, kind as usize, 0) != 0,
                    "Window icon missing"
                );
            }
            let id = NOTIFYICONIDENTIFIER {
                cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: hwnd,
                uID: 1,
                ..Default::default()
            };
            let mut bounds = RECT::default();
            anyhow::ensure!(
                Shell_NotifyIconGetRect(&id, &mut bounds) >= 0,
                "Shell did not register the tray icon"
            );
        }
        Ok(())
    }

    pub fn visible(window: &Window) -> anyhow::Result<bool> {
        Ok(unsafe { IsWindowVisible(hwnd(window)?) != 0 })
    }

    pub fn action(window: &Window, name: &str) -> anyhow::Result<()> {
        let hwnd = hwnd(window)?;
        let (message, command) = match name {
            "close" => (WM_CLOSE, 0),
            "receive" => (MENU_COMMAND, RECEIVE),
            "automatic" => (MENU_COMMAND, AUTOMATIC),
            "exit" => (MENU_COMMAND, EXIT),
            "shell-restart" => {
                // Simulate Explorer losing just our icon, without restarting the
                // user's shell or disturbing other applications.
                let data = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: hwnd,
                    uID: 1,
                    ..Default::default()
                };
                unsafe {
                    Shell_NotifyIconW(NIM_DELETE, &data);
                }
                (
                    unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) },
                    0,
                )
            }
            _ => anyhow::bail!("Unknown tray acceptance action"),
        };
        anyhow::ensure!(
            unsafe { PostMessageW(hwnd, message, command, 0) } != 0,
            "Tray command failed"
        );
        Ok(())
    }
}
