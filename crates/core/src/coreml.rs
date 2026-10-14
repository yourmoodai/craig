use crate::{Error, Result};
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum ComputeUnits {
    #[default]
    All = 0,
    CpuOnly = 1,
    CpuGpu = 2,
    CpuNeuralEngine = 3,
}
impl std::str::FromStr for ComputeUnits {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "all" => Ok(Self::All),
            "cpu_only" => Ok(Self::CpuOnly),
            "cpu_gpu" => Ok(Self::CpuGpu),
            "cpu_neural_engine" => Ok(Self::CpuNeuralEngine),
            _ => Err(Error::InvalidOptions),
        }
    }
}

pub struct CoreMl {
    #[cfg(all(feature = "coreml", target_os = "macos"))]
    handle: std::ptr::NonNull<std::ffi::c_void>,
    #[cfg(all(feature = "coreml", target_os = "macos"))]
    shapes: Vec<[usize; 3]>,
}

#[cfg(all(feature = "coreml", target_os = "macos"))]
mod native {
    use super::*;
    use std::ffi::{CString, c_char, c_void};

    unsafe extern "C" {
        fn craig_coreml_load(path: *const c_char, units: i32) -> *mut c_void;
        fn craig_coreml_free(handle: *mut c_void);
        fn craig_coreml_predict(
            handle: *mut c_void,
            features: *const i32,
            batch: usize,
            width: usize,
            logits: *mut f32,
        ) -> i32;
    }
    // The model is owned by one detector; Node serializes access through its mutex.
    unsafe impl Send for CoreMl {}

