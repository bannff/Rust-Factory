//! Framework-free public Evaluation contract and V1 compatibility vectors.

use std::future::Future;
use std::pin::Pin;
#[cfg(feature = "local")]
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

#[cfg(feature = "local")]
use evaluation::local::DeterministicCriteriaEvaluator;
use evaluation::{
    CreateOrMatch, CriterionV1, EvaluationDefinitionV1, EvaluationError, EvaluationResultV1,
    EvaluationService, EvaluationStore, EvidenceEventV1, ExecutorGuaranteesV1,
    LogicalEvaluationKey, StoreGuaranteesV1, TerminalEvidenceSnapshotV1, TerminalReason,
    TerminalStatus, Verdict, WorkflowEvidenceReader, definition_canonical_bytes, definition_digest,
    evaluate, result_canonical_bytes, snapshot_canonical_bytes, snapshot_digest,
};

fn poll_immediate<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = Box::pin(future);
    match Pin::new(&mut future).poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("local evaluation future unexpectedly requires a runtime"),
    }
}

#[derive(Clone)]
struct Reader(Option<TerminalEvidenceSnapshotV1>);
impl WorkflowEvidenceReader for Reader {
    fn get_terminal(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Option<TerminalEvidenceSnapshotV1>, EvaluationError> {
        Ok(self.0.clone())
    }
}
#[derive(Clone, Copy)]
struct NullStore;
impl EvaluationStore for NullStore {
    fn create_or_match(
        &self,
        result: EvaluationResultV1,
    ) -> Result<CreateOrMatch, EvaluationError> {
        Ok(CreateOrMatch::Created(result))
    }
    fn get(
        &self,
        _: &str,
        _: &LogicalEvaluationKey,
    ) -> Result<Option<EvaluationResultV1>, EvaluationError> {
        Ok(None)
    }
    fn list(&self, _: &str) -> Result<Vec<EvaluationResultV1>, EvaluationError> {
        Ok(vec![])
    }
    fn guarantees(&self) -> StoreGuaranteesV1 {
        StoreGuaranteesV1 {
            durable_across_restart: false,
            visible_across_processes: false,
            crash_atomic: false,
            evicts_on_capacity: false,
            max_results_per_tenant: 1,
            max_results_global: 1,
        }
    }
}

fn executor_guarantees() -> ExecutorGuaranteesV1 {
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

fn snapshot() -> TerminalEvidenceSnapshotV1 {
    TerminalEvidenceSnapshotV1 {
        tenant_id: "tenant".into(),
        run_id: "run".into(),
        workflow_id: "workflow".into(),
        workflow_version: "v1".into(),
        run_revision: 7,
        terminal_status: TerminalStatus::Succeeded,
        terminal_reason: TerminalReason::Completed,
        attempt_id: "attempt".into(),
        agent_id: "agent".into(),
        capability_scope_digest: "a".repeat(64),
        output: "hello".into(),
        events: vec![
            EvidenceEventV1 {
                sequence: 1,
                kind: "started".into(),
                data: String::new(),
            },
            EvidenceEventV1 {
                sequence: 2,
                kind: "result".into(),
                data: "hello".into(),
            },
        ],
    }
}
fn definition() -> EvaluationDefinitionV1 {
    EvaluationDefinitionV1 {
        evaluator_id: "exact".into(),
        evaluator_version: "1".into(),
        criteria: vec![
            CriterionV1::ExactOutput {
                expected: "hello".into(),
            },
            CriterionV1::EventKindCount {
                kind: "result".into(),
                expected: 1,
            },
            CriterionV1::EventDataEquals {
                sequence: 2,
                expected: "hello".into(),
            },
        ],
    }
}

#[test]
fn exact_v1_golden_bytes_and_hashes_survive_sync_and_async_paths() {
    let snapshot = snapshot();
    let definition = definition();
    let expected_snapshot = "2:v1\n6:tenant\n3:run\n8:workflow\n2:v1\n1:7\n9:succeeded\n9:completed\n7:attempt\n5:agent\n64:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n5:hello\n1:2\n1:1\n7:started\n0:\n1:2\n6:result\n5:hello\n";
    let expected_definition = "2:v1\n5:exact\n1:1\n1:3\n12:exact_output\n5:hello\n16:event_kind_count\n6:result\n1:1\n17:event_data_equals\n1:2\n5:hello\n";
    let expected_result = "2:v1\n6:tenant\n5:exact\n1:1\n64:5c94014a3ba627135274d1cf4c9b54e2c06af1a24e396d8d6dc3c5f6ab90d401\n3:run\n1:7\n64:400d023425c9ee77e3eb9ac40032e0871dcc3eaf6980b743f29fccdc025150eb\n4:pass\n1:0\n";

    assert_eq!(
        snapshot_canonical_bytes(&snapshot).expect("snapshot bytes"),
        expected_snapshot.as_bytes()
    );
    assert_eq!(
        snapshot_digest(&snapshot).expect("snapshot digest"),
        "400d023425c9ee77e3eb9ac40032e0871dcc3eaf6980b743f29fccdc025150eb"
    );
    assert_eq!(
        definition_canonical_bytes(&definition).expect("definition bytes"),
        expected_definition.as_bytes()
    );
    assert_eq!(
        definition_digest(&definition).expect("definition digest"),
        "5c94014a3ba627135274d1cf4c9b54e2c06af1a24e396d8d6dc3c5f6ab90d401"
    );

    let sync = evaluate(
        &Reader(Some(snapshot.clone())),
        "tenant",
        "run",
        &definition,
    )
    .expect("sync");
    #[cfg(feature = "local")]
    {
        let service = EvaluationService::new(
            Reader(Some(snapshot)),
            NullStore,
            DeterministicCriteriaEvaluator,
        );
        let asynchronous =
            poll_immediate(service.evaluate("tenant", "run", &definition)).expect("async");
        assert_eq!(sync, asynchronous);
    }
    assert_eq!(
        result_canonical_bytes(&sync).expect("result bytes"),
        expected_result.as_bytes()
    );
    assert_eq!(
        sync.content_hash,
        "03414bc05e2c0b4aae494cc0fe12473da48fa0922f637e3836662839a5bebe72"
    );
}

#[cfg(feature = "local")]
#[test]
fn all_three_ports_are_object_safe_behind_arc_dyn_and_local_future_is_immediate() {
    let reader: Arc<dyn WorkflowEvidenceReader> = Arc::new(Reader(Some(snapshot())));
    let store: Arc<dyn EvaluationStore> = Arc::new(NullStore);
    let executor: Arc<dyn evaluation::EvaluationExecutor> =
        Arc::new(DeterministicCriteriaEvaluator);
    let service = EvaluationService::new(reader, store, executor);
    let result = poll_immediate(service.evaluate("tenant", "run", &definition())).expect("result");
    assert_eq!(result.verdict, Verdict::Pass);
    assert_eq!(
        service.executor_guarantees(),
        ExecutorGuaranteesV1 {
            deterministic: true,
            ordered_findings: true,
            runtime_required: false,
            external_io: false,
            network_access: false,
            model_judging: false,
            framework_backed: false,
        }
    );
}

#[test]
fn executor_error_and_invalid_assessment_are_projected_to_adapter_failure() {
    #[derive(Clone, Copy)]
    struct BadExecutor(bool);
    impl evaluation::EvaluationExecutor for BadExecutor {
        fn descriptor(&self) -> evaluation::EvaluatorDescriptorV1 {
            evaluation::EvaluatorDescriptorV1 {
                backend: "bad",
                version: "1",
            }
        }
        fn guarantees(&self) -> ExecutorGuaranteesV1 {
            executor_guarantees()
        }
        fn assess<'a>(
            &'a self,
            _: &'a EvaluationDefinitionV1,
            _: &'a TerminalEvidenceSnapshotV1,
        ) -> evaluation::EvaluationFuture<'a> {
            Box::pin(async move {
                if self.0 {
                    Err(EvaluationError::InvalidRequest)
                } else {
                    Ok(evaluation::EvaluatorAssessmentV1 {
                        verdict: Verdict::Pass,
                        findings: vec!["forged".into()],
                    })
                }
            })
        }
    }
    for executor in [BadExecutor(true), BadExecutor(false)] {
        let service = EvaluationService::new(Reader(Some(snapshot())), NullStore, executor);
        assert_eq!(
            poll_immediate(service.evaluate("tenant", "run", &definition())),
            Err(EvaluationError::AdapterFailure)
        );
    }
}

