//! External OAuth browser with account-specific networking and an owned lifetime.
//! The main UI remains GPUI. System mode uses the user's default browser.
use lightmail_core::{fail, AccountProxyMode, AccountProxySettings, Result};
use std::{
    ffi::{OsStr, OsString},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    System::{JobObjects::*, Threading::*},
    UI::WindowsAndMessaging::SW_SHOWNORMAL,
};

pub struct OAuthBrowser(Option<Session>);
struct Session {
    job: OwnedHandle,
    process: OwnedHandle,
    profile: Option<tempfile::TempDir>,
}
impl OAuthBrowser {
    pub async fn closed(&self) {
        let Some(session) = &self.0 else {
            return;
        };
        let Ok(job) = session.job.try_clone() else {
            return;
        };
        let (reply, closed) = async_channel::bounded::<()>(1);
        let _ = std::thread::Builder::new()
            .name("oauth-browser-watch".into())
            .spawn(move || {
                while !reply.is_closed() {
                    let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                    let valid = unsafe {
                        QueryInformationJobObject(
                            job.as_raw_handle(),
                            JobObjectBasicAccountingInformation,
                            &mut info as *mut _ as _,
                            std::mem::size_of_val(&info) as u32,
                            std::ptr::null_mut(),
                        )
                    };
                    if valid == 0 || info.ActiveProcesses == 0 {
                        let _ = reply.try_send(());
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
        let _ = closed.recv().await;
    }
}

impl Drop for OAuthBrowser {
    fn drop(&mut self) {
        if let Some(session) = self.0.take() {
            // Waiting for browser descendants must never stall GPUI's event thread.
            let _ = std::thread::Builder::new()
                .name("oauth-browser-cleanup".into())
                .spawn(move || drop(session));
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            TerminateJobObject(self.job.as_raw_handle(), 0);
            WaitForSingleObject(self.process.as_raw_handle(), 5000);
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                if QueryInformationJobObject(
                    self.job.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as _,
                    std::mem::size_of_val(&info) as u32,
                    std::ptr::null_mut(),
                ) == 0
                    || info.ActiveProcesses == 0
                    || Instant::now() >= deadline
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        // The profile is unique to this login, never an existing browser profile.
        if let Some(profile) = self.profile.take() {
            let _ = profile.close();
        }
    }
}

fn utf16(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

// CreateProcess receives a command line, not an argv array. Quote every argument
// using the Windows backslash/quote rules, without passing it through a shell.
fn quote(argument: &OsStr) -> Vec<u16> {
    let mut output = vec![b'"' as u16];
    let mut slashes = 0;
    for c in argument.encode_wide() {
        if c == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        output.extend(std::iter::repeat_n(
            b'\\' as u16,
            if c == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        slashes = 0;
        output.push(c);
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
    output
}

fn spawn(program: &Path, args: &[OsString], profile: tempfile::TempDir) -> Result<OAuthBrowser> {
    let application = utf16(program.as_os_str());
    let mut command = quote(program.as_os_str());
    for arg in args {
        command.push(b' ' as u16);
        command.extend(quote(arg));
    }
    command.push(0);
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(fail("无法建立独立的授权浏览器会话"));
        }
        let job = OwnedHandle::from_raw_handle(job);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            &limits as *const _ as _,
            std::mem::size_of_val(&limits) as u32,
        ) == 0
        {
            return Err(fail("无法管理独立授权浏览器的生命周期"));
        }
        let mut startup = STARTUPINFOW::default();
        startup.cb = std::mem::size_of_val(&startup) as u32;
        startup.dwFlags = STARTF_USESHOWWINDOW;
        startup.wShowWindow = SW_SHOWNORMAL as u16;
        let mut child = PROCESS_INFORMATION::default();
        if CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_SUSPENDED | CREATE_NO_WINDOW,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut child,
        ) == 0
        {
            return Err(fail(
                "无法打开独立授权浏览器，请检查 Edge 或 Chrome 是否可用",
            ));
        }
        let process = OwnedHandle::from_raw_handle(child.hProcess);
        let thread = OwnedHandle::from_raw_handle(child.hThread);
        // Assign before its first instruction, so no descendants escape the session.
        if AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) == 0 {
            TerminateProcess(process.as_raw_handle(), 1);
            WaitForSingleObject(process.as_raw_handle(), 5000);
            return Err(fail(
                "无法隔离授权浏览器，未使用系统浏览器代替此邮箱的代理设置",
            ));
        }
        let session = Session {
            job,
            process,
            profile: Some(profile),
        };
        if ResumeThread(thread.as_raw_handle()) == u32::MAX {
            return Err(fail("无法启动授权浏览器"));
        }
        Ok(OAuthBrowser(Some(session)))
    }
}

fn arguments(profile: &Path, url: &str, settings: &AccountProxySettings) -> Result<Vec<OsString>> {
    let settings = settings.clone().validated()?;
    let mut args = vec![
        OsString::from("--new-window"),
        OsString::from("--no-first-run"),
        OsString::from("--no-default-browser-check"),
        OsString::from("--disable-extensions"),
        OsString::from("--disable-background-mode"),
    ];
    let mut directory = OsString::from("--user-data-dir=");
    directory.push(profile);
    args.push(directory);
    match settings.mode {
        AccountProxyMode::System => return Err(fail("系统代理授权使用默认浏览器")),
        AccountProxyMode::Direct => args.push("--no-proxy-server".into()),
        AccountProxyMode::Http | AccountProxyMode::Socks5 => {
            let host = if settings.host.contains(':') {
                format!("[{}]", settings.host)
            } else {
                settings.host
            };
            let scheme = if settings.mode == AccountProxyMode::Http {
                "http"
            } else {
                "socks5"
            };
            args.push(format!("--proxy-server={scheme}://{host}:{}", settings.port).into());
            // Browser-to-app callback always remains on this machine.
            args.push("--proxy-bypass-list=localhost;127.0.0.1;[::1]".into());
        }
    }
    args.push(url.into());
    Ok(args)
}

fn policy_blocks(policy: &str) -> bool {
    use winreg::{
        enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
        RegKey,
    };
    [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
        .into_iter()
        .any(|hive| match RegKey::predef(hive).open_subkey(policy) {
            Ok(key) => [
                "UserDataDir",
                "ProxySettings",
                "ProxyMode",
                "ProxyServer",
                "ProxyPacUrl",
                "ProxyBypassList",
            ]
            .iter()
            .any(|value| key.get_raw_value(value).is_ok()),
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        })
}

fn browser() -> Result<PathBuf> {
    use winreg::{
        enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
        RegKey,
    };
    for (policy, relative) in [
        (
            "SOFTWARE\\Policies\\Microsoft\\Edge",
            "Microsoft/Edge/Application/msedge.exe",
        ),
        (
            "SOFTWARE\\Policies\\Google\\Chrome",
            "Google/Chrome/Application/chrome.exe",
        ),
    ] {
        if policy_blocks(policy) {
            continue;
        }
        let executable = Path::new(relative).file_name().unwrap().to_string_lossy();
        let registration =
            format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{executable}");
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            if let Ok(path) = RegKey::predef(hive)
                .open_subkey(&registration)
                .and_then(|key| key.get_value::<String, _>(""))
            {
                let path = PathBuf::from(path.trim_matches('"'));
                if path.is_file() {
                    return Ok(path);
                }
            }
        }
        for root in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(root) {
                let exe = PathBuf::from(root).join(relative);
                if exe.is_file() {
                    return Ok(exe);
                }
            }
        }
    }
    Err(fail("此邮箱的独立代理授权需要未被代理策略锁定的 Edge 或 Chrome；可改用应用专用密码，或选择跟随系统后重试"))
}

pub fn open(url: &str, settings: &AccountProxySettings) -> Result<OAuthBrowser> {
    let parsed = url::Url::parse(url).map_err(|_| fail("Google 授权地址无效"))?;
    if parsed.scheme() != "https" || parsed.host_str() != Some("accounts.google.com") {
        return Err(fail("Google 授权地址无效"));
    }
    let program = browser()?;
    let profile = tempfile::Builder::new()
        .prefix("lightmail-oauth-")
        .tempdir()
        .map_err(|_| fail("无法建立临时授权浏览器配置"))?;
    let args = arguments(profile.path(), url, settings)?;
    spawn(&program, &args, profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
    };

    #[test]
    fn browser_close_watcher_finishes_when_the_owned_process_exits() {
        let profile = tempfile::Builder::new()
            .prefix("lightmail-close-fixture-")
            .tempdir()
            .unwrap();
        let directory = profile.path().to_path_buf();
        let program = PathBuf::from(std::env::var_os("WINDIR").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let browser = spawn(
            &program,
            &["-NoProfile".into(), "-Command".into(), "exit 0".into()],
            profile,
        )
        .unwrap();
        {
            use std::future::Future;
            let mut closed = Box::pin(browser.closed());
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if closed.as_mut().poll(&mut context).is_ready() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "browser close notification timed out"
                );
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        drop(browser);
        let deadline = Instant::now() + Duration::from_secs(10);
        while directory.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(!directory.exists());
    }

    struct LoopbackBrowserFixture {
        port: u16,
        requests: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    impl LoopbackBrowserFixture {
        fn new(proxy: Option<AccountProxyMode>, callback_port: Option<u16>) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let port = listener.local_addr().unwrap().port();
            let requests = Arc::new(AtomicUsize::new(0));
            let observed = requests.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let worker = std::thread::spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    let Ok((mut peer, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    peer.set_nonblocking(false).unwrap();
                    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    let response = (|| -> std::io::Result<()> {
                        if proxy == Some(AccountProxyMode::Socks5) {
                            let mut greeting = [0; 2];
                            peer.read_exact(&mut greeting)?;
                            let mut methods = vec![0; greeting[1] as usize];
                            peer.read_exact(&mut methods)?;
                            peer.write_all(&[5, 0])?;
                            let mut command = [0; 5];
                            peer.read_exact(&mut command)?;
                            if command[..4] != [5, 1, 0, 3] {
                                return Ok(());
                            }
                            let mut domain = vec![0; command[4] as usize];
                            peer.read_exact(&mut domain)?;
                            let mut endpoint = [0; 2];
                            peer.read_exact(&mut endpoint)?;
                            if domain != b"oauth-route.fixture.invalid"
                                || u16::from_be_bytes(endpoint) != callback_port.unwrap()
                            {
                                peer.write_all(&[5, 2, 0, 1, 127, 0, 0, 1, 0, 0])?;
                                return Ok(());
                            }
                            peer.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])?;
                        }
                        let mut header = Vec::new();
                        let mut byte = [0; 1];
                        while !header.ends_with(b"\r\n\r\n") && header.len() < 16384 {
                            peer.read_exact(&mut byte)?;
                            header.push(byte[0]);
                        }
                        let line = String::from_utf8_lossy(&header);
                        let target = line.split_whitespace().nth(1).unwrap_or("");
                        let path = url::Url::parse(target)
                            .map(|u| u.path().to_owned())
                            .unwrap_or_else(|_| target.to_owned());
                        if proxy.is_some()
                            && !target.contains("oauth-route.fixture.invalid")
                            && proxy != Some(AccountProxyMode::Socks5)
                        {
                            peer.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
                            return Ok(());
                        }
                        if proxy.is_some() && path.starts_with("/authorize") {
                            observed.fetch_add(1, Ordering::SeqCst);
                        }
                        if proxy.is_none() && path.starts_with("/callback") {
                            observed.fetch_add(1, Ordering::SeqCst);
                        }
                        if path.starts_with("/authorize") {
                            let port = callback_port.unwrap_or(port);
                            write!(peer, "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/callback\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
                        } else {
                            peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 17\r\nConnection: close\r\n\r\nsynthetic success")?;
                        }
                        Ok(())
                    })();
                    let _ = response;
                }
            });
            Self {
                port,
                requests,
                stop,
                worker: Some(worker),
            }
        }
    }
    impl Drop for LoopbackBrowserFixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = self.worker.take().unwrap().join();
        }
    }

