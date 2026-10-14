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
        let offset = fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
        let url = format!(
            "https://github.com/yourmoodai/craig/releases/download/v{}/model-{}",
            env!("CARGO_PKG_VERSION"),
            artifact.file
        );
        let mut request = client.get(url);
        if offset > 0 {
            request = request.header("Range", format!("bytes={offset}-"));
        }
        let response = request.send().map_err(|_| "download_failed")?;
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
            .open(&partial)
            .map_err(|_| "cache_unavailable")?;
        let bytes = std::io::copy(&mut response.take(MAX_ARTIFACT + 1), &mut file)
            .map_err(|_| "download_failed")?;
        file.flush().map_err(|_| "cache_unavailable")?;
        file.sync_all().map_err(|_| "cache_unavailable")?;
        drop(file);
        if bytes + if append { offset } else { 0 } > MAX_ARTIFACT
            || digest(&partial)? != artifact.sha256
        {
            let _ = fs::remove_file(&partial);
            return Err("model_checksum_mismatch".into());
        }
        if path.exists() {
            fs::remove_file(&path).map_err(|_| "cache_unavailable")?;
        }
        fs::rename(partial, path).map_err(|_| "cache_unavailable")?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
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
