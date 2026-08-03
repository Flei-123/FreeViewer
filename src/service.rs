//! Windows service - "reachable even when nobody is logged in".
//!
//! A service alone cannot do the job: it lives in session 0, which has no
//! desktop, so neither DXGI Desktop Duplication nor SendInput would see
//! anything. This is the same split TeamViewer uses:
//!
//!   * the **service** (`--service`) watches which session is on the console
//!     and whether the lock screen is up,
//!   * it starts an **agent** (`--tray --agent`) inside that session, on the
//!     right desktop (`Default` while somebody is logged in, `Winlogon` for
//!     the lock/login screen), with the right token (the user's token, or
//!     winlogon's SYSTEM token when nobody is logged in),
//!   * when the desktop changes, the agent is restarted on the new one.
//!
//! Because the agent may run as SYSTEM its configuration cannot live in a
//! user profile - installing the service moves identity, password and address
//! book to `%ProgramData%\FreeViewer` so the FreeViewer ID stays the same no
//! matter who (or what) runs the host.

#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::{anyhow, Result};

pub const SERVICE_NAME: &str = "FreeViewer";
pub const DISPLAY_NAME: &str = "FreeViewer Fernwartung";
pub const DESCRIPTION: &str =
    "Haelt FreeViewer erreichbar - auch am Anmeldebildschirm und wenn niemand angemeldet ist.";

/// What the agent publishes for a GUI that does not run the host itself.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Published {
    pub id: String,
    pub password: String,
    /// unix seconds of the last update - lets the GUI spot a dead agent
    pub at: u64,
    pub desktop: String,
}

pub fn state_file() -> PathBuf {
    crate::ident::config_dir().join("state.json")
}