    #[test]
    #[ignore = "requires installed Edge/Chrome; only loopback synthetic pages"]
    fn installed_browser_honors_account_proxy_and_direct_loopback_callback() {
        let program = browser().expect("No unmanaged Edge/Chrome available for the explicit probe");
        for mode in [
            AccountProxyMode::Http,
            AccountProxyMode::Socks5,
            AccountProxyMode::Direct,
        ] {
            let origin = LoopbackBrowserFixture::new(None, None);
            let proxy = LoopbackBrowserFixture::new(Some(mode), Some(origin.port));
            let profile = tempfile::Builder::new()
                .prefix("lightmail-oauth-browser-probe-")
                .tempdir()
                .unwrap();
            let profile_path = profile.path().to_path_buf();
            let url = if mode == AccountProxyMode::Direct {
                format!("http://127.0.0.1:{}/authorize", origin.port)
            } else {
                format!(
                    "http://oauth-route.fixture.invalid:{}/authorize",
                    origin.port
                )
            };
            let policy = AccountProxySettings {
                mode,
                host: "127.0.0.1".into(),
                port: proxy.port,
            };
            let mut args = arguments(profile.path(), &url, &policy).unwrap();
            args.extend(
                [
                    "--headless=new",
                    "--disable-gpu",
                    "--disable-background-networking",
                    "--disable-component-update",
                    "--dump-dom",
                ]
                .into_iter()
                .map(Into::into),
            );
            let browser = spawn(&program, &args, profile).unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            while origin.requests.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(25));
            }
            let count = origin.requests.load(Ordering::SeqCst);
            drop(browser);
            let deadline = Instant::now() + Duration::from_secs(12);
            while profile_path.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(25));
            }
            assert!(
                count > 0,
                "browser did not return to local callback for {mode:?}"
            );
            assert_eq!(
                proxy.requests.load(Ordering::SeqCst) > 0,
                mode != AccountProxyMode::Direct
            );
            assert!(!profile_path.exists(), "probe browser profile retained");
            println!("Installed browser validated: {mode:?} page route, direct local callback, owned profile cleanup");
        }
    }

    #[test]
    fn browser_proxy_arguments_keep_other_accounts_and_loopback_independent() {
        let profile = Path::new(r"C:\fixture profile\授权");
        let url = "https://accounts.google.com/o/oauth2/v2/auth?state=synthetic%2Bvalue";
        for (mode, flag) in [
            (AccountProxyMode::Direct, "--no-proxy-server"),
            (AccountProxyMode::Http, "--proxy-server=http://[::1]:7897"),
            (
                AccountProxyMode::Socks5,
                "--proxy-server=socks5://[::1]:7897",
            ),
        ] {
            let policy = AccountProxySettings {
                mode,
                host: "::1".into(),
                port: 7897,
            };
            let args = arguments(profile, url, &policy).unwrap();
            assert!(args.iter().any(|arg| arg == flag));
            assert!(args
                .iter()
                .any(|arg| arg.to_string_lossy().starts_with("--user-data-dir=")));
            assert_eq!(args.last().unwrap(), url);
            if mode != AccountProxyMode::Direct {
                assert!(args
                    .iter()
                    .any(|arg| arg == "--proxy-bypass-list=localhost;127.0.0.1;[::1]"));
                assert!(!args
                    .iter()
                    .any(|arg| arg.to_string_lossy().contains("direct://")));
            }
        }
    }

    #[test]
    fn native_command_line_preserves_unicode_quotes_backslashes_and_signed_url() {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::{Foundation::LocalFree, UI::Shell::CommandLineToArgvW};
        let expected: Vec<OsString> = [
            r"C:\fixture program.exe",
            "",
            "中文 空格",
            r"tail\",
            "embedded\"quote",
            "https://accounts.google.com/?x=a%2Bb%3D&state=synthetic",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        let mut line = Vec::new();
        for (i, argument) in expected.iter().enumerate() {
            if i > 0 {
                line.push(b' ' as u16);
            }
            line.extend(quote(argument));
        }
        line.push(0);
        unsafe {
            let mut count = 0;
            let values = CommandLineToArgvW(line.as_ptr(), &mut count);
            assert!(!values.is_null());
            let actual = std::slice::from_raw_parts(values, count as usize)
                .iter()
                .map(|ptr| {
                    let mut len = 0;
                    while *ptr.add(len) != 0 {
                        len += 1;
                    }
                    OsString::from_wide(std::slice::from_raw_parts(*ptr, len))
                })
                .collect::<Vec<_>>();
            LocalFree(values as _);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn owned_browser_job_stops_only_its_fixture_tree_and_removes_its_profile() {
        let profile = tempfile::Builder::new()
            .prefix("lightmail-oauth-fixture-")
            .tempdir()
            .unwrap();
        let directory = profile.path().to_path_buf();
        let marker = directory.join("child-pid.txt");
        let marker_literal = marker.to_string_lossy().replace('\'', "''");
        let program = PathBuf::from(std::env::var_os("WINDIR").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let script = format!("$child=Start-Process -FilePath powershell.exe -WindowStyle Hidden -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 300') -PassThru; [IO.File]::WriteAllText('{marker_literal}',[string]$child.Id); Start-Sleep -Seconds 300");
        let session = spawn(
            &program,
            &["-NoProfile".into(), "-Command".into(), script.into()],
            profile,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        let pid = std::fs::read_to_string(&marker)
            .expect("fixture child did not start")
            .parse::<u32>()
            .unwrap();
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        assert!(!process.is_null());
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        drop(session);
        let deadline = Instant::now() + Duration::from_secs(12);
        while directory.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(!directory.exists(), "owned login profile retained");
        let mut code = 259;
        assert_ne!(
            unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) },
            0
        );
        assert_ne!(code, 259, "fixture browser descendant still running");
    }
}
