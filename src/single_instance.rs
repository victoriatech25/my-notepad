//! 단일 인스턴스 (PLAN.md §7 결정, P5-04).
//!
//! 앱이 이미 떠 있으면 새 창을 띄우지 않고, 받은 파일을 기존 창에 넘겨 새 탭으로 연 뒤 끝난다.
//! - 기준: 같은 사용자·같은 로그온 세션·**같은 실행 파일 경로** — 설치본과 포터블·개발 빌드는 서로 따로 뜬다
//! - 파일 없이 실행해도 기존 창을 앞으로 가져오기만 한다 (Notepad++와 같음). `--new-window`면 이 동작을 건너뛴다
//! - Windows: 이름 있는 뮤텍스(`Local\…`)로 첫 인스턴스를 가리고, 이름 있는 파이프(`\\.\pipe\…`)로 경로를 넘긴다.
//!   보내는 쪽이 `AllowSetForegroundWindow`로 받는 쪽에 포커스 권한을 넘겨, 최소화된 창도 복원해 앞으로 나온다.
//!   탐색기에서 여러 파일을 고르면 프로세스가 파일마다 거의 동시에 뜨므로, 파이프가 바쁘면 잠시 기다렸다 다시 붙는다
//! - macOS: `.app` 번들은 OS가 이미 하나만 띄운다 (Finder 파일 열기 이벤트는 P5-03). 그 밖의 OS에서는 아무것도 하지 않는다
//!
//! 메시지(바이트): `[파일 수 u32 LE]` + 파일마다 `[UTF-16 길이 u32 LE][UTF-16 LE]`, 받는 쪽은 1바이트로 응답한다.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

/// 시작할 때 고른 역할.
pub enum Startup {
    /// 이 프로세스가 창을 띄운다. 다른 인스턴스가 보내는 파일은 [`Listener`]로 받는다
    Primary(Listener),
    /// 기존 창에 파일을 넘겼다 — 창을 띄우지 않고 끝낸다
    Forwarded,
}

/// 다른 인스턴스가 넘긴 요청을 받는 쪽. 앱이 매 프레임 [`Listener::drain`]으로 꺼낸다.
pub struct Listener {
    requests: Option<Receiver<Vec<PathBuf>>>,
    target: Arc<Mutex<Target>>,
    /// 첫 인스턴스 표시 — 프로세스가 끝날 때까지 쥐고 있는다
    #[cfg(windows)]
    _mutex: Option<win::Handle>,
}

/// 요청을 받았을 때 깨울 화면과 앞으로 가져올 창
#[derive(Default)]
struct Target {
    ctx: Option<eframe::egui::Context>,
    #[cfg(windows)]
    hwnd: Option<isize>,
}

impl Listener {
    /// 아무것도 받지 않는 리스너 (`--new-window`, 측정 실행, 지원하지 않는 OS)
    pub fn none() -> Self {
        Self {
            requests: None,
            target: Arc::default(),
            #[cfg(windows)]
            _mutex: None,
        }
    }

    /// 창이 만들어진 뒤 부른다 — 요청이 오면 이 화면을 다시 그리고 창을 앞으로 가져온다.
    pub fn attach(&self, cc: &eframe::CreationContext<'_>) {
        let mut target = self.target.lock().unwrap_or_else(|e| e.into_inner());
        target.ctx = Some(cc.egui_ctx.clone());
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            target.hwnd = cc
                .window_handle()
                .ok()
                .and_then(|handle| match handle.as_raw() {
                    RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
                    _ => None,
                });
        }
    }

    /// 지금까지 받은 요청의 파일을 받은 순서대로 꺼낸다. 파일 없는 요청(창만 앞으로)은 빈 목록이다.
    pub fn drain(&self) -> Vec<PathBuf> {
        self.requests
            .as_ref()
            .map(|rx| rx.try_iter().flatten().collect())
            .unwrap_or_default()
    }
}

/// 이미 떠 있는 인스턴스가 있으면 `files`를 넘기고 [`Startup::Forwarded`], 없으면 이 프로세스가 첫 인스턴스가 된다.
/// 넘기지 못하면(기존 인스턴스가 끝나는 중 등) 로그를 남기고 이 프로세스가 창을 띄운다.
pub fn acquire(files: &[PathBuf]) -> Startup {
    #[cfg(windows)]
    {
        let name = win::instance_name();
        win::acquire_named(&name, files)
    }
    #[cfg(not(windows))]
    {
        let _ = files;
        Startup::Primary(Listener::none())
    }
}

/// 요청 메시지로 바꾼다.
pub fn encode(files: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for file in files {
        let wide = wide_of(file);
        out.extend_from_slice(&(wide.len() as u32).to_le_bytes());
        for unit in wide {
            out.extend_from_slice(&unit.to_le_bytes());
        }
    }
    out
}

