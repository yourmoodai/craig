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

#[cfg(any(test, feature = "judge"))]
fn write_reply(
    pipe: HANDLE,
    bytes: &[u8],
    stopping: &std::sync::atomic::AtomicBool,
    timeout: std::time::Duration,
) -> ReviewResult<()> {
    use std::{sync::atomic::Ordering, time::Instant};
    use windows_sys::Win32::Storage::FileSystem::WriteFile;
    let deadline = Instant::now() + timeout;
    let mut offset = 0;
    while offset < bytes.len() {
        if stopping.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err("review_write_cancelled".into());
        }
        let mut count = 0;
        let ok = unsafe {
            WriteFile(
                pipe,
                bytes[offset..].as_ptr(),
                (bytes.len() - offset) as u32,
                &mut count,
                null_mut(),
            )
        };
        if ok == 0 {
            // ERROR_NO_DATA means the peer is closing; it is not backpressure.
            return Err("review_write_failed".into());
        }
        offset += count as usize;
        if count == 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    Ok(())
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
    let directory = profile.join("savior");
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
        "\\\\.\\pipe\\savior-{}",
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
                    let bytes = crate::transport::response_bytes(&value);
                    // Shutdown must receive its acknowledgement before the worker exits.
                    let acknowledgement = std::sync::atomic::AtomicBool::new(false);
                    let stopping = if worker.stopping.load(Ordering::Acquire) {
                        &acknowledgement
                    } else {
                        &worker.stopping
                    };
                    if write_reply(pipe, &bytes, stopping, Duration::from_secs(3)).is_err() {
                        break;
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

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::{
        io::Read,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{Storage::FileSystem::*, System::Pipes::*};

    fn pair() -> (OwnedHandle, std::fs::File) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "\\\\.\\pipe\\savior-write-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let pipe = unsafe {
            CreateNamedPipeW(
                wide(&name).as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                1024,
                1024,
                0,
                null(),
            )
        }; // 1 KiB buffers force backpressure
        assert_ne!(pipe, INVALID_HANDLE_VALUE);
        let server = unsafe { OwnedHandle::from_raw_handle(pipe) };
        let client = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(name)
            .unwrap();
        let connected = unsafe {
            ConnectNamedPipe(pipe, null_mut()) != 0 || GetLastError() == ERROR_PIPE_CONNECTED
        };
        assert!(connected);
        (server, client)
    }

    #[test]
    fn partial_writes_deliver_the_entire_response() {
        let (server, mut client) = pair();
        let bytes = vec![b'x'; 256 * 1024]; // 256 KiB
        let reader = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let mut result = vec![0; 256 * 1024];
            client.read_exact(&mut result).unwrap();
            result
        });
        let result = write_reply(
            server.as_raw_handle(),
            &bytes,
            &AtomicBool::new(false),
            Duration::from_secs(3),
        );
        // The reader consumes the expected length; flushing prevents close from
        // discarding the final buffered bytes before that read completes.
        if result.is_ok() {
            unsafe {
                FlushFileBuffers(server.as_raw_handle());
            }
        }
        drop(server);
        assert!(result.is_ok());
        assert_eq!(reader.join().unwrap(), bytes);
    }

    #[test]
    fn stalled_disconnected_and_cancelled_readers_are_bounded() {
        let bytes = vec![b'x'; 256 * 1024]; // 256 KiB
        let (server, client) = pair();
        let start = Instant::now();
        assert!(
            write_reply(
                server.as_raw_handle(),
                &bytes,
                &AtomicBool::new(false),
                Duration::from_millis(50)
            )
            .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        drop(client);
        assert!(
            write_reply(
                server.as_raw_handle(),
                &bytes,
                &AtomicBool::new(false),
                Duration::from_secs(1)
            )
            .is_err()
        );
        let (server, _client) = pair();
        assert!(
            write_reply(
                server.as_raw_handle(),
                &bytes,
                &AtomicBool::new(true),
                Duration::from_secs(1)
            )
            .is_err()
        );
    }
}
