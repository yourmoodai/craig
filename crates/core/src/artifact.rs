use crate::{Error, MODEL_ID, MODEL_REVISION, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, str::FromStr};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    #[default]
    OffensiveSpans,
    DirectedAbuse,
}
impl Policy {
    pub fn name(self) -> &'static str {
        match self {
            Self::OffensiveSpans => "offensive_spans",
            Self::DirectedAbuse => "directed_abuse",
        }
    }
}
impl FromStr for Policy {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "offensive_spans" => Ok(Self::OffensiveSpans),
            "directed_abuse" => Ok(Self::DirectedAbuse),
            _ => Err(Error::InvalidOptions),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub version: u32,
    pub base_model: String,
    pub base_revision: String,
    pub policy: Policy,
    pub threshold: f32,
    pub labels: Vec<String>,
    pub head_sha256: String,
    pub provenance: serde_json::Value,
}
impl ModelManifest {
    pub fn read(dir: &Path) -> Result<Self> {
        let bytes =
            std::fs::read(dir.join("manifest.json")).map_err(|_| Error::ModelUnavailable)?;
        let manifest: Self =
            serde_json::from_slice(&bytes).map_err(|_| Error::ModelIncompatible)?;
        if manifest.version != 1
            || manifest.base_model != MODEL_ID
            || manifest.base_revision != MODEL_REVISION
            || manifest.labels != ["NOT_TOXIC", "TOXIC"]
            || !manifest.threshold.is_finite()
            || manifest.threshold <= 0.0
            || manifest.threshold > 1.0
            || manifest.head_sha256 != artifact_digest(&dir.join("head.safetensors"))?
        {
            return Err(Error::ModelIncompatible);
        }
        Ok(manifest)
    }
}

pub fn artifact_digest(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).map_err(|_| Error::ModelUnavailable)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65_536]; // 64 KiB
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| Error::ModelUnavailable)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_manifest_rejects_tampering_and_incompatible_labels() {
        let dir = tempfile::tempdir().unwrap();
        let head = dir.path().join("head.safetensors");
        std::fs::write(&head, b"synthetic artifact").unwrap();
        let mut manifest = ModelManifest {
            version: 1,
            base_model: MODEL_ID.into(),
            base_revision: MODEL_REVISION.into(),
            policy: Policy::DirectedAbuse,
            threshold: 0.7,
            labels: vec!["NOT_TOXIC".into(), "TOXIC".into()],
            head_sha256: artifact_digest(&head).unwrap(),
            provenance: serde_json::json!({}),
        };
        let path = dir.path().join("manifest.json");
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(
            ModelManifest::read(dir.path()).unwrap().policy,
            Policy::DirectedAbuse
        );
        std::fs::write(&head, b"tampered artifact").unwrap();
        assert_eq!(
            ModelManifest::read(dir.path()).unwrap_err(),
            Error::ModelIncompatible
        );
        manifest.head_sha256 = artifact_digest(&head).unwrap();
        manifest.labels.reverse();
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(
            ModelManifest::read(dir.path()).unwrap_err(),
            Error::ModelIncompatible
        );
    }
    #[test]
    fn policies_are_explicit() {
        assert_eq!(
            "directed_abuse".parse::<Policy>().unwrap(),
            Policy::DirectedAbuse
        );
        assert_eq!("unknown".parse::<Policy>(), Err(Error::InvalidOptions));
    }
}
