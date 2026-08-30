use crate::model::{AuthorizationDecisionV1, AuthorizationRequestV1, TokenPresentation};

/// The consumed authorization port.
///
/// A single synchronous, infallible method: any failure — crypto, parse,
/// expiry, limit, or policy denial — becomes [`crate::deny_decision`]. `token`
/// contains untrusted caller credentials; trust is established only when an
/// adapter verifies its signature and Datalog against a host-injected public
/// root key. Implementations must not place token bytes in logs, errors, or
/// evidence. Request and correlation identifiers bind request scope and are not
/// identity.
pub trait AuthorizationResolver: Send + Sync {
    fn authorize(
        &self,
        request: AuthorizationRequestV1,
        token: &TokenPresentation,
    ) -> AuthorizationDecisionV1;
}
