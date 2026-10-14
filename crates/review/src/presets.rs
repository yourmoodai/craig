use crate::{ReviewResult, judge::JudgeConfig};
use std::{io::Write, path::Path, time::Duration};

pub struct Preset {
    pub name: &'static str,
    pub model_id: &'static str,
    pub repository: &'static str,
    pub revision: &'static str,
    pub file: &'static str,
    pub sha256: &'static str,
    pub quantization: &'static str,
}
pub const PRESETS: [Preset; 2] = [
    Preset {
        name: "qwen",
        model_id: "Qwen/Qwen3-4B-Instruct-2507",
        repository: "bartowski/Qwen_Qwen3-4B-Instruct-2507-GGUF",
        revision: "ae44f08e1392f39c0e474af10c3ff8355c8b6688",
        file: "Qwen_Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        sha256: "2fde00ce69dd4899c70d020845e2638353015bba0fdf161b3eb965f2bca4464e",
        quantization: "Q4_K_M",
    },
    Preset {
        name: "gemma",
        model_id: "google/gemma-4-12B-it",
        repository: "ggml-org/gemma-4-12B-it-GGUF",
        revision: "e3e681731089efaa3f0917336944ac64752db8ba",
        file: "gemma-4-12B-it-Q4_0.gguf",
        sha256: "3712b9bd32cae83a22f67ee7a4466d8d7a4f21646ac8a07d19bf9418e8767a70",
        quantization: "Q4_0",
    },
];
pub fn validate(config: &JudgeConfig) -> ReviewResult<()> {
    if PRESETS.iter().any(|p| {
        config.model_id == p.model_id
            && config.revision == p.revision
            && config.sha256 == p.sha256
            && config.quantization == p.quantization
    }) {
        Ok(())
    } else {
        Err("judge_artifact_mismatch".into())
    }
}
pub fn fetch(name: &str, directory: &Path, binary: &Path) -> ReviewResult<()> {
    let preset = PRESETS
        .iter()
        .find(|p| p.name == name)
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
        || middle_finger::artifact_digest(&artifact).map_err(|_| "judge_artifact_unavailable")?
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
        file.flush().map_err(|_| "setup_failed")?;
        if middle_finger::artifact_digest(&temporary).map_err(|_| "judge_artifact_unavailable")?
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