    impl Drop for CoreMl {
        fn drop(&mut self) {
            // Safety: this is the sole owner of the retained bridge handle returned by load.
            unsafe { craig_coreml_free(self.handle.as_ptr()) }
        }
    }
    #[derive(serde::Deserialize)]
    struct Export {
        version: u32,
        precision: String,
        sources: std::collections::BTreeMap<String, String>,
        files: std::collections::BTreeMap<String, String>,
        shapes: Vec<[usize; 3]>,
    }
    fn verify(dir: &Path, source: &Path, fp16: bool) -> Result<Vec<[usize; 3]>> {
        let manifest: Export = serde_json::from_slice(
            &std::fs::read(dir.join("export.json")).map_err(|_| Error::ModelUnavailable)?,
        )
        .map_err(|_| Error::ModelIncompatible)?;
        let shapes: Vec<_> = [1, 2, 4, 8]
            .into_iter()
            .flat_map(|b| {
                [16, 32, 64, 128, 256, 512]
                    .into_iter()
                    .map(move |w| [3, b, w])
            })
            .collect();
        if manifest.version != 1
            || manifest.precision != if fp16 { "float16" } else { "float32" }
            || manifest.shapes.is_empty()
            || manifest.shapes.iter().any(|shape| !shapes.contains(shape))
            || manifest.files.is_empty()
        {
            return Err(Error::ModelIncompatible);
        }
        for name in [
            "pytorch_model.bin",
            "config.json",
            "vocab.txt",
            "head.safetensors",
            "manifest.json",
        ] {
            let path = source.join(name);
            let exists = path.try_exists().map_err(|_| Error::ModelUnavailable)?;
            if exists != manifest.sources.contains_key(name)
                || (exists && manifest.sources[name] != crate::artifact_digest(&path)?)
            {
                return Err(Error::ModelIncompatible);
            }
        }
        for (name, hash) in &manifest.files {
            let path = Path::new(name);
            if !path.starts_with("classifier.mlpackage")
                || path
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
                || crate::artifact_digest(&dir.join(path))? != *hash
            {
                return Err(Error::ModelIncompatible);
            }
        }
        Ok(manifest.shapes)
    }
    impl CoreMl {
        pub fn load(dir: &Path, source: &Path, fp16: bool, units: ComputeUnits) -> Result<Self> {
            let shapes = verify(dir, source, fp16)?;
            // Flexible shapes crashed/stalled in the measured CPU/Neural Engine path.
            // Reject that configuration before entering the platform runtime.
            if shapes.len() > 1 && units != ComputeUnits::CpuGpu {
                return Err(Error::InvalidOptions);
            }
            let path = CString::new(
                dir.join("classifier.mlpackage")
                    .as_os_str()
                    .as_encoded_bytes(),
            )
            .map_err(|_| Error::InvalidOptions)?;
            // Safety: load consumes a valid C string synchronously and retains its own model.
            let handle =
                std::ptr::NonNull::new(unsafe { craig_coreml_load(path.as_ptr(), units as i32) })
                    .ok_or(Error::ModelUnavailable)?;
            Ok(Self { handle, shapes })
        }
        pub fn predict(
            &self,
            windows: &[&[u32]],
            cls: u32,
            sep: u32,
        ) -> Result<Vec<Vec<Vec<f32>>>> {
            let needed_width = windows.iter().map(|w| w.len()).max().unwrap_or(0) + 2;
            let &[_, batch, width] = self
                .shapes
                .iter()
                .filter(|s| s[1] >= windows.len() && s[2] >= needed_width)
                .min_by_key(|s| s[1] * s[2])
                .ok_or(Error::InvalidOptions)?;
            if windows.is_empty() || batch > 8 || width > 512 {
                return Err(Error::InvalidOptions);
            }
            let count = batch * width;
            let mut features = vec![0i32; 3 * count];
            for (row, tokens) in windows.iter().enumerate() {
                let start = row * width;
                features[start] = cls as i32;
                for (i, &token) in tokens.iter().enumerate() {
                    features[start + i + 1] = token as i32;
                }
                features[start + tokens.len() + 1] = sep as i32;
                features[count + start..count + start + tokens.len() + 2].fill(1);
            }
            let mut logits = vec![0f32; count * 2];
            // Safety: dimensions bound both buffers; the bridge copies them synchronously.
            if unsafe {
                craig_coreml_predict(
                    self.handle.as_ptr(),
                    features.as_ptr(),
                    batch,
                    width,
                    logits.as_mut_ptr(),
                )
            } != 1
                || logits.iter().any(|x| !x.is_finite())
            {
                return Err(Error::InferenceFailed);
            }
            Ok(logits
                .chunks_exact(width * 2)
                .take(windows.len())
                .map(|row| {
                    row.as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| {
                            let max = pair[0].max(pair[1]);
                            let a = (pair[0] - max).exp();
                            let b = (pair[1] - max).exp();
                            vec![a / (a + b), b / (a + b)]
                        })
                        .collect()
                })
                .collect())
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn export_is_bound_to_source_precision_and_package_bytes() {
            let dir = tempfile::tempdir().unwrap();
            let source = tempfile::tempdir().unwrap();
            let mut sources = std::collections::BTreeMap::new();
            for name in ["pytorch_model.bin", "config.json", "vocab.txt"] {
                std::fs::write(source.path().join(name), b"synthetic").unwrap();
                sources.insert(
                    name,
                    crate::artifact_digest(&source.path().join(name)).unwrap(),
                );
            }
            let package = dir.path().join("classifier.mlpackage");
            std::fs::create_dir(&package).unwrap();
            std::fs::write(package.join("weights"), b"synthetic").unwrap();
            let shapes: Vec<_> = [1, 2, 4, 8]
                .into_iter()
                .flat_map(|b| {
                    [16, 32, 64, 128, 256, 512]
                        .into_iter()
                        .map(move |w| [3, b, w])
                })
                .collect();
            let manifest = serde_json::json!({"version":1,"precision":"float32","sources":sources,"shapes":shapes,
                "files":{"classifier.mlpackage/weights":crate::artifact_digest(&package.join("weights")).unwrap()}});
            std::fs::write(
                dir.path().join("export.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            assert_eq!(verify(dir.path(), source.path(), false), Ok(shapes));
            assert_eq!(
                CoreMl::load(dir.path(), source.path(), false, ComputeUnits::All).err(),
                Some(Error::InvalidOptions)
            );
            assert_eq!(
                verify(dir.path(), source.path(), true),
                Err(Error::ModelIncompatible)
            );
            std::fs::write(package.join("weights"), b"modified").unwrap();
            assert_eq!(
                verify(dir.path(), source.path(), false),
                Err(Error::ModelIncompatible)
            );
            std::fs::write(package.join("weights"), b"synthetic").unwrap();
            std::fs::write(source.path().join("vocab.txt"), b"modified").unwrap();
            assert_eq!(
                verify(dir.path(), source.path(), false),
                Err(Error::ModelIncompatible)
            );
        }
    }
}

#[cfg(not(all(feature = "coreml", target_os = "macos")))]
impl CoreMl {
    pub fn load(_: &Path, _: &Path, _: bool, _: ComputeUnits) -> Result<Self> {
        Err(Error::ModelUnavailable)
    }
    pub fn predict(&self, _: &[&[u32]], _: u32, _: u32) -> Result<Vec<Vec<Vec<f32>>>> {
        Err(Error::ModelUnavailable)
    }
}