/// 메시지 하나를 읽는다. 길이가 터무니없거나 중간에 끊기면 오류.
pub fn decode(reader: &mut impl std::io::Read) -> std::io::Result<Vec<PathBuf>> {
    use std::io::{Error, ErrorKind};
    /// 탐색기가 한 번에 넘길 수 있는 수보다 넉넉하게
    const MAX_FILES: u32 = 4096;
    /// Windows 긴 경로 한도 (UTF-16 단위)
    const MAX_LEN: u32 = 32_767;

    let read_u32 = |reader: &mut dyn std::io::Read| -> std::io::Result<u32> {
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    };
    let count = read_u32(reader)?;
    if count > MAX_FILES {
        return Err(Error::new(ErrorKind::InvalidData, "파일 수가 너무 많음"));
    }
    let mut files = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let len = read_u32(reader)?;
        if len == 0 || len > MAX_LEN {
            return Err(Error::new(ErrorKind::InvalidData, "경로 길이가 잘못됨"));
        }
        let mut bytes = vec![0u8; len as usize * 2];
        reader.read_exact(&mut bytes)?;
        let wide: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| u16::from_le_bytes(b))
            .collect();
        files.push(path_of(&wide));
    }
    Ok(files)
}

#[cfg(windows)]
fn wide_of(path: &std::path::Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().collect()
}

#[cfg(not(windows))]
fn wide_of(path: &std::path::Path) -> Vec<u16> {
    path.to_string_lossy().encode_utf16().collect()
}

#[cfg(windows)]
fn path_of(wide: &[u16]) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_wide(wide))
}

#[cfg(not(windows))]
fn path_of(wide: &[u16]) -> PathBuf {
    PathBuf::from(String::from_utf16_lossy(wide))
}

/// 받은 요청을 앱에 넘기고 화면을 깨운다.
fn deliver(tx: &Sender<Vec<PathBuf>>, target: &Mutex<Target>, files: Vec<PathBuf>) {
    let _ = tx.send(files);
    let target = target.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(windows)]
    if let Some(hwnd) = target.hwnd {
        win::bring_to_front(hwnd);
    }
    if let Some(ctx) = &target.ctx {
        ctx.request_repaint();
    }
}

#[cfg(windows)]
mod win {
    use super::{Listener, Startup, Target, deliver, encode};
    use std::ffi::c_void;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::PathBuf;
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_ALREADY_EXISTS: u32 = 183;
    const ERROR_PIPE_BUSY: i32 = 231;
    const ERROR_PIPE_CONNECTED: u32 = 535;
    const INVALID_HANDLE_VALUE: isize = -1;
    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_TYPE_BYTE: u32 = 0x0000_0000;
    const PIPE_WAIT: u32 = 0x0000_0000;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
    const PIPE_UNLIMITED_INSTANCES: u32 = 255;
    const SW_RESTORE: i32 = 9;

