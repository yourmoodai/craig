use craig_review::{
    ReviewResult,
    store::{private_dir, profile_dir},
    worker::{Request, Worker},
};
use std::{
    io::{BufRead, Read, Write},
    sync::atomic::Ordering,
};

#[cfg(unix)]
fn serve() -> ReviewResult<()> {
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    let profile = profile_dir()?;
    let directory = profile.join("craig");
    private_dir(&directory)?;
    if directory.join("worker.lock").is_symlink() {
        return Err("unsafe_metrics_path".into());
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("worker.lock"))
        .map_err(|_| "worker_unavailable")?;
    fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| "worker_already_running")?;
    let socket = directory.join("worker.sock");
    if socket.exists() {
        std::fs::remove_file(&socket).map_err(|_| "worker_unavailable")?;
    }
    let listener = UnixListener::bind(&socket).map_err(|_| "worker_unavailable")?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
        .map_err(|_| "worker_unavailable")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "worker_unavailable")?;
    let worker = Worker::new(&profile)?;
    let stopping = worker.clone();
    ctrlc::set_handler(move || {
        stopping.stopping.store(true, Ordering::Release);
        let _ = stopping.cancel(None);
    })
    .map_err(|_| "worker_unavailable")?;
    let thread = {
        let worker = worker.clone();
        std::thread::spawn(move || worker.run())
    };
    let mut activity = std::time::Instant::now();
    while !worker.stopping.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                activity = std::time::Instant::now();
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(3)));
                let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(3)));
                let mut input = Vec::new();
                let result = (|| -> ReviewResult<_> {
                    std::io::BufReader::new((&mut stream).take(1_048_577))
                        .read_until(b'\n', &mut input)
                        .map_err(|_| "invalid_request")?; // 1 MiB plus overflow sentinel
                    if input.len() > 1_048_576 {
                        return Err("invalid_request".into());
                    }
                    let request: Request =
                        serde_json::from_slice(&input).map_err(|_| "invalid_request")?;
                    worker.handle(request)
                })();
                let response = match result {
                    Ok(value) => serde_json::json!({"version":1,"status":"ok","value":value}),
                    Err(code) => serde_json::json!({"version":1,"status":"error","code":code}),
                };
                let _ = writeln!(stream, "{response}");
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(25))
            }
            Err(_) => break,
        }
        if activity.elapsed().as_secs() > 60 && !worker.busy() {
            break;
        }
    }
    worker.stopping.store(true, Ordering::Release);
    worker.cancel(None)?;
    let _ = thread.join();
    let _ = std::fs::remove_file(socket);
    Ok(())
}
#[cfg(not(unix))]
fn serve() -> ReviewResult<()> {
    Err("unsupported_platform".into())
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [command] if command == "serve" => serve(),
        [command, output] if command == "policy-fetch" => {
            craig_review::policy::fetch(std::path::Path::new(output))
        }
        [command, corpus, output] if command == "policy-evaluate" => {
            craig_review::policy::evaluate_file(
                std::path::Path::new(corpus),
                std::path::Path::new(output),
            )
        }
        [command, preset, directory, binary] if command == "judge-setup" => {
            craig_review::presets::fetch(
                preset,
                std::path::Path::new(directory),
                std::path::Path::new(binary),
            )
        }
        [command, corpus, left, right, output] if command == "policy-compare" => {
            craig_review::comparison::evaluate(
                std::path::Path::new(corpus),
                std::path::Path::new(left),
                std::path::Path::new(right),
                std::path::Path::new(output),
            )
        }
        _ => Err("invalid_command".into()),
    };
    if let Err(code) = result {
        eprintln!("craig-review: {code}");
        std::process::exit(1);
    }
}
