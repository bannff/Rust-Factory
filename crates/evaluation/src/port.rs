use crate::{
    CreateOrMatch, EvaluationDefinitionV1, EvaluationError, EvaluationResultV1,
    EvaluatorAssessmentV1, EvaluatorDescriptorV1, ExecutorGuaranteesV1, LogicalEvaluationKey,
    StoreGuaranteesV1, TerminalEvidenceSnapshotV1,
};
use std::{future::Future, pin::Pin, sync::Arc};

pub type EvaluationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<EvaluatorAssessmentV1, EvaluationError>> + Send + 'a>>;
pub trait WorkflowEvidenceReader: Send + Sync {
    fn get_terminal(
        &self,
        tenant_id: &str,
        run_id: &str,
    ) -> Result<Option<TerminalEvidenceSnapshotV1>, EvaluationError>;
}
pub trait EvaluationExecutor: Send + Sync {
    fn descriptor(&self) -> EvaluatorDescriptorV1;
    fn guarantees(&self) -> ExecutorGuaranteesV1;
    fn assess<'a>(
        &'a self,
        definition: &'a EvaluationDefinitionV1,
        evidence: &'a TerminalEvidenceSnapshotV1,
    ) -> EvaluationFuture<'a>;
}
pub trait EvaluationStore: Send + Sync {
    fn create_or_match(&self, result: EvaluationResultV1)
    -> Result<CreateOrMatch, EvaluationError>;
    fn get(
        &self,
        tenant_id: &str,
        key: &LogicalEvaluationKey,
    ) -> Result<Option<EvaluationResultV1>, EvaluationError>;
    fn list(&self, tenant_id: &str) -> Result<Vec<EvaluationResultV1>, EvaluationError>;
    fn guarantees(&self) -> StoreGuaranteesV1;
}
impl<T: WorkflowEvidenceReader + ?Sized> WorkflowEvidenceReader for Arc<T> {
    fn get_terminal(
        &self,
        tenant_id: &str,
        run_id: &str,
    ) -> Result<Option<TerminalEvidenceSnapshotV1>, EvaluationError> {
        (**self).get_terminal(tenant_id, run_id)
    }
}
impl<T: EvaluationExecutor + ?Sized> EvaluationExecutor for Arc<T> {
    fn descriptor(&self) -> EvaluatorDescriptorV1 {
        (**self).descriptor()
    }
    fn guarantees(&self) -> ExecutorGuaranteesV1 {
        (**self).guarantees()
    }
    fn assess<'a>(
        &'a self,
        definition: &'a EvaluationDefinitionV1,
        evidence: &'a TerminalEvidenceSnapshotV1,
    ) -> EvaluationFuture<'a> {
        (**self).assess(definition, evidence)
    }
}
impl<T: EvaluationStore + ?Sized> EvaluationStore for Arc<T> {
    fn create_or_match(
        &self,
        result: EvaluationResultV1,
    ) -> Result<CreateOrMatch, EvaluationError> {
        (**self).create_or_match(result)
    }
    fn get(
        &self,
        tenant_id: &str,
        key: &LogicalEvaluationKey,
    ) -> Result<Option<EvaluationResultV1>, EvaluationError> {
        (**self).get(tenant_id, key)
    }
    fn list(&self, tenant_id: &str) -> Result<Vec<EvaluationResultV1>, EvaluationError> {
        (**self).list(tenant_id)
    }
    fn guarantees(&self) -> StoreGuaranteesV1 {
        (**self).guarantees()
    }
}
