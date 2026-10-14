//! User-scoped Windows storage and named-pipe transport.
use crate::ReviewResult;
use std::{
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    System::Threading::*,
};

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn descriptor() -> ReviewResult<Descriptor> {
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err("security_unavailable".into());
        }
        let mut size = 0;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut size);
        let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        );
        CloseHandle(token);
        if ok == 0 {
            return Err("security_unavailable".into());
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err("security_unavailable".into());
        }
        let mut len = 0;
        while *sid.add(len) != 0 {
            len += 1;
        }
        let sid_text = String::from_utf16_lossy(std::slice::from_raw_parts(sid, len));
        LocalFree(sid.cast());
        let sddl = wide(&format!("O:{sid_text}D:P(A;OICI;GA;;;{sid_text})"));
        let mut descriptor = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        ) == 0
        {
            return Err("security_unavailable".into());
        }
        Ok(Descriptor(descriptor))
    }
}

pub fn protect_directory(path: &Path) -> ReviewResult<()> {
    use std::os::windows::fs::MetadataExt;
    if std::fs::symlink_metadata(path)
        .map_err(|_| "metrics_unavailable")?
        .file_attributes()
        & 0x400
        != 0
    {
        return Err("unsafe_metrics_path".into());
    }
    let sd = descriptor()?;
    let name = wide(&path.to_string_lossy());
    unsafe {
        let mut acl = null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        if GetSecurityDescriptorDacl(sd.0, &mut present, &mut acl, &mut defaulted) == 0
            || present == 0
        {
            return Err("security_unavailable".into());
        }
        if SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        ) != 0
        {
            return Err("security_unavailable".into());
        }
    }
    Ok(())
}

#[cfg(feature = "judge")]
pub fn serve() -> ReviewResult<()> {
    use crate::{
        store::{fingerprint, private_dir, profile_dir},
        worker::{Request, Worker},
    };
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Storage::FileSystem::*,
        System::{IO::*, Pipes::*},
    };
    let profile = profile_dir()?;
    let directory = profile.join("craig");
    private_dir(&directory)?;
    let lock_path = directory.join("worker.lock");
    if lock_path.is_symlink() {
        return Err("unsafe_metrics_path".into());
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|_| "worker_unavailable")?;
    fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| "worker_already_running")?;
    let normalized = profile
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .to_lowercase();
    let name = wide(&format!(
        "\\\\.\\pipe\\craig-{}",
        fingerprint(normalized.as_bytes())
    ));
    let sd = descriptor()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    let worker = Worker::new(&profile)?;
    let stopping = worker.clone();
    ctrlc::set_handler(move || {
        stopping.stopping.store(true, Ordering::Release);
        let _ = stopping.cancel(None);
    })
    .map_err(|_| "worker_unavailable")?;
    let runner = worker.clone();
    let thread = std::thread::spawn(move || runner.run());
    let result = (|| -> ReviewResult<()> {
        let mut activity = Instant::now();
        while !worker.stopping.load(Ordering::Acquire) {
            let pipe = unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    16_777_216,
                    1_048_576,
                    0,
                    &attributes,
                )
            }; // 16 MiB output, 1 MiB input
            if pipe == INVALID_HANDLE_VALUE {
                return Err("worker_unavailable".into());
            }
            while !worker.stopping.load(Ordering::Acquire) {
                let connected = unsafe {
                    ConnectNamedPipe(pipe, null_mut()) != 0
                        || GetLastError() == ERROR_PIPE_CONNECTED
                };
                if connected {
                    activity = Instant::now();
                    let mut input = Vec::new();
                    let mut buffer = [0u8; 8192]; // 8 KiB
                    while activity.elapsed() < Duration::from_secs(3) && input.len() <= 1_048_576 {
                        let mut count = 0;
                        let ok = unsafe {
                            ReadFile(
                                pipe,
                                buffer.as_mut_ptr(),
                                buffer.len() as u32,
                                &mut count,
                                null_mut(),
                            )
                        };
                        if count > 0 {
                            input.extend_from_slice(&buffer[..count as usize]);
                            if input.contains(&b'\n') {
                                break;
                            }
                        } else if ok == 0 && unsafe { GetLastError() } != ERROR_NO_DATA {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    let response = if input.len() > 1_048_576 || !input.ends_with(b"\n") {
                        Err("invalid_request".into())
                    } else {
                        serde_json::from_slice::<Request>(&input)
                            .map_err(|_| "invalid_request".into())
                            .and_then(|r| worker.handle(r))
                    };
                    let value = match response {
                        Ok(value) => serde_json::json!({"version":1,"status":"ok","value":value}),
                        Err(code) => serde_json::json!({"version":1,"status":"error","code":code}),
                    };
                    let bytes = (value.to_string() + "\n").into_bytes();
                    let mut count = 0;
                    unsafe {
                        WriteFile(
                            pipe,
                            bytes.as_ptr(),
                            bytes.len() as u32,
                            &mut count,
                            null_mut(),
                        );
                    }
                    // Drain the reply before closing, but never let a client that
                    // stops reading hold the worker indefinitely.
                    let current = unsafe { OpenThread(THREAD_TERMINATE, 0, GetCurrentThreadId()) };
                    if !current.is_null() {
                        let pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
                        let signal = pending.clone();
                        let handle = current as usize;
                        let watchdog = std::thread::spawn(move || {
                            std::thread::park_timeout(Duration::from_secs(3));
                            if signal.swap(false, Ordering::AcqRel) {
                                unsafe {
                                    CancelSynchronousIo(handle as HANDLE);
                                }
                            }
                            unsafe {
                                CloseHandle(handle as HANDLE);
                            }
                        });
                        unsafe {
                            FlushFileBuffers(pipe);
                        }
                        pending.store(false, Ordering::Release);
                        watchdog.thread().unpark();
                        let _ = watchdog.join();
                    }
                    break;
                }
                if activity.elapsed() > Duration::from_secs(60) && !worker.busy() {
                    worker.stopping.store(true, Ordering::Release);
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            unsafe {
                CloseHandle(pipe);
            }
        }
        Ok(())
    })();
    worker.stopping.store(true, Ordering::Release);
    let _ = worker.cancel(None);
    let _ = thread.join();
    result
}