    /// 기존 인스턴스의 파이프가 생길 때까지(또는 바쁜 동안) 기다리는 최대 시간
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
    const RETRY_DELAY: Duration = Duration::from_millis(20);

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateMutexW(attributes: *const c_void, initial_owner: i32, name: *const u16) -> isize;
        fn CloseHandle(handle: isize) -> i32;
        fn GetLastError() -> u32;
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: u32,
            pipe_mode: u32,
            max_instances: u32,
            out_buffer_size: u32,
            in_buffer_size: u32,
            default_timeout: u32,
            attributes: *const c_void,
        ) -> isize;
        fn ConnectNamedPipe(pipe: isize, overlapped: *mut c_void) -> i32;
        fn DisconnectNamedPipe(pipe: isize) -> i32;
        fn GetNamedPipeServerProcessId(pipe: isize, server_process_id: *mut u32) -> i32;
        fn GetCurrentProcessId() -> u32;
        fn ProcessIdToSessionId(process_id: u32, session_id: *mut u32) -> i32;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn AllowSetForegroundWindow(process_id: u32) -> i32;
        fn IsIconic(hwnd: isize) -> i32;
        fn ShowWindow(hwnd: isize, cmd_show: i32) -> i32;
        fn SetForegroundWindow(hwnd: isize) -> i32;
    }

    /// 닫을 때 `CloseHandle`하는 핸들
    pub struct Handle(isize);

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn to_wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 사용자·세션·실행 파일 경로마다 다른 이름
    pub fn instance_name() -> String {
        let mut session = 0u32;
        unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let user = std::env::var("USERNAME").unwrap_or_default().to_lowercase();
        format!(
            "VicNotepad-{session}-{:016x}",
            fnv1a(user.as_bytes().iter().chain(b"\0").chain(exe.as_bytes()))
        )
    }

    /// 64비트 FNV-1a — 실행할 때마다 같은 값이어야 하므로 `DefaultHasher`(Rust 버전마다 달라질 수 있음) 대신 쓴다
    fn fnv1a<'a>(bytes: impl Iterator<Item = &'a u8>) -> u64 {
        bytes.fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
            (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    fn pipe_path(name: &str) -> String {
        format!(r"\\.\pipe\{name}")
    }

    pub fn acquire_named(name: &str, files: &[PathBuf]) -> Startup {
        let mutex_name = to_wide(&format!(r"Local\{name}"));
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
        let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let mutex = (mutex != 0).then_some(Handle(mutex));
        if mutex.is_none() {
            log::warn!("단일 인스턴스 뮤텍스를 만들지 못함 — 새 창으로 실행");
            return Startup::Primary(Listener::none());
        }

        if already_running {
            match forward(name, files) {
                Ok(()) => {
                    log::info!("실행 중인 창에 파일 {}개를 넘김", files.len());
                    return Startup::Forwarded;
                }
                Err(e) => {
                    log::warn!("실행 중인 창에 파일을 넘기지 못함 — 새 창으로 실행: {e}");
                    return Startup::Primary(Listener::none());
                }
            }
        }

        let (tx, rx) = channel();
        let target = Arc::new(Mutex::new(Target::default()));
        match serve(name, tx, Arc::clone(&target)) {
            Ok(()) => Startup::Primary(Listener {
                requests: Some(rx),
                target,
                _mutex: mutex,
            }),
            Err(e) => {
                log::warn!("단일 인스턴스 파이프를 만들지 못함 — 다른 실행은 새 창으로 뜬다: {e}");
                Startup::Primary(Listener {
                    requests: None,
                    target,
                    _mutex: mutex,
                })
            }
        }
    }

    /// 첫 파이프 인스턴스를 지금 만들고(창을 띄우기 전에 받을 준비), 이후 연결은 스레드가 차례로 받는다.
    fn serve(
        name: &str,
        tx: Sender<Vec<PathBuf>>,
        target: Arc<Mutex<Target>>,
    ) -> std::io::Result<()> {
        let path = to_wide(&pipe_path(name));
        let first = create_pipe(&path, true)?;
        std::thread::Builder::new()
            .name("single-instance".into())
            .spawn(move || {
                let mut pipe = Some(first);
                loop {
                    let current = match pipe.take() {
                        Some(p) => p,
                        None => match create_pipe(&path, false) {
                            Ok(p) => p,
                            Err(e) => {
                                log::warn!("단일 인스턴스 파이프 재생성 실패 — 받기 중단: {e}");
                                return;
                            }
                        },
                    };
                    handle_client(current, &tx, &target);
                }
            })?;
        Ok(())
    }

    fn create_pipe(path: &[u16], first: bool) -> std::io::Result<File> {
        let open_mode = PIPE_ACCESS_DUPLEX
            | if first {
                FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                0
            };
        let handle = unsafe {
            CreateNamedPipeW(
                path.as_ptr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_handle(handle as _) })
    }

    /// 클라이언트 하나를 기다려 메시지를 받고 응답한 뒤 연결을 끊는다.
    fn handle_client(mut pipe: File, tx: &Sender<Vec<PathBuf>>, target: &Mutex<Target>) {
        let raw = pipe.as_raw_handle() as isize;
        let connected = unsafe { ConnectNamedPipe(raw, std::ptr::null_mut()) } != 0
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !connected {
            return;
        }
        match super::decode(&mut pipe) {
            Ok(files) => {
                log::info!("다른 실행에서 파일 {}개를 받음", files.len());
                deliver(tx, target, files);
                let _ = pipe.write_all(&[1]);
                let _ = pipe.flush();
                // 끊기 전에 보낸 쪽이 응답을 읽고 닫을 때까지 기다린다 — 먼저 끊으면 읽지 않은 응답이 버려진다
                let _ = pipe.read(&mut [0u8; 1]);
            }
            Err(e) => log::warn!("단일 인스턴스 메시지를 읽지 못함: {e}"),
        }
        unsafe { DisconnectNamedPipe(raw) };
    }

    /// 기존 인스턴스에 파일을 넘기고 받았다는 응답까지 기다린다.
    pub fn forward(name: &str, files: &[PathBuf]) -> std::io::Result<()> {
        let path = pipe_path(name);
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        let mut pipe = loop {
            match OpenOptions::new().read(true).write(true).open(&path) {
                Ok(pipe) => break pipe,
                Err(e)
                    if Instant::now() < deadline
                        && matches!(
                            e.raw_os_error(),
                            Some(ERROR_FILE_NOT_FOUND | ERROR_PIPE_BUSY)
                        ) =>
                {
                    std::thread::sleep(RETRY_DELAY);
                }
                Err(e) => return Err(e),
            }
        };
        // 탐색기에서 실행된 이 프로세스가 가진 포커스 권한을 기존 창에 넘긴다
        let mut server_pid = 0u32;
        if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle() as isize, &mut server_pid) }
            != 0
        {
            unsafe { AllowSetForegroundWindow(server_pid) };
        }
        pipe.write_all(&encode(files))?;
        pipe.flush()?;
        let mut ack = [0u8; 1];
        pipe.read_exact(&mut ack)?;
        Ok(())
    }

    /// 최소화돼 있으면 복원하고 앞으로 가져온다.
    pub fn bring_to_front(hwnd: isize) {
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::time::Duration;

        fn unique_name(tag: &str) -> String {
            format!(
                "VicNotepad-test-{tag}-{}-{:?}",
                std::process::id(),
                Instant::now()
            )
            .replace([' ', ':', '.'], "_")
        }

        fn recv_files(listener: &Listener, want: usize) -> Vec<PathBuf> {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut got = Vec::new();
            while got.len() < want && Instant::now() < deadline {
                got.extend(listener.drain());
                std::thread::sleep(Duration::from_millis(10));
            }
            got
        }

        #[test]
        fn second_instance_forwards_files_to_first() {
            let name = unique_name("forward");
            let Startup::Primary(listener) = acquire_named(&name, &[]) else {
                panic!("첫 실행은 창을 띄워야 함");
            };
            let files = vec![
                PathBuf::from(r"C:\메모\첫째.txt"),
                PathBuf::from(r"D:\b 😀.md"),
            ];
            assert!(matches!(acquire_named(&name, &files), Startup::Forwarded));
            assert_eq!(recv_files(&listener, 2), files);
        }

        #[test]
        fn many_simultaneous_senders_all_arrive() {
            let name = unique_name("burst");
            let Startup::Primary(listener) = acquire_named(&name, &[]) else {
                panic!("첫 실행은 창을 띄워야 함");
            };
            // 탐색기 여러 파일 선택: 프로세스가 파일마다 거의 동시에 뜬다
            let senders: Vec<_> = (0..15)
                .map(|i| {
                    let name = name.clone();
                    std::thread::spawn(move || {
                        let file = PathBuf::from(format!(r"C:\f{i}.txt"));
                        matches!(acquire_named(&name, &[file]), Startup::Forwarded)
                    })
                })
                .collect();
            assert!(senders.into_iter().all(|s| s.join().unwrap()));
            let mut got = recv_files(&listener, 15);
            got.sort();
            let mut want: Vec<_> = (0..15)
                .map(|i| PathBuf::from(format!(r"C:\f{i}.txt")))
                .collect();
            want.sort();
            assert_eq!(got, want);
        }

        #[test]
        fn no_files_still_forwards() {
            let name = unique_name("empty");
            let Startup::Primary(_listener) = acquire_named(&name, &[]) else {
                panic!("첫 실행은 창을 띄워야 함");
            };
            assert!(matches!(acquire_named(&name, &[]), Startup::Forwarded));
        }

        #[test]
        fn instance_name_is_stable() {
            assert_eq!(instance_name(), instance_name());
            assert!(instance_name().starts_with("VicNotepad-"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn message_round_trip_keeps_paths() {
        let files = vec![
            PathBuf::from("C:/메모/한글 이름.txt"),
            PathBuf::from("/tmp/emoji 😀.md"),
        ];
        let bytes = encode(&files);
        assert_eq!(decode(&mut bytes.as_slice()).unwrap(), files);
        assert!(decode(&mut encode(&[]).as_slice()).unwrap().is_empty());
    }

    #[test]
    fn truncated_or_bogus_message_is_rejected() {
        let bytes = encode(&[PathBuf::from("a.txt")]);
        assert!(decode(&mut &bytes[..bytes.len() - 1]).is_err());
        assert!(decode(&mut u32::MAX.to_le_bytes().as_slice()).is_err());
        let mut zero_len = 1u32.to_le_bytes().to_vec();
        zero_len.extend_from_slice(&0u32.to_le_bytes());
        assert!(decode(&mut zero_len.as_slice()).is_err());
    }

    #[test]
    fn listener_none_drains_nothing() {
        assert!(Listener::none().drain().is_empty());
    }

    #[test]
    fn deliver_reaches_listener() {
        let (tx, rx) = channel();
        let target = Arc::new(Mutex::new(Target::default()));
        let listener = Listener {
            requests: Some(rx),
            target: Arc::clone(&target),
            #[cfg(windows)]
            _mutex: None,
        };
        deliver(&tx, &target, vec![PathBuf::from("x.txt")]);
        deliver(&tx, &target, vec![]);
        assert_eq!(listener.drain(), [PathBuf::from("x.txt")]);
    }
}
