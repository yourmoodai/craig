//! Shared detector selection for submission and personal assessment.
use crate::{
    Detector, Error, ExecutionOptions, ModelDetector, Policy, Result, Span,
    decision::DecisionDetector,
};
use std::path::Path;

pub enum RuntimeDetector {
    Bert(Box<ModelDetector>),
    Lexical(DecisionDetector),
}
impl RuntimeDetector {
    pub fn load(
        dir: impl AsRef<Path>,
        kind: &str,
        policy: Policy,
        execution: ExecutionOptions,
    ) -> Result<Self> {
        match kind {
            "bert" => Ok(Self::Bert(Box::new(
                ModelDetector::load_policy_with_execution(dir, policy, execution)?,
            ))),
            "lexical" => {
                if policy != Policy::OffensiveSpans {
                    return Err(Error::PolicyUnavailable);
                }
                if execution.backend != crate::Backend::Cpu
                    || execution.fp16
                    || execution.window_batch != 1
                    || execution.coreml_dir.is_some()
                    || execution.compute_units != crate::ComputeUnits::All
                {
                    return Err(Error::InvalidOptions);
                }
                Ok(Self::Lexical(DecisionDetector::load(dir, "lexical")?))
            }
            _ => Err(Error::InvalidOptions),
        }
    }
    pub fn threshold(&self) -> f32 {
        match self {
            Self::Bert(m) => m.threshold(),
            Self::Lexical(m) => m.threshold(),
        }
    }
}
impl Detector for RuntimeDetector {
    fn detect(&mut self, text: &str) -> Result<Vec<Span>> {
        match self {
            Self::Bert(m) => m.detect(text),
            Self::Lexical(m) => m.detect(text),
        }
    }
}