#[test]
fn executor_cannot_forge_key_evidence_digest_or_content_hash() {
    #[derive(Clone, Copy)]
    struct AssessmentOnly;
    impl evaluation::EvaluationExecutor for AssessmentOnly {
        fn descriptor(&self) -> evaluation::EvaluatorDescriptorV1 {
            evaluation::EvaluatorDescriptorV1 {
                backend: "assessment_only",
                version: "1",
            }
        }
        fn guarantees(&self) -> ExecutorGuaranteesV1 {
            executor_guarantees()
        }
        fn assess<'a>(
            &'a self,
            _: &'a EvaluationDefinitionV1,
            _: &'a TerminalEvidenceSnapshotV1,
        ) -> evaluation::EvaluationFuture<'a> {
            Box::pin(async {
                Ok(evaluation::EvaluatorAssessmentV1 {
                    verdict: Verdict::Fail,
                    findings: vec!["criterion_1_failed".into()],
                })
            })
        }
    }
    let snapshot = snapshot();
    let definition = definition();
    let service = EvaluationService::new(Reader(Some(snapshot.clone())), NullStore, AssessmentOnly);
    let result = poll_immediate(service.evaluate("tenant", "run", &definition)).expect("result");
    assert_eq!(result.logical_key.tenant_id, "tenant");
    assert_eq!(result.logical_key.workflow_run_id, "run");
    assert_eq!(
        result.logical_key.criterion_digest,
        definition_digest(&definition).expect("digest")
    );
    assert_eq!(
        result.evidence_digest,
        snapshot_digest(&snapshot).expect("digest")
    );
    assert_eq!(
        result.content_hash,
        evaluation::result_digest(&result).expect("hash")
    );
}

