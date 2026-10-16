use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_ARTIFACT: u64 = 536_870_912; // 512 MiB

fn digest(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| "model_unavailable")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65536]; // 64 KiB
    loop {
        let n = file.read(&mut buffer).map_err(|_| "model_unavailable")?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn regular(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => Err("unsafe_model_path".into()),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("model_unavailable".into()),
    }
}

pub fn prepare() -> Result<PathBuf, String> {
    // An explicit override is validated by the detector; setup must not overwrite it.
    if let Some(path) = std::env::var_os("CRAIG_MODEL_DIR").filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("invalid_model_path".into());
        }
        craig::RuntimeDetector::load(
            &path,
            &std::env::var("CRAIG_DETECTOR").unwrap_or_else(|_| "bert".into()),
            craig::Policy::OffensiveSpans,
            Default::default(),
        )
        .map_err(|_| "model_unavailable")?;
        return Ok(path);
    }
    let root = if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("cache_unavailable")?)
    } else if let Some(root) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(root)
    } else {
        PathBuf::from(std::env::var_os("HOME").ok_or("cache_unavailable")?).join(".cache")
    };
    if !root.is_absolute() {
        return Err("invalid_cache_path".into());
    }
    let dir = root.join("craig/models").join(craig::MODEL_REVISION);
    fs::create_dir_all(&dir).map_err(|_| "cache_unavailable")?;
    if fs::symlink_metadata(&dir)
        .map_err(|_| "cache_unavailable")?
        .file_type()
        .is_symlink()
    {
        return Err("unsafe_model_path".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| "cache_unavailable")?;
    }
    let lock_path = dir.join("download.lock");
    regular(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|_| "cache_unavailable")?;
    let started = Instant::now();
    while fs2::FileExt::try_lock_exclusive(&lock).is_err() {
        if started.elapsed() > Duration::from_secs(540) {
            return Err("setup_busy".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(480))
        .build()
        .map_err(|_| "download_failed")?;
    for artifact in &craig::model_catalog::delivery_detector().artifacts {
        let path = dir.join(&artifact.file);
        regular(&path)?;
        if digest(&path).ok().as_deref() == Some(&artifact.sha256) {
            continue;
        }
        let partial = path.with_extension("partial");
        regular(&partial)?;
        // A previous process may have finished downloading before it was stopped.
        if promote_verified(&partial, &path, &artifact.sha256)? {
            continue;
        }
        let url = format!(
            "https://github.com/yourmoodai/craig/releases/download/v{}/model-{}",
            env!("CARGO_PKG_VERSION"),
            artifact.file
        );
        download(&client, &url, &partial, &artifact.sha256)?;
        if path.exists() {
            fs::remove_file(&path).map_err(|_| "cache_unavailable")?;
        }
        fs::rename(partial, path).map_err(|_| "cache_unavailable")?;
    }
    Ok(dir)
}

fn download(
    client: &reqwest::blocking::Client,
    url: &str,
    partial: &Path,
    expected: &str,
) -> Result<(), String> {
    let offset = fs::metadata(partial).map(|m| m.len()).unwrap_or(0);
    let mut request = client.get(url);
    if offset > 0 {
        request = request.header("Range", format!("bytes={offset}-"));
    }
    let response = request.send().map_err(|_| "download_failed")?;
    if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        fs::remove_file(partial).map_err(|_| "cache_unavailable")?;
        return Err("download_failed_retry".into());
    }
    let append = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !response.status().is_success()
        || (append
            && !response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with(&format!("bytes {offset}-"))))
    {
        return Err("download_failed".into());
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(partial)
        .map_err(|_| "cache_unavailable")?;
    let bytes = std::io::copy(&mut response.take(MAX_ARTIFACT + 1), &mut file)
        .map_err(|_| "download_failed")?;
    file.flush().map_err(|_| "cache_unavailable")?;
    file.sync_all().map_err(|_| "cache_unavailable")?;
    drop(file);
    if bytes + if append { offset } else { 0 } > MAX_ARTIFACT || digest(partial)? != expected {
        let _ = fs::remove_file(partial);
        return Err("model_checksum_mismatch".into());
    }
    Ok(())
}

fn promote_verified(partial: &Path, path: &Path, expected: &str) -> Result<bool, String> {
    if digest(partial).ok().as_deref() != Some(expected) {
        return Ok(false);
    }
    if path.exists() {
        fs::remove_file(path).map_err(|_| "cache_unavailable")?;
    }
    fs::rename(partial, path).map_err(|_| "cache_unavailable")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn serve_once(response: &'static [u8]) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            socket.write_all(response).unwrap();
            String::from_utf8(request).unwrap().to_lowercase()
        });
        (url, thread)
    }

    #[test]
    fn downloads_resume_or_restart_and_reject_corruption() {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("model.partial");
        let expected = format!("{:x}", Sha256::digest(b"abcdef"));
        for response in [
            b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 3-5/6\r\nContent-Length: 3\r\n\r\ndef".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabcdef".as_slice(),
        ] {
            fs::write(&partial, b"abc").unwrap();
            let (url, server) = serve_once(response);
            download(&client, &url, &partial, &expected).unwrap();
            assert!(server.join().unwrap().contains("range: bytes=3-"));
            assert_eq!(fs::read(&partial).unwrap(), b"abcdef");
        }
        for response in [
            b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-2/6\r\nContent-Length: 3\r\n\r\nabc".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nwrong!".as_slice(),
            b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\r\n".as_slice(),
        ] {
            fs::write(&partial, b"abc").unwrap();
            let (url, server) = serve_once(response);
            assert!(download(&client, &url, &partial, &expected).is_err());
            server.join().unwrap();
        }
        assert!(!partial.exists());
    }

    #[test]
    fn truncated_download_retains_bytes_for_retry() {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("model.partial");
        let (url, server) = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabc");
        let expected = format!("{:x}", Sha256::digest(b"abcdef"));
        assert!(download(&client, &url, &partial, &expected).is_err());
        server.join().unwrap();
        assert_eq!(fs::read(partial).unwrap(), b"abc");
    }
    #[test]
    fn finished_download_survives_restart_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("model.partial");
        let destination = dir.path().join("model.bin");
        fs::write(&partial, b"verified model").unwrap();
        fs::write(&destination, b"corrupt cache").unwrap();
        assert!(!promote_verified(&partial, &destination, "wrong").unwrap());
        assert_eq!(fs::read(&destination).unwrap(), b"corrupt cache");
        let expected = digest(&partial).unwrap();
        assert!(promote_verified(&partial, &destination, &expected).unwrap());
        assert_eq!(fs::read(&destination).unwrap(), b"verified model");
        assert!(!partial.exists());
    }
    #[test]
    fn digest_and_path_checks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            digest(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(regular(dir.path()).is_err());
        assert!(regular(&path).is_ok());
        #[cfg(unix)]
        {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(regular(&link).is_err());
        }
    }
}
