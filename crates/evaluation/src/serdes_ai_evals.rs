//! `serdes-ai-evals` executor adapter.

use crate::{
    CriterionV1, EvaluationDefinitionV1, EvaluationExecutor, EvaluationFuture,
    EvaluatorAssessmentV1, EvaluatorDescriptorV1, ExecutorGuaranteesV1, TerminalEvidenceSnapshotV1,
    Verdict,
};
use serdes_ai_evals::{EvaluationResult, Evaluator, ExactMatchScorer, FunctionScorer};

#[derive(Clone, Copy, Debug, Default)]
pub struct SerdesAiEvalsExecutor;
impl EvaluationExecutor for SerdesAiEvalsExecutor {
    fn descriptor(&self) -> EvaluatorDescriptorV1 {
        EvaluatorDescriptorV1 {
            backend: "serdes_ai_evals",
            version: "0.2.6",
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
            framework_backed: true,
        }
    }
    fn assess<'a>(
        &'a self,
        definition: &'a EvaluationDefinitionV1,
        evidence: &'a TerminalEvidenceSnapshotV1,
    ) -> EvaluationFuture<'a> {
        Box::pin(async move {
            let mut findings = Vec::new();
            let mut error = false;
            for (index, criterion) in definition.criteria.iter().enumerate() {
                let outcome = match criterion {
                    CriterionV1::ExactOutput { expected } => {
                        ExactMatchScorer::default()
                            .evaluate_str(&evidence.output, Some(expected))
                            .await
                    }
                    CriterionV1::EventKindCount { kind, expected } => {
                        let actual = evidence
                            .events
                            .iter()
                            .filter(|event| event.kind == *kind)
                            .count();
                        let scorer = FunctionScorer::new("event_kind_count", move |_, _| {
                            if actual == *expected as usize {
                                EvaluationResult::pass()
                            } else {
                                EvaluationResult::fail("mismatch")
                            }
                        });
                        scorer.evaluate_str("", None).await
                    }
                    CriterionV1::EventDataEquals { sequence, expected } => {
                        let matches = evidence
                            .events
                            .iter()
                            .find(|event| event.sequence == *sequence)
                            .is_some_and(|event| event.data == *expected);
                        let scorer = FunctionScorer::new("event_data_equals", move |_, _| {
                            if matches {
                                EvaluationResult::pass()
                            } else {
                                EvaluationResult::fail("mismatch")
                            }
                        });
                        scorer.evaluate_str("", None).await
                    }
                };
                let (criterion_error, finding) = map_outcome(index, outcome);
                error |= criterion_error;
                if let Some(finding) = finding {
                    findings.push(finding);
                }
            }
            Ok(EvaluatorAssessmentV1 {
                verdict: if error {
                    Verdict::Error
                } else if findings.is_empty() {
                    Verdict::Pass
                } else {
                    Verdict::Fail
                },
                findings,
            })
        })
    }
}

fn map_outcome(index: usize, outcome: EvaluationResult) -> (bool, Option<String>) {
    match outcome {
        EvaluationResult::Pass { .. } => (false, None),
        EvaluationResult::Fail { .. } => (false, Some(format!("criterion_{}_failed", index + 1))),
        EvaluationResult::Skip { .. } | EvaluationResult::Error { .. } => {
            (true, Some(format!("criterion_{}_error", index + 1)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_and_error_payloads_are_reduced_to_ordered_safe_findings() {
        let secret = "provider detail: /private/path token=secret";
        for outcome in [
            EvaluationResult::skip(secret),
            EvaluationResult::error(secret),
        ] {
            let (is_error, finding) = map_outcome(4, outcome);
            assert!(is_error);
            let finding = finding.expect("safe finding");
            assert_eq!(finding, "criterion_5_error");
            assert!(!finding.contains(secret));
            assert!(!finding.contains("provider"));
        }
    }

    #[test]
    fn mixed_framework_outcomes_preserve_criterion_order_and_aggregate_error() {
        let outcomes = [
            EvaluationResult::fail("secret reason"),
            EvaluationResult::skip("secret skip"),
            EvaluationResult::error("secret error"),
        ];
        let mut findings = Vec::new();
        let mut error = false;
        for (index, outcome) in outcomes.into_iter().enumerate() {
            let (criterion_error, finding) = map_outcome(index, outcome);
            error |= criterion_error;
            if let Some(finding) = finding {
                findings.push(finding);
            }
        }
        assert!(error, "Skip/Error must force the aggregate Error verdict");
        assert_eq!(
            findings,
            [
                "criterion_1_failed",
                "criterion_2_error",
                "criterion_3_error"
            ]
        );
        assert!(findings.iter().all(|finding| !finding.contains("secret")));
    }
}