#[test]
fn assessment_finding_count_bytes_and_verdict_shape_are_exact() {
    let at_limit = evaluation::EvaluatorAssessmentV1 {
        verdict: Verdict::Fail,
        findings: vec!["é".repeat(evaluation::MAX_FINDING_BYTES / 2); evaluation::MAX_FINDINGS],
    };
    assert_eq!(at_limit.findings[0].len(), evaluation::MAX_FINDING_BYTES);
    assert!(evaluation::validate_assessment(&at_limit).is_ok());

    let mut too_many = at_limit.clone();
    too_many.findings.push("extra".into());
    assert_eq!(
        evaluation::validate_assessment(&too_many),
        Err(EvaluationError::AdapterFailure)
    );
    let oversized = evaluation::EvaluatorAssessmentV1 {
        verdict: Verdict::Fail,
        findings: vec![format!(
            "{}é",
            "x".repeat(evaluation::MAX_FINDING_BYTES - 1)
        )],
    };
    assert_eq!(
        evaluation::validate_assessment(&oversized),
        Err(EvaluationError::AdapterFailure)
    );
    assert_eq!(
        evaluation::validate_assessment(&evaluation::EvaluatorAssessmentV1 {
            verdict: Verdict::Pass,
            findings: vec!["unexpected".into()]
        }),
        Err(EvaluationError::AdapterFailure)
    );
    assert_eq!(
        evaluation::validate_assessment(&evaluation::EvaluatorAssessmentV1 {
            verdict: Verdict::Fail,
            findings: vec![]
        }),
        Err(EvaluationError::AdapterFailure)
    );
}