/// Reads what the running agent published (id and password to show).
pub fn published() -> Option<Published> {
    let raw = std::fs::read_to_string(state_file()).ok()?;
    serde_json::from_str(&raw).ok()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Keeps `state.json` up to date so the user's GUI can show ID and password
/// even though the host is running in another account.
pub fn publish_loop(shared: std::sync::Arc<crate::shared::Shared>) {
    std::thread::spawn(move || {
        let mut last = String::new();
        loop {
            let id = shared.my_id.lock().unwrap().clone();
            let pw = shared.password.lock().unwrap().clone();
            if !id.is_empty() {
                let p = Published {
                    id: id.clone(),
                    password: pw.clone(),
                    at: now(),
                    desktop: std::env::var("FV_DESKTOP").unwrap_or_default(),
                };
                let key = format!("{}|{}", id, pw);
                let json = serde_json::to_string_pretty(&p).unwrap_or_default();
                // rewrite once a minute anyway, so `at` stays fresh
                if key != last || now() % 60 < 3 {
                    last = key;
                    let _ = std::fs::write(state_file(), json);
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
    });
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};

    use windows::core::{w, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, STILL_ACTIVE};
    use windows::Win32::Security::{
        DuplicateTokenEx, GetTokenInformation, SecurityImpersonation, TokenElevation, TokenPrimary,
        TOKEN_ACCESS_MASK, TOKEN_ALL_ACCESS, TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
    use windows::Win32::System::RemoteDesktop::{
        ProcessIdToSessionId, WTSGetActiveConsoleSessionId, WTSQueryUserToken,
    };
    use windows::Win32::System::Services::*;
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, GetProcessWindowStation, GetThreadDesktop, GetUserObjectInformationW,
        OpenInputDesktop, DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, UOI_NAME,
    };
    use windows::Win32::System::Threading::{
        CreateProcessAsUserW, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId,
        GetExitCodeProcess, OpenProcess, OpenProcessToken, TerminateProcess,
        CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, PROCESS_QUERY_INFORMATION, STARTUPINFOW,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn exe_path() -> String {
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "freeviewer.exe".to_string())
    }

    // ---------------------------------------------------------------- log

    fn log_path() -> PathBuf {
        crate::ident::config_dir().join("service.log")
    }

    pub fn log(line: &str) {
        use std::io::Write;
        let stamp = now();
        let _ = std::fs::create_dir_all(crate::ident::config_dir());
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path())
        {
            let _ = writeln!(f, "{} {}", stamp, line);
        }
    }

    // ------------------------------------------------------------ elevation

    pub fn is_elevated() -> bool {
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut el = TOKEN_ELEVATION::default();
            let mut ret = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut el as *mut _ as *mut c_void),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret,
            )
            .is_ok();
            let _ = CloseHandle(token);
            ok && el.TokenIsElevated != 0
        }
    }

    /// Restarts this binary with the given arguments and an UAC prompt.
    pub fn elevate(args: &str) -> Result<()> {
        let exe = wide(&exe_path());
        let params = wide(args);
        let r = unsafe {
            ShellExecuteW(
                HWND::default(),
                w!("runas"),
                PCWSTR(exe.as_ptr()),
                PCWSTR(params.as_ptr()),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if r.0 as isize > 32 {
            Ok(())
        } else {
            Err(anyhow!("Administrator-Rechte wurden abgelehnt"))
        }
    }

    // -------------------------------------------------------------- machine
    // configuration in ProgramData

    /// Moves identity/password/address book to the machine wide folder so the
    /// service agent (which may run as SYSTEM) keeps the same FreeViewer ID.
    pub fn prepare_machine_config() -> Result<PathBuf> {
        let pd = std::env::var("ProgramData").map_err(|_| anyhow!("ProgramData unbekannt"))?;
        let dir = PathBuf::from(pd).join("FreeViewer");
        std::fs::create_dir_all(&dir)?;
        // let normal users read and write it, otherwise the GUI could not
        // change the password of an installation owned by SYSTEM
        let _ = std::process::Command::new("icacls")
            .arg(dir.display().to_string())
            .arg("/grant")
            .arg("*S-1-5-32-545:(OI)(CI)M")
            .output();

        let old = crate::ident::user_config_dir();
        if old != dir {
            for name in [
                "identity.txt",
                "password.txt",
                "partners.json",
                "name.txt",
                "noupdate",
            ] {
                let src = old.join(name);
                let dst = dir.join(name);
                if src.exists() && !dst.exists() {
                    let _ = std::fs::copy(&src, &dst);
                }
            }
        }
        // unattended access needs a password that survives restarts
        if !dir.join("password.txt").exists() {
            let pw = crate::ident::random_password();
            std::fs::write(dir.join("password.txt"), &pw)?;
        }
        Ok(dir)
    }

    // ---------------------------------------------------------------- SCM

    unsafe fn scm(access: u32) -> Result<SC_HANDLE> {
        Ok(OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), access)?)
    }

    pub fn installed() -> bool {
        unsafe {
            let Ok(m) = scm(SC_MANAGER_CONNECT) else {
                return false;
            };
            let name = wide(SERVICE_NAME);
            let h = OpenServiceW(m, PCWSTR(name.as_ptr()), SERVICE_QUERY_STATUS);
            let ok = h.is_ok();
            if let Ok(h) = h {
                let _ = CloseServiceHandle(h);
            }
            let _ = CloseServiceHandle(m);
            ok
        }
    }

    pub fn running() -> bool {
        unsafe {
            let Ok(m) = scm(SC_MANAGER_CONNECT) else {
                return false;
            };
            let name = wide(SERVICE_NAME);
            let mut out = false;
            if let Ok(h) = OpenServiceW(m, PCWSTR(name.as_ptr()), SERVICE_QUERY_STATUS) {
                let mut st = SERVICE_STATUS::default();
                if QueryServiceStatus(h, &mut st).is_ok() {
                    out = st.dwCurrentState == SERVICE_RUNNING;
                }
                let _ = CloseServiceHandle(h);
            }
            let _ = CloseServiceHandle(m);
            out
        }
    }

    pub fn install() -> Result<()> {
        if !is_elevated() {
            return Err(anyhow!("Administrator-Rechte noetig"));
        }
        prepare_machine_config()?;
        let bin = format!("\"{}\" --service", exe_path());
        unsafe {
            let m = scm(SC_MANAGER_ALL_ACCESS)?;
            let name = wide(SERVICE_NAME);
            let disp = wide(DISPLAY_NAME);
            let binw = wide(&bin);
            let existing = OpenServiceW(m, PCWSTR(name.as_ptr()), SERVICE_ALL_ACCESS);
            let h = match existing {
                Ok(h) => {
                    // keep an older installation pointing at this binary
                    let _ = ChangeServiceConfigW(
                        h,
                        ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                        SERVICE_AUTO_START,
                        SERVICE_ERROR(SERVICE_NO_CHANGE),
                        PCWSTR(binw.as_ptr()),
                        PCWSTR::null(),
                        None,
                        PCWSTR::null(),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        PCWSTR(disp.as_ptr()),
                    );
                    h
                }
                Err(_) => CreateServiceW(
                    m,
                    PCWSTR(name.as_ptr()),
                    PCWSTR(disp.as_ptr()),
                    SERVICE_ALL_ACCESS,
                    SERVICE_WIN32_OWN_PROCESS,
                    SERVICE_AUTO_START,
                    SERVICE_ERROR_NORMAL,
                    PCWSTR(binw.as_ptr()),
                    PCWSTR::null(),
                    None,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    PCWSTR::null(),
                )?,
            };

            let mut descw = wide(DESCRIPTION);
            let desc = SERVICE_DESCRIPTIONW {
                lpDescription: PWSTR(descw.as_mut_ptr()),
            };
            let _ = ChangeServiceConfig2W(
                h,
                SERVICE_CONFIG_DESCRIPTION,
                Some(&desc as *const _ as *const c_void),
            );

            // if we ever crash, come back
            let mut actions = [
                SC_ACTION {
                    Type: SC_ACTION_RESTART,
                    Delay: 5_000,
                },
                SC_ACTION {
                    Type: SC_ACTION_RESTART,
                    Delay: 10_000,
                },
                SC_ACTION {
                    Type: SC_ACTION_RESTART,
                    Delay: 30_000,
                },
            ];
            let fa = SERVICE_FAILURE_ACTIONSW {
                dwResetPeriod: 86_400,
                lpRebootMsg: PWSTR::null(),
                lpCommand: PWSTR::null(),
                cActions: actions.len() as u32,
                lpsaActions: actions.as_mut_ptr(),
            };
            let _ = ChangeServiceConfig2W(
                h,
                SERVICE_CONFIG_FAILURE_ACTIONS,
                Some(&fa as *const _ as *const c_void),
            );

            let _ = StartServiceW(h, None);
            let _ = CloseServiceHandle(h);
            let _ = CloseServiceHandle(m);
        }
        // The service starts the agent in the user's session itself, so an
        // additional autostart entry would only produce a second host that
        // kicks the first one off the relay.
        if crate::autostart::enabled() {
            let _ = crate::autostart::set(false);
            log("Autostart abgeschaltet - der Dienst startet FreeViewer jetzt");
        }
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        if !is_elevated() {
            return Err(anyhow!("Administrator-Rechte noetig"));
        }
        unsafe {
            let m = scm(SC_MANAGER_ALL_ACCESS)?;
            let name = wide(SERVICE_NAME);
            let h = OpenServiceW(m, PCWSTR(name.as_ptr()), SERVICE_ALL_ACCESS)?;
            let mut st = SERVICE_STATUS::default();
            let _ = ControlService(h, SERVICE_CONTROL_STOP, &mut st);
            for _ in 0..30 {
                let mut cur = SERVICE_STATUS::default();
                if QueryServiceStatus(h, &mut cur).is_err() || cur.dwCurrentState == SERVICE_STOPPED
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            DeleteService(h)?;
            let _ = CloseServiceHandle(h);
            let _ = CloseServiceHandle(m);
        }
        // without the service somebody has to start us again after a login
        if !crate::autostart::enabled() {
            let _ = crate::autostart::set(true);
            log("Autostart wieder eingeschaltet");
        }
        Ok(())
    }

    // ------------------------------------------------- where are we running

    /// Name of a window station / desktop handle.
    unsafe fn object_name(h: HANDLE) -> String {
        let mut buf = [0u16; 128];
        let mut need = 0u32;
        if GetUserObjectInformationW(
            h,
            UOI_NAME,
            Some(buf.as_mut_ptr() as *mut c_void),
            (buf.len() * 2) as u32,
            Some(&mut need),
        )
        .is_err()
        {
            return "?".to_string();
        }
        String::from_utf16_lossy(&buf)
            .trim_end_matches('\0')
            .to_string()
    }

    /// Name of the desktop this thread is on.
    pub unsafe fn own_desktop_name() -> String {
        match GetThreadDesktop(GetCurrentThreadId()) {
            Ok(d) => object_name(HANDLE(d.0)),
            Err(_) => String::new(),
        }
    }

    /// Name of the desktop that currently receives keyboard and mouse. This
    /// is the only desktop that can be captured - everything else is blind.
    ///
    /// `Err(code)` is information too: a normal user account is not allowed
    /// to open the secure desktop, so "access denied" means the password
    /// prompt or an UAC dialog is on screen right now.
    pub unsafe fn input_desktop() -> std::result::Result<String, u32> {
        match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_ACCESS_FLAGS(0x0001)) {
            Ok(d) => {
                let n = object_name(HANDLE(d.0));
                let _ = CloseDesktop(d);
                Ok(n)
            }
            Err(e) => Err(e.code().0 as u32 & 0xffff),
        }
    }

    pub unsafe fn input_desktop_name() -> String {
        input_desktop().unwrap_or_default()
    }

    /// Where the service should start the agent. The agent itself keeps this
    /// file up to date, see `watch_desktop`.
    pub fn desktop_file() -> PathBuf {
        crate::ident::config_dir().join("desktop.txt")
    }

    pub fn wanted_desktop() -> String {
        std::fs::read_to_string(desktop_file())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or_else(|| "Default".to_string())
    }

    /// Runs inside the agent: as soon as another desktop owns the input (the
    /// password prompt and UAC live on the secure desktop "Winlogon", the
    /// normal screen and even the lock screen on "Default"), write down where
    /// we have to be and quit. The service restarts us there within a second.
    ///
    /// This is deliberately not guessed from LogonUI or session flags -
    /// Windows moved that around between versions, the input desktop never
    /// lies.
    pub fn watch_desktop() {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let mine = unsafe { own_desktop_name() };
            if mine.is_empty() {
                return;
            }
            let secure = "Winlogon";
            loop {
                let target = match unsafe { input_desktop() } {
                    Ok(name) if !name.is_empty() && !name.eq_ignore_ascii_case(&mine) => {
                        Some(name)
                    }
                    // access denied = the secure desktop has the input and we
                    // are not privileged enough to even look at it
                    Err(5) if !mine.eq_ignore_ascii_case(secure) => Some(secure.to_string()),
                    _ => None,
                };
                if let Some(target) = target {
                    log(&format!(
                        "Eingabe liegt auf {} - Agent wechselt dorthin (war {})",
                        target, mine
                    ));
                    let _ = std::fs::write(desktop_file(), &target);
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    std::process::exit(7);
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
    }

    /// One line describing session, window station, desktop and the desktop
    /// that currently owns the input. Written into the log on agent start -
    /// capture only ever works when our desktop IS the input desktop.
    pub fn desktop_report() -> String {
        unsafe {
            let mut sess = 0u32;
            let _ = ProcessIdToSessionId(GetCurrentProcessId(), &mut sess);
            let station = object_name(HANDLE(GetProcessWindowStation().map(|s| s.0).unwrap_or(std::ptr::null_mut())));
            let desk = match GetThreadDesktop(GetCurrentThreadId()) {
                Ok(d) => object_name(HANDLE(d.0)),
                Err(_) => "?".to_string(),
            };
            let input = match input_desktop() {
                Ok(n) => n,
                Err(c) => format!("nicht zu oeffnen (Fehler {})", c),
            };
            format!(
                "Sitzung {}, Station {}, Desktop {}, Eingabe-Desktop {}",
                sess, station, desk, input
            )
        }
    }

    // ------------------------------------------------------- service itself

    static STOP: AtomicBool = AtomicBool::new(false);
    static WAKE: AtomicBool = AtomicBool::new(false);
    static STATUS_HANDLE: AtomicIsize = AtomicIsize::new(0);
    static CHECKPOINT: AtomicU32 = AtomicU32::new(0);
    /// Handle and pid of the agent we started.
    static AGENT: AtomicIsize = AtomicIsize::new(0);
    static AGENT_PID: AtomicU32 = AtomicU32::new(0);

    fn status_handle() -> SERVICE_STATUS_HANDLE {
        SERVICE_STATUS_HANDLE(STATUS_HANDLE.load(Ordering::Relaxed) as *mut c_void)
    }

    fn set_state(state: SERVICE_STATUS_CURRENT_STATE, wait_hint: u32) {
        let h = status_handle();
        if h.0.is_null() {
            return;
        }
        let accept = if state == SERVICE_RUNNING {
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN | SERVICE_ACCEPT_SESSIONCHANGE
        } else {
            0
        };
        let st = SERVICE_STATUS {
            dwServiceType: SERVICE_WIN32_OWN_PROCESS,
            dwCurrentState: state,
            dwControlsAccepted: accept,
            dwWin32ExitCode: 0,
            dwServiceSpecificExitCode: 0,
            dwCheckPoint: CHECKPOINT.fetch_add(1, Ordering::Relaxed),
            dwWaitHint: wait_hint,
        };
        unsafe {
            let _ = SetServiceStatus(h, &st);
        }
    }

    unsafe extern "system" fn handler(
        control: u32,
        _event_type: u32,
        _event_data: *mut c_void,
        _context: *mut c_void,
    ) -> u32 {
        match control {
            SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
                STOP.store(true, Ordering::Relaxed);
                set_state(SERVICE_STOP_PENDING, 5_000);
            }
            SERVICE_CONTROL_SESSIONCHANGE => {
                // login, logout, lock, unlock - look again right away
                WAKE.store(true, Ordering::Relaxed);
            }
            SERVICE_CONTROL_INTERROGATE => set_state(SERVICE_RUNNING, 0),
            _ => {}
        }
        0
    }

    unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
        let name = wide(SERVICE_NAME);
        let h = match RegisterServiceCtrlHandlerExW(PCWSTR(name.as_ptr()), Some(handler), None) {
            Ok(h) => h,
            Err(e) => {
                log(&format!("handler konnte nicht registriert werden: {}", e));
                return;
            }
        };
        STATUS_HANDLE.store(h.0 as isize, Ordering::Relaxed);
        set_state(SERVICE_START_PENDING, 3_000);
        log("Dienst startet");
        set_state(SERVICE_RUNNING, 0);
        supervise();
        kill_agent();
        log("Dienst beendet");
        set_state(SERVICE_STOPPED, 0);
    }

    pub fn run() -> Result<()> {
        let mut name = wide(SERVICE_NAME);
        let table = [
            SERVICE_TABLE_ENTRYW {
                lpServiceName: PWSTR(name.as_mut_ptr()),
                lpServiceProc: Some(service_main),
            },
            SERVICE_TABLE_ENTRYW::default(),
        ];
        unsafe { StartServiceCtrlDispatcherW(table.as_ptr())? };
        Ok(())
    }

    // ------------------------------------------------------------- the agent


    /// First process with that name inside the given session.
    fn find_process(name: &str, session: u32) -> Option<u32> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
            let mut e = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut found = None;
            if Process32FirstW(snap, &mut e).is_ok() {
                loop {
                    let exe = String::from_utf16_lossy(&e.szExeFile);
                    let exe = exe.trim_end_matches('\0');
                    if exe.eq_ignore_ascii_case(name) {
                        let mut sid = 0u32;
                        if ProcessIdToSessionId(e.th32ProcessID, &mut sid).is_ok()
                            && sid == session
                        {
                            found = Some(e.th32ProcessID);
                            break;
                        }
                    }
                    if Process32NextW(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
            found
        }
    }

    /// A primary token we can start the agent with: the token of the logged in
    /// user, or - on the login screen - winlogon's SYSTEM token.
    unsafe fn session_token(session: u32, secure: bool) -> Result<HANDLE> {
        if !secure {
            let mut t = HANDLE::default();
            if WTSQueryUserToken(session, &mut t).is_ok() {
                return duplicate(t);
            }
        }
        let pid = find_process("winlogon.exe", session)
            .ok_or_else(|| anyhow!("winlogon.exe in Sitzung {} nicht gefunden", session))?;
        let p = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid)?;
        let mut t = HANDLE::default();
        let r = OpenProcessToken(p, TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ACCESS_MASK(0), &mut t);
        let _ = CloseHandle(p);
        r?;
        duplicate(t)
    }

    unsafe fn duplicate(t: HANDLE) -> Result<HANDLE> {
        let mut dup = HANDLE::default();
        let r = DuplicateTokenEx(
            t,
            TOKEN_ALL_ACCESS,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut dup,
        );
        let _ = CloseHandle(t);
        r?;
        Ok(dup)
    }

    fn agent_alive() -> bool {
        let h = AGENT.load(Ordering::Relaxed);
        if h == 0 {
            return false;
        }
        unsafe {
            let mut code = 0u32;
            if GetExitCodeProcess(HANDLE(h as *mut c_void), &mut code).is_err() {
                return false;
            }
            code == STILL_ACTIVE.0 as u32
        }
    }

    pub fn kill_agent() {
        let h = AGENT.swap(0, Ordering::Relaxed);
        AGENT_PID.store(0, Ordering::Relaxed);
        if h == 0 {
            return;
        }
        unsafe {
            let handle = HANDLE(h as *mut c_void);
            let _ = TerminateProcess(handle, 0);
            let _ = CloseHandle(handle);
        }
    }

    unsafe fn spawn_agent(session: u32, desktop: &str) -> Result<u32> {
        // the secure desktop only accepts SYSTEM
        let secure = desktop.eq_ignore_ascii_case("Winlogon");
        let token = session_token(session, secure)?;
        let mut env: *mut c_void = std::ptr::null_mut();
        let have_env = CreateEnvironmentBlock(&mut env, token, false).is_ok();

        let mut deskw = wide(&format!("winsta0\\{}", desktop));
        let mut si = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            lpDesktop: PWSTR(deskw.as_mut_ptr()),
            ..Default::default()
        };
        let mut cmd = wide(&format!("\"{}\" --tray --agent", exe_path()));
        let mut pi = PROCESS_INFORMATION::default();
        let r = CreateProcessAsUserW(
            token,
            PCWSTR::null(),
            PWSTR(cmd.as_mut_ptr()),
            None,
            None,
            false,
            CREATE_UNICODE_ENVIRONMENT,
            if have_env { Some(env) } else { None },
            PCWSTR::null(),
            &si,
            &mut pi,
        );
        if have_env {
            let _ = DestroyEnvironmentBlock(env);
        }
        let _ = CloseHandle(token);
        r?;
        let _ = CloseHandle(pi.hThread);
        AGENT.store(pi.hProcess.0 as isize, Ordering::Relaxed);
        AGENT_PID.store(pi.dwProcessId, Ordering::Relaxed);
        // silence the unused warning on si without dropping it too early
        si.cb = si.cb;
        Ok(pi.dwProcessId)
    }

    /// The heart of the service: keep exactly one agent alive, on the desktop
    /// that is currently receiving input.
    fn supervise() {
        let mut current: Option<(u32, String)> = None;
        let mut last_try = std::time::Instant::now() - std::time::Duration::from_secs(10);
        while !STOP.load(Ordering::Relaxed) {
            let session = unsafe { WTSGetActiveConsoleSessionId() };
            let wanted = if session == 0xFFFF_FFFF {
                None
            } else {
                Some((session, wanted_desktop()))
            };
            let alive = agent_alive();
            let change = wanted != current || !alive;
            if change && wanted.is_some() {
                let (sess, desktop) = wanted.clone().unwrap();
                // never hammer the SCM/desktop when something keeps failing
                if last_try.elapsed() >= std::time::Duration::from_secs(3) {
                    last_try = std::time::Instant::now();
                    kill_agent();
                    match unsafe { spawn_agent(sess, &desktop) } {
                        Ok(pid) => {
                            log(&format!(
                                "Agent gestartet: pid {} in Sitzung {} auf {}",
                                pid, sess, desktop
                            ));
                            current = wanted;
                        }
                        Err(e) => {
                            log(&format!("Agent-Start fehlgeschlagen: {}", e));
                            current = None;
                        }
                    }
                }
            } else if wanted.is_none() && current.is_some() {
                kill_agent();
                current = None;
            }

            // react to lock/unlock within a blink, poll as a safety net
            for _ in 0..20 {
                if STOP.load(Ordering::Relaxed) || WAKE.swap(false, Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

#[cfg(windows)]
pub use imp::{
    desktop_report, elevate, install, installed, is_elevated, kill_agent, log,
    prepare_machine_config, run, running, uninstall, watch_desktop,
};

#[cfg(not(windows))]
mod stub {
    use super::*;
    pub fn installed() -> bool {
        false
    }
    pub fn running() -> bool {
        false
    }
    pub fn is_elevated() -> bool {
        false
    }
    pub fn elevate(_args: &str) -> Result<()> {
        Err(anyhow!("nur unter Windows"))
    }
    pub fn install() -> Result<()> {
        Err(anyhow!("nur unter Windows"))
    }
    pub fn uninstall() -> Result<()> {
        Err(anyhow!("nur unter Windows"))
    }
    pub fn run() -> Result<()> {
        Err(anyhow!("nur unter Windows"))
    }
    pub fn kill_agent() {}
    pub fn log(_s: &str) {}
    /// Auf welchem Eingabe-Desktop laeuft der Agent? Nur Windows kennt das
    /// Problem (Anmeldebildschirm, UAC), anderswo gibt es genau einen.
    pub fn desktop_report() -> String {
        "kein Windows - nur ein Desktop".to_string()
    }
    /// Dem Eingabe-Desktop folgen - anderswo nicht noetig.
    pub fn watch_desktop() {}
    pub fn prepare_machine_config() -> Result<PathBuf> {
        Err(anyhow!("nur unter Windows"))
    }
}

#[cfg(not(windows))]
pub use stub::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_state_survives_a_round_trip() {
        let p = Published {
            id: "497628420".into(),
            password: "abc23xyz".into(),
            at: 1234,
            desktop: "Default".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: Published = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "497628420");
        assert_eq!(back.password, "abc23xyz");
        assert_eq!(back.at, 1234);
    }
}
