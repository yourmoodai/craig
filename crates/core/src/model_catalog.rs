use serde::Deserialize;
use std::{collections::BTreeSet, sync::OnceLock};

const CATALOG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../models/catalog.json"
));

#[derive(Debug, Deserialize)]
pub struct ModelCatalog {
    pub schema_version: u32,
    pub models: Vec<Model>,
}
#[derive(Debug, Deserialize)]
pub struct Model {
    pub id: String,
    pub display_name: String,
    pub publisher: String,
    pub task: String,
    pub license: String,
    pub distribution: String,
    pub roles: Vec<String>,
    pub scope: Scope,
    pub provenance: Provenance,
    pub artifacts: Vec<Artifact>,
}
#[derive(Debug, Deserialize)]
pub struct Scope {
    pub allowed: Vec<String>,
    pub prohibited: Vec<String>,
}
#[derive(Debug, Deserialize)]
pub struct Provenance {
    pub upstream_url: String,
    pub upstream_revision: String,
    pub license_url: String,
    pub converter_url: Option<String>,
    pub converter_revision: Option<String>,
}
#[derive(Debug, Deserialize)]
pub struct Artifact {
    pub file: String,
    pub sha256: String,
    pub quantization: Option<String>,
}

pub fn catalog() -> &'static ModelCatalog {
    static CATALOG_VALUE: OnceLock<ModelCatalog> = OnceLock::new();
    CATALOG_VALUE.get_or_init(|| {
        let catalog: ModelCatalog = serde_json::from_str(CATALOG).expect("valid model catalog");
        validate(&catalog).expect("compatible model catalog");
        catalog
    })
}
pub fn model(id: &str) -> Option<&'static Model> {
    catalog().models.iter().find(|entry| entry.id == id)
}
pub fn delivery_detector() -> &'static Model {
    catalog()
        .models
        .iter()
        .find(|entry| entry.roles.iter().any(|role| role == "delivery_detector"))
        .expect("one delivery detector")
}
pub fn judges() -> impl Iterator<Item = &'static Model> {
    catalog()
        .models
        .iter()
        .filter(|entry| entry.roles.iter().any(|role| role == "local_judge"))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
fn validate(catalog: &ModelCatalog) -> Result<(), &'static str> {
    if catalog.schema_version != 1 {
        return Err("invalid_schema");
    }
    let mut ids = BTreeSet::new();
    let mut detectors = 0;
    for model in &catalog.models {
        if !ids.insert(model.id.as_str())
            || model.license != "Apache-2.0"
            || model.distribution != "explicit_local_download"
            || model.display_name.is_empty()
            || model.publisher.is_empty()
            || model.task.is_empty()
            || model.scope.allowed.is_empty()
            || model.scope.prohibited.is_empty()
            || !model
                .scope
                .prohibited
                .iter()
                .any(|rule| rule == "provider_policy_enforcement")
            || !model
                .scope
                .prohibited
                .iter()
                .any(|rule| rule == "certification")
            || model.provenance.upstream_url.is_empty()
            || model.provenance.upstream_revision.is_empty()
            || model.provenance.license_url.is_empty()
            || model.artifacts.is_empty()
            || model.roles.is_empty()
            || model.roles.iter().any(|role| {
                !["delivery_detector", "local_judge", "cross_model_judge"].contains(&role.as_str())
            })
            || model
                .artifacts
                .iter()
                .any(|artifact| artifact.file.is_empty() || !valid_digest(&artifact.sha256))
        {
            return Err("invalid_model");
        }
        if model.roles.iter().any(|role| role == "delivery_detector") {
            detectors += 1;
        }
        if model.roles.iter().any(|role| role == "local_judge")
            && !model
                .scope
                .prohibited
                .iter()
                .any(|rule| rule == "delivery_detector")
        {
            return Err("judge_delivery_overlap");
        }
    }
    if detectors != 1
        || !ids.contains("mudes/en-base")
        || !ids.contains("Qwen/Qwen3-4B-Instruct-2507")
        || !ids.contains("google/gemma-4-12B-it")
    {
        return Err("missing_required_model");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_binds_one_detector_and_two_local_judges() {
        assert_eq!(delivery_detector().id, "mudes/en-base");
        assert_eq!(judges().count(), 2);
        assert!(judges().all(|judge| !judge.roles.iter().any(|role| role == "delivery_detector")));
    }
}
