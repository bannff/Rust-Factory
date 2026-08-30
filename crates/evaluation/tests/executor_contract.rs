#![cfg(any(feature = "local", feature = "serdes-ai-evals"))]
//! Generic executor conformance and cross-adapter V1 parity.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

#[cfg(feature = "local")]
use evaluation::local::DeterministicCriteriaEvaluator;
use evaluation::{
    CriterionV1, EvaluationDefinitionV1, EvaluationExecutor, EvidenceEventV1, MAX_CRITERIA,
    MAX_EXPECTED_BYTES, TerminalEvidenceSnapshotV1, TerminalReason, TerminalStatus, Verdict,
};

fn immediate<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = Box::pin(future);
    match Pin::new(&mut future).poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("executor future unexpectedly requires a runtime"),
    }
}
fn evidence(output: &str) -> TerminalEvidenceSnapshotV1 {
    TerminalEvidenceSnapshotV1 {
        tenant_id: "tenant".into(),
        run_id: "run".into(),
        workflow_id: "workflow".into(),
        workflow_version: "1".into(),
        run_revision: 1,
        terminal_status: TerminalStatus::Succeeded,
        terminal_reason: TerminalReason::Completed,
        attempt_id: "attempt".into(),
        agent_id: "agent".into(),
        capability_scope_digest: "a".repeat(64),
        output: output.into(),
        events: vec![
            EvidenceEventV1 {
                sequence: 1,
                kind: "alpha".into(),
                data: "one".into(),
            },
            EvidenceEventV1 {
                sequence: 2,
                kind: "alpha".into(),
                data: "two".into(),
            },
            EvidenceEventV1 {
                sequence: 3,
                kind: "omega".into(),
                data: "三".into(),
            },
        ],
    }
}
fn definition(criteria: Vec<CriterionV1>) -> EvaluationDefinitionV1 {
    EvaluationDefinitionV1 {
        evaluator_id: "evaluator".into(),
        evaluator_version: "1".into(),
        criteria,
    }
}
fn assess<E: EvaluationExecutor>(
    executor: &E,
    definition: &EvaluationDefinitionV1,
    evidence: &TerminalEvidenceSnapshotV1,
) -> evaluation::EvaluatorAssessmentV1 {
    immediate(executor.assess(definition, evidence)).expect("assessment")
}
fn run_conformance<E: EvaluationExecutor>(executor: &E) {
    let snapshot = evidence(" hello \n世界");
    let pass = definition(vec![
        CriterionV1::ExactOutput {
            expected: " hello \n世界".into(),
        },
        CriterionV1::EventKindCount {
            kind: "alpha".into(),
            expected: 2,
        },
        CriterionV1::EventDataEquals {
            sequence: 3,
            expected: "三".into(),
        },
    ]);
    assert_eq!(
        assess(executor, &pass, &snapshot),
        evaluation::EvaluatorAssessmentV1 {
            verdict: Verdict::Pass,
            findings: vec![]
        }
    );

    for (index, criterion) in [
        CriterionV1::ExactOutput {
            expected: "hello \n世界".into(),
        },
        CriterionV1::EventKindCount {
            kind: "alpha".into(),
            expected: 1,
        },
        CriterionV1::EventDataEquals {
            sequence: 3,
            expected: "三 ".into(),
        },
        CriterionV1::EventDataEquals {
            sequence: 4,
            expected: String::new(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            assess(executor, &definition(vec![criterion]), &snapshot),
            evaluation::EvaluatorAssessmentV1 {
                verdict: Verdict::Fail,
                findings: vec!["criterion_1_failed".into()]
            },
            "failure cohort {index}"
        );
    }

    let all_fail = definition(vec![
        CriterionV1::ExactOutput {
            expected: "wrong".into(),
        },
        CriterionV1::EventKindCount {
            kind: "alpha".into(),
            expected: 0,
        },
        CriterionV1::EventDataEquals {
            sequence: 99,
            expected: "missing".into(),
        },
    ]);
    assert_eq!(
        assess(executor, &all_fail, &snapshot).findings,
        [
            "criterion_1_failed",
            "criterion_2_failed",
            "criterion_3_failed"
        ]
    );
    assert_eq!(
        assess(executor, &definition(vec![]), &snapshot).verdict,
        Verdict::Pass
    );

    let duplicate = definition(vec![
        CriterionV1::ExactOutput {
            expected: "wrong".into(),
        },
        CriterionV1::ExactOutput {
            expected: "wrong".into(),
        },
    ]);
    assert_eq!(
        assess(executor, &duplicate, &snapshot).findings,
        ["criterion_1_failed", "criterion_2_failed"]
    );

    let max = "x".repeat(MAX_EXPECTED_BYTES);
    let max_evidence = evidence(&max);
    let max_definition = definition(vec![
        CriterionV1::ExactOutput { expected: max };
        MAX_CRITERIA
    ]);
    assert_eq!(
        assess(executor, &max_definition, &max_evidence).verdict,
        Verdict::Pass
    );
}

#[cfg(feature = "local")]
#[test]
fn deterministic_executor_conforms() {
    run_conformance(&DeterministicCriteriaEvaluator);
    assert_eq!(
        DeterministicCriteriaEvaluator.descriptor().backend,
        "local_deterministic"
    );
}

#[cfg(feature = "serdes-ai-evals")]
#[derive(Clone)]
struct Reader(TerminalEvidenceSnapshotV1);
#[cfg(feature = "serdes-ai-evals")]
impl evaluation::WorkflowEvidenceReader for Reader {
    fn get_terminal(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Option<TerminalEvidenceSnapshotV1>, evaluation::EvaluationError> {
        Ok(Some(self.0.clone()))
    }
}
#[cfg(feature = "serdes-ai-evals")]
#[derive(Clone, Copy)]
struct Store;
#[cfg(feature = "serdes-ai-evals")]
impl evaluation::EvaluationStore for Store {
    fn create_or_match(
        &self,
        result: evaluation::EvaluationResultV1,
    ) -> Result<evaluation::CreateOrMatch, evaluation::EvaluationError> {
        Ok(evaluation::CreateOrMatch::Created(result))
    }
    fn get(
        &self,
        _: &str,
        _: &evaluation::LogicalEvaluationKey,
    ) -> Result<Option<evaluation::EvaluationResultV1>, evaluation::EvaluationError> {
        Ok(None)
    }
    fn list(
        &self,
        _: &str,
    ) -> Result<Vec<evaluation::EvaluationResultV1>, evaluation::EvaluationError> {
        Ok(vec![])
    }
    fn guarantees(&self) -> evaluation::StoreGuaranteesV1 {
        evaluation::StoreGuaranteesV1 {
            durable_across_restart: false,
            visible_across_processes: false,
            crash_atomic: false,
            evicts_on_capacity: false,
            max_results_per_tenant: 1,
            max_results_global: 1,
        }
    }
}

#[cfg(feature = "serdes-ai-evals")]
fn assert_serdes_ai_evals_golden() {
    use evaluation::serdes_ai_evals::SerdesAiEvalsExecutor;
    let snapshot = TerminalEvidenceSnapshotV1 {
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
    };
    let definition = EvaluationDefinitionV1 {
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
    };
    let service =
        evaluation::EvaluationService::new(Reader(snapshot), Store, SerdesAiEvalsExecutor);
    let result = immediate(service.evaluate("tenant", "run", &definition))
        .expect("golden serdes_ai_evals result");
    assert_eq!(
        result.logical_key.criterion_digest,
        "5c94014a3ba627135274d1cf4c9b54e2c06af1a24e396d8d6dc3c5f6ab90d401"
    );
    assert_eq!(
        result.evidence_digest,
        "400d023425c9ee77e3eb9ac40032e0871dcc3eaf6980b743f29fccdc025150eb"
    );
    assert_eq!(
        result.content_hash,
        "03414bc05e2c0b4aae494cc0fe12473da48fa0922f637e3836662839a5bebe72"
    );
}

#[cfg(feature = "serdes-ai-evals")]
#[test]
fn serdes_ai_evals_executor_conforms() {
    use evaluation::serdes_ai_evals::SerdesAiEvalsExecutor;
    run_conformance(&SerdesAiEvalsExecutor);
    assert_serdes_ai_evals_golden();
}

#[cfg(all(feature = "local", feature = "serdes-ai-evals"))]
#[test]
fn serdes_ai_evals_executor_matches_local_for_full_results() {
    use evaluation::serdes_ai_evals::SerdesAiEvalsExecutor;
    run_conformance(&SerdesAiEvalsExecutor);
    let fixtures = [
        (
            evidence("same"),
            definition(vec![CriterionV1::ExactOutput {
                expected: "same".into(),
            }]),
        ),
        (
            evidence("different"),
            definition(vec![
                CriterionV1::ExactOutput {
                    expected: "same".into(),
                },
                CriterionV1::EventKindCount {
                    kind: "alpha".into(),
                    expected: 9,
                },
                CriterionV1::EventDataEquals {
                    sequence: 99,
                    expected: "none".into(),
                },
            ]),
        ),
        (
            evidence("é \n"),
            definition(vec![CriterionV1::ExactOutput {
                expected: "é\n".into(),
            }]),
        ),
    ];
    for (snapshot, definition) in fixtures {
        let local = assess(&DeterministicCriteriaEvaluator, &definition, &snapshot);
        let serdes_ai_evals = assess(&SerdesAiEvalsExecutor, &definition, &snapshot);
        assert_eq!(
            serdes_ai_evals, local,
            "adapter assessments must be byte-semantic peers"
        );

        let local_service = evaluation::EvaluationService::new(
            Reader(snapshot.clone()),
            Store,
            DeterministicCriteriaEvaluator,
        );
        let serdes_ai_evals_service =
            evaluation::EvaluationService::new(Reader(snapshot), Store, SerdesAiEvalsExecutor);
        let local_result =
            immediate(local_service.evaluate("tenant", "run", &definition)).expect("local");
        let serdes_ai_evals_result =
            immediate(serdes_ai_evals_service.evaluate("tenant", "run", &definition))
                .expect("serdes_ai_evals");
        assert_eq!(
            serdes_ai_evals_result, local_result,
            "verdict, findings, digests, and hashes must match"
        );
    }

    assert_serdes_ai_evals_golden();
}
