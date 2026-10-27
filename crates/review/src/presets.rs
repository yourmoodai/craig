use crate::{ReviewResult, judge::JudgeConfig};
use std::{io::Write, path::Path, time::Duration};

#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub name: &'static str,
    pub model_id: &'static str,
    pub repository: &'static str,
    pub revision: &'static str,
    pub file: &'static str,
    pub sha256: &'static str,
    pub quantization: &'static str,
}

pub fn presets() -> Vec<Preset> {
    savior::model_catalog::judges()
        .map(|model| {
            let artifact = model.artifacts.first().expect("judge artifact");
            let converter_url = model
                .provenance
                .converter_url
                .as_deref()
                .expect("judge converter");
            Preset {
                name: if model.id.starts_with("Qwen/") {
                    "qwen"
                } else {
                    "gemma"
                },
                model_id: &model.id,
                repository: converter_url
                    .strip_prefix("https://huggingface.co/")
                    .expect("hugging face converter"),
                revision: model
                    .provenance
                    .converter_revision
                    .as_deref()
                    .expect("converter revision"),
                file: &artifact.file,
                sha256: &artifact.sha256,
                quantization: artifact
                    .quantization
                    .as_deref()
                    .expect("judge quantization"),
            }
        })
        .collect()
}

pub fn validate(config: &JudgeConfig) -> ReviewResult<()> {
    if presets().iter().any(|preset| {
        config.model_id == preset.model_id
            && config.revision == preset.revision
            && config.sha256 == preset.sha256
            && config.quantization == preset.quantization
    }) {
        Ok(())
    } else {
        Err("judge_artifact_mismatch".into())
    }
}

pub fn fetch(name: &str, directory: &Path, binary: &Path) -> ReviewResult<()> {
    let preset = presets()
        .into_iter()
        .find(|preset| preset.name == name)
        .ok_or("unknown_judge_preset")?;
    let binary = binary
        .canonicalize()
        .map_err(|_| "judge_binary_unavailable")?;
    std::fs::create_dir_all(directory).map_err(|_| "setup_directory_unavailable")?;
    let directory = directory
        .canonicalize()
        .map_err(|_| "setup_directory_unavailable")?;
    let artifact = directory.join(preset.file);
    let config = JudgeConfig {
        model_id: preset.model_id.into(),
        revision: preset.revision.into(),
        quantization: preset.quantization.into(),
        artifact: artifact.clone(),
        sha256: preset.sha256.into(),
        server_binary: binary,
    };
    let config_path = directory.join("judge.json");
    let value = serde_json::to_value(&config).map_err(|_| "setup_failed")?;
    if config_path.exists() {
        let old: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&config_path).map_err(|_| "setup_failed")?)
                .map_err(|_| "setup_failed")?;
        if old != value {
            return Err("judge_config_exists_with_different_settings".into());
        }
    }
    if !artifact.exists()
        || savior::artifact_digest(&artifact).map_err(|_| "judge_artifact_unavailable")?
            != preset.sha256
    {
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            preset.repository, preset.revision, preset.file
        );
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .timeout(Duration::from_secs(1800))
            .build()
            .map_err(|_| "setup_failed")?;
        let mut response = client
            .get(url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|_| "judge_download_failed")?;
        let temporary = directory.join("model.partial");
        let mut file = std::fs::File::create(&temporary).map_err(|_| "setup_failed")?;
        response
            .copy_to(&mut file)
            .map_err(|_| "judge_download_failed")?;
        file.flush().map_err(|_| "judge_download_failed")?;
        if savior::artifact_digest(&temporary).map_err(|_| "judge_artifact_unavailable")?
            != preset.sha256
        {
            return Err("judge_artifact_mismatch".into());
        }
        std::fs::rename(temporary, artifact).map_err(|_| "setup_failed")?;
    }
    if !config_path.exists() {
        crate::policy::write_report(&config_path, &value)?;
    }
    Ok(())
}
