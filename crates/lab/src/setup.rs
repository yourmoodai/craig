use crate::LabResult;
use std::path::Path;

pub fn fetch_judge(directory: &Path, binary: &Path) -> LabResult<()> {
    craig_review::presets::fetch("qwen", directory, binary)
}
