use crate::{
    Clock, MAX_QUERY_LIMIT, ObservabilityError, TelemetryContext, TelemetryEnvelopeV1,
    TelemetryEventV1, TelemetryGuarantees, TelemetryQueryV1, TelemetryReader, TelemetryRecordV1,
    TelemetrySink, record_matches, validate_envelope, validate_query, validate_record,
};

pub struct TelemetryService<S, R, C> {
    sink: S,
    reader: R,
    clock: C,
    max_query_limit: usize,
}
impl<S, R, C> TelemetryService<S, R, C>
where
    S: TelemetrySink,
    R: TelemetryReader,
    C: Clock,
{
    pub fn new(
        sink: S,
        reader: R,
        clock: C,
        max_query_limit: usize,
    ) -> Result<Self, ObservabilityError> {
        if max_query_limit == 0 || max_query_limit > MAX_QUERY_LIMIT {
            return Err(ObservabilityError::LimitExceeded);
        }
        Ok(Self {
            sink,
            reader,
            clock,
            max_query_limit,
        })
    }

    pub fn emit(
        &self,
        context: &TelemetryContext,
        event: TelemetryEventV1,
    ) -> Result<(), ObservabilityError> {
        let envelope = TelemetryEnvelopeV1 {
            tenant_id: context.tenant_id().clone(),
            timestamp: self.clock.now()?,
            event,
        };
        validate_envelope(&envelope)?;
        self.sink.emit(envelope)
    }

    pub fn validate_query(&self, query: &TelemetryQueryV1) -> Result<(), ObservabilityError> {
        validate_query(query, self.max_query_limit)
    }

    pub fn query(
        &self,
        context: &TelemetryContext,
        query: &TelemetryQueryV1,
    ) -> Result<Vec<TelemetryRecordV1>, ObservabilityError> {
        self.validate_query(query)?;
        let records = self.reader.query(context.tenant_id(), query)?;
        Ok(records
            .into_iter()
            .filter(|record| {
                validate_record(record).is_ok()
                    && record.envelope.tenant_id == *context.tenant_id()
                    && record_matches(record, query)
            })
            .take(query.limit)
            .collect())
    }

    #[must_use]
    pub fn guarantees(&self) -> TelemetryGuarantees {
        let sink = self.sink.guarantees();
        let reader = self.reader.guarantees();
        TelemetryGuarantees {
            durable_across_restart: sink.durable_across_restart && reader.durable_across_restart,
            visible_across_processes: sink.visible_across_processes
                && reader.visible_across_processes,
            delivery_confirmed: sink.delivery_confirmed,
            queryable: reader.queryable,
            // Deliberately mirrors the sink only, unlike the AND'd fields
            // above: `may_block` is an emit-path-only concern (see
            // `TelemetryGuarantees::may_block`), and `query()` carries no
            // synchronous hot-path contract at stake here. Folding the
            // reader's value in would either mask a blocking sink behind
            // a non-blocking reader, or falsely flag an emit-safe sink as
            // unsafe due to unrelated read-path behavior.
            may_block: sink.may_block,
        }
    }
}
