//! Deterministic process-local criteria evaluator.
//!
//! Enabled by the `local` feature. It performs no external I/O and requires no
//! async runtime; callers own polling, cancellation, and process lifecycle.

use crate::service::deterministic_assessment;
use crate::{
    EvaluationDefinitionV1, EvaluationExecutor, EvaluationFuture, EvaluatorDescriptorV1,
    ExecutorGuaranteesV1, TerminalEvidenceSnapshotV1,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct DeterministicCriteriaEvaluator;

impl EvaluationExecutor for DeterministicCriteriaEvaluator {
    fn descriptor(&self) -> EvaluatorDescriptorV1 {
        EvaluatorDescriptorV1 {
            backend: "local_deterministic",
            version: "v1",
        }
    }

    fn guarantees(&self) -> ExecutorGuaranteesV1 {
        ExecutorGuaranteesV1 {
            deterministic: true,
            ordered_findings: true,
            runtime_required: false,
            external_io: false,
            network_access: false,
            model_judging: false,
            framework_backed: false,
        }
    }

    fn assess<'a>(
        &'a self,
        definition: &'a EvaluationDefinitionV1,
        evidence: &'a TerminalEvidenceSnapshotV1,
    ) -> EvaluationFuture<'a> {
        Box::pin(async move { Ok(deterministic_assessment(definition, evidence)) })
    }
}
