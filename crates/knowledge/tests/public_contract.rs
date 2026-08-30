use std::error::Error as _;
use std::fmt::Debug;
use std::sync::Mutex;

use knowledge::{
    DocumentId, KnowledgeDocument, KnowledgeError, KnowledgeIndex, KnowledgeService,
    MAX_DOCUMENT_TEXT_BYTES, MAX_IDENTIFIER_BYTES, MAX_QUERY_BYTES, MAX_RESULT_TEXT_BYTES,
    MAX_SEARCH_LIMIT, MAX_STATIC_DOCUMENTS, MAX_STATIC_TEXT_BYTES, NamespaceId, PrincipalId, Query,
    SearchContext, SearchLimit, SearchRequest, TenantId,
};

fn tenant(value: &str) -> TenantId {
    TenantId::new(value).expect("test tenant must be valid")
}

fn principal(value: &str) -> PrincipalId {
    PrincipalId::new(value).expect("test principal must be valid")
}

fn namespace(value: &str) -> NamespaceId {
    NamespaceId::new(value).expect("test namespace must be valid")
}

fn document_id(value: &str) -> DocumentId {
    DocumentId::new(value).expect("test document ID must be valid")
}

fn request_with(
    tenant_value: &str,
    namespace_value: &str,
    query: &str,
    limit: u32,
) -> SearchRequest {
    SearchRequest::new(
        SearchContext::new(tenant(tenant_value), principal("principal")),
        namespace(namespace_value),
        Query::new(query).expect("test query must be valid"),
        SearchLimit::new(limit).expect("test limit must be valid"),
    )
}

fn request(limit: u32) -> SearchRequest {
    request_with("tenant", "namespace", "needle", limit)
}

fn document(
    tenant_value: &str,
    namespace_value: &str,
    id: &str,
    text: impl Into<String>,
) -> KnowledgeDocument {
    KnowledgeDocument::new(
        tenant(tenant_value),
        namespace(namespace_value),
        document_id(id),
        text,
    )
    .expect("test document must be valid")
}

#[derive(Debug)]
struct ScriptedIndex {
    response: Result<Vec<KnowledgeDocument>, KnowledgeError>,
    calls: Mutex<usize>,
}

impl ScriptedIndex {
    fn documents(documents: Vec<KnowledgeDocument>) -> Self {
        Self {
            response: Ok(documents),
            calls: Mutex::new(0),
        }
    }

    fn error(error: KnowledgeError) -> Self {
        Self {
            response: Err(error),
            calls: Mutex::new(0),
        }
    }
}

impl KnowledgeIndex for ScriptedIndex {
    fn search(&self, _request: &SearchRequest) -> Result<Vec<KnowledgeDocument>, KnowledgeError> {
        *self
            .calls
            .lock()
            .expect("call counter lock must not be poisoned") += 1;
        self.response.clone()
    }
}

fn assert_identifier_derives<T>(left: &T, right: &T)
where
    T: Clone + Debug + Eq + Ord + PartialEq + PartialOrd,
{
    assert!(left < right);
    let cloned: T = T::clone(left);
    assert_eq!(&cloned, left);
    assert!(!format!("{left:?}").is_empty());
}

#[test]
fn public_constants_have_the_exact_contract_values() {
    assert_eq!(MAX_IDENTIFIER_BYTES, 128);
    assert_eq!(MAX_QUERY_BYTES, 16 * 1024);
    assert_eq!(MAX_SEARCH_LIMIT, 64);
    assert_eq!(MAX_DOCUMENT_TEXT_BYTES, 16 * 1024);
    assert_eq!(MAX_RESULT_TEXT_BYTES, 64 * 1024);
    assert_eq!(MAX_STATIC_DOCUMENTS, 10_000);
    assert_eq!(MAX_STATIC_TEXT_BYTES, 64 * 1024 * 1024);
}

#[test]
fn identifier_types_support_the_required_derives_and_accessors() {
    assert_identifier_derives(&tenant("a"), &tenant("b"));
    assert_identifier_derives(&principal("a"), &principal("b"));
    assert_identifier_derives(&namespace("a"), &namespace("b"));
    assert_identifier_derives(&document_id("a"), &document_id("b"));
    assert_eq!(tenant("tenant-1_x").as_str(), "tenant-1_x");
    assert_eq!(principal("principal-1_x").as_str(), "principal-1_x");
    assert_eq!(namespace("namespace-1_x").as_str(), "namespace-1_x");
    assert_eq!(document_id("document-1_x").as_str(), "document-1_x");
}

#[test]
fn all_identifier_types_enforce_every_grammar_boundary() {
    macro_rules! assert_identifier_contract {
        ($type:ty) => {{
            assert_eq!(<$type>::new(""), Err(KnowledgeError::InvalidRequest));
            assert_eq!(<$type>::new("a").expect("minimum ID").as_str(), "a");
            let exact = format!("a{}", "z".repeat(MAX_IDENTIFIER_BYTES - 1));
            assert_eq!(
                <$type>::new(exact.clone()).expect("128-byte ID").as_str(),
                exact
            );
            assert_eq!(
                <$type>::new(format!("a{}", "z".repeat(MAX_IDENTIFIER_BYTES))),
                Err(KnowledgeError::InvalidRequest)
            );
            for invalid in ["Upper", "a.b", "_leading", "-leading", "é", "a/b", "a b"] {
                assert_eq!(
                    <$type>::new(invalid),
                    Err(KnowledgeError::InvalidRequest),
                    "{invalid:?} must be rejected"
                );
            }
            assert_eq!(<$type>::new("0").expect("digit start").as_str(), "0");
            assert_eq!(
                <$type>::new("a_-9").expect("valid continuation").as_str(),
                "a_-9"
            );
        }};
    }

    assert_identifier_contract!(TenantId);
    assert_identifier_contract!(PrincipalId);
    assert_identifier_contract!(NamespaceId);
    assert_identifier_contract!(DocumentId);
}

#[test]
fn query_rejects_empty_and_all_unicode_whitespace() {
    let all_unicode_whitespace = [
        '\u{0009}', '\u{000a}', '\u{000b}', '\u{000c}', '\u{000d}', '\u{0020}', '\u{0085}',
        '\u{00a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}',
        '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}',
        '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
    ]
    .into_iter()
    .collect::<String>();
    for invalid in ["", " ", "\t\r\n", all_unicode_whitespace.as_str()] {
        assert_eq!(
            Query::new(invalid),
            Err(KnowledgeError::InvalidRequest),
            "{invalid:?} must be rejected"
        );
    }
}

#[test]
fn query_enforces_utf8_byte_limit_including_multibyte_boundaries() {
    let ascii_exact = "q".repeat(MAX_QUERY_BYTES);
    assert_eq!(
        Query::new(ascii_exact.clone())
            .expect("exact query")
            .as_str(),
        ascii_exact
    );
    assert_eq!(
        Query::new("q".repeat(MAX_QUERY_BYTES + 1)),
        Err(KnowledgeError::InvalidRequest)
    );

    let multibyte_exact = "é".repeat(MAX_QUERY_BYTES / 2);
    assert_eq!(multibyte_exact.len(), MAX_QUERY_BYTES);
    assert_eq!(
        Query::new(multibyte_exact.clone())
            .expect("exact multibyte query")
            .as_str(),
        multibyte_exact
    );
    assert_eq!(
        Query::new(format!("{multibyte_exact}é")),
        Err(KnowledgeError::InvalidRequest)
    );
}

#[test]
fn query_preserves_whitespace_case_and_unicode_without_normalization() {
    let original = "  École e\u{301}  ";
    let query = Query::new(original).expect("non-whitespace query");
    assert_eq!(query.as_str().as_bytes(), original.as_bytes());
    assert_ne!(query, Query::new("école é").expect("comparison query"));
    let cloned = query.clone();
    assert_eq!(query, cloned);
    assert!(!format!("{query:?}").is_empty());
}

#[test]
fn search_limit_accepts_only_one_through_sixty_four() {
    assert_eq!(SearchLimit::new(0), Err(KnowledgeError::InvalidRequest));
    let one = SearchLimit::new(1).expect("one is valid");
    let copied = one;
    assert_eq!(copied.get(), 1);
    assert_eq!(SearchLimit::new(64).expect("64 is valid").get(), 64);
    assert_eq!(SearchLimit::new(65), Err(KnowledgeError::InvalidRequest));
    assert!(!format!("{one:?}").is_empty());
}

#[test]
fn search_context_and_request_getters_return_exact_components() {
    let context = SearchContext::new(tenant("tenant-a"), principal("principal-a"));
    assert_eq!(context.tenant_id().as_str(), "tenant-a");
    assert_eq!(context.principal_id().as_str(), "principal-a");
    let cloned_context = context.clone();
    assert_eq!(context, cloned_context);

    let request = SearchRequest::new(
        context,
        namespace("namespace-a"),
        Query::new(" exact query ").expect("query"),
        SearchLimit::new(7).expect("limit"),
    );
    assert_eq!(request.context().tenant_id().as_str(), "tenant-a");
    assert_eq!(request.context().principal_id().as_str(), "principal-a");
    assert_eq!(request.namespace().as_str(), "namespace-a");
    assert_eq!(request.query().as_str(), " exact query ");
    assert_eq!(request.limit().get(), 7);
    assert_eq!(request, request.clone());
    assert!(!format!("{request:?}").is_empty());
}

#[test]
fn knowledge_document_validates_text_and_exposes_exact_values() {
    assert_eq!(
        KnowledgeDocument::new(
            tenant("tenant"),
            namespace("namespace"),
            document_id("id"),
            ""
        ),
        Err(KnowledgeError::InvalidRequest)
    );
    let exact = "é".repeat(MAX_DOCUMENT_TEXT_BYTES / 2);
    let value = document("tenant", "namespace", "id", exact.clone());
    assert_eq!(value.tenant_id().as_str(), "tenant");
    assert_eq!(value.namespace().as_str(), "namespace");
    assert_eq!(value.document_id().as_str(), "id");
    assert_eq!(value.text().as_bytes(), exact.as_bytes());
    assert_eq!(value, value.clone());
    assert!(!format!("{value:?}").is_empty());
    assert_eq!(
        KnowledgeDocument::new(
            tenant("tenant"),
            namespace("namespace"),
            document_id("id"),
            format!("{exact}x")
        ),
        Err(KnowledgeError::InvalidRequest)
    );
}

#[test]
fn knowledge_error_has_exact_closed_data_free_behavior() {
    let cases = [
        (
            KnowledgeError::InvalidRequest,
            "invalid_request",
            "InvalidRequest",
        ),
        (
            KnowledgeError::LimitExceeded,
            "limit_exceeded",
            "LimitExceeded",
        ),
        (KnowledgeError::Unavailable, "unavailable", "Unavailable"),
        (
            KnowledgeError::ProtocolViolation,
            "protocol_violation",
            "ProtocolViolation",
        ),
    ];
    for (error, display, debug) in cases {
        let copied = error;
        assert_eq!(copied.to_string(), display);
        assert_eq!(format!("{copied:?}"), debug);
        assert!(copied.source().is_none());
        assert_eq!(copied, copied.clone());
        for secret in ["tenant", "principal", "namespace", "query", "document"] {
            assert!(!copied.to_string().contains(secret));
            assert!(!format!("{copied:?}").contains(secret));
        }
    }
}

#[test]
fn service_accepts_canonical_scoped_output_and_projects_only_id_and_text() {
    let index = ScriptedIndex::documents(vec![
        document("tenant", "namespace", "a", "first needle"),
        document("tenant", "namespace", "b", "second needle"),
    ]);
    let result = KnowledgeService::new(&index)
        .search(&request(2))
        .expect("valid output");
    assert_eq!(result.hits().len(), 2);
    assert_eq!(result.hits()[0].document_id().as_str(), "a");
    assert_eq!(result.hits()[0].text(), "first needle");
    assert_eq!(result.hits()[1].document_id().as_str(), "b");
    assert_eq!(result, result.clone());
    assert!(!format!("{result:?}").is_empty());
}

#[test]
fn service_rejects_foreign_tenant_or_namespace() {
    for foreign in [
        document("other", "namespace", "a", "needle"),
        document("tenant", "other", "a", "needle"),
    ] {
        let index = ScriptedIndex::documents(vec![foreign]);
        assert_eq!(
            KnowledgeService::new(&index).search(&request(1)),
            Err(KnowledgeError::ProtocolViolation)
        );
    }
}

#[test]
fn service_rejects_duplicate_and_nonascending_document_ids() {
    for documents in [
        vec![
            document("tenant", "namespace", "a", "needle one"),
            document("tenant", "namespace", "a", "needle two"),
        ],
        vec![
            document("tenant", "namespace", "b", "needle two"),
            document("tenant", "namespace", "a", "needle one"),
        ],
    ] {
        let index = ScriptedIndex::documents(documents);
        assert_eq!(
            KnowledgeService::new(&index).search(&request(2)),
            Err(KnowledgeError::ProtocolViolation)
        );
    }
}

#[test]
fn service_rejects_count_above_request_limit() {
    let index = ScriptedIndex::documents(vec![
        document("tenant", "namespace", "a", "needle"),
        document("tenant", "namespace", "b", "needle"),
    ]);
    assert_eq!(
        KnowledgeService::new(&index).search(&request(1)),
        Err(KnowledgeError::LimitExceeded)
    );
}

#[test]
fn service_accepts_exact_aggregate_and_rejects_one_over_without_partial_result() {
    let exact = vec![
        document(
            "tenant",
            "namespace",
            "a",
            "a".repeat(MAX_DOCUMENT_TEXT_BYTES),
        ),
        document(
            "tenant",
            "namespace",
            "b",
            "b".repeat(MAX_DOCUMENT_TEXT_BYTES),
        ),
        document(
            "tenant",
            "namespace",
            "c",
            "c".repeat(MAX_DOCUMENT_TEXT_BYTES),
        ),
        document(
            "tenant",
            "namespace",
            "d",
            "d".repeat(MAX_DOCUMENT_TEXT_BYTES),
        ),
    ];
    assert_eq!(
        KnowledgeService::new(&ScriptedIndex::documents(exact))
            .search(&request(4))
            .expect("exact aggregate")
            .hits()
            .len(),
        4
    );

    let over = vec![
        document(
            "tenant",
            "namespace",
            "a",
            "a".repeat(MAX_DOCUMENT_TEXT_BYTES),
        ),
        document(
            "tenant",
            "namespace",
            "b",
            "b".repeat(MAX_DOCUMENT_TEXT_BYTES),
        ),
        document(
            "tenant",
            "namespace",
            "c",
            "c".repeat(MAX_DOCUMENT_TEXT_BYTES),
        ),
        document(
            "tenant",
            "namespace",
            "d",
            "d".repeat(MAX_DOCUMENT_TEXT_BYTES),
        ),
        document("tenant", "namespace", "e", "x"),
    ];
    let outcome = KnowledgeService::new(&ScriptedIndex::documents(over)).search(&request(5));
    assert_eq!(outcome, Err(KnowledgeError::LimitExceeded));
}

#[test]
fn service_passes_adapter_errors_through_unchanged() {
    for error in [
        KnowledgeError::Unavailable,
        KnowledgeError::ProtocolViolation,
    ] {
        let index = ScriptedIndex::error(error);
        assert_eq!(
            KnowledgeService::new(&index).search(&request(1)),
            Err(error)
        );
    }
}

#[test]
fn service_is_deterministic_for_repeated_searches() {
    let index = ScriptedIndex::documents(vec![document("tenant", "namespace", "a", "needle")]);
    let service = KnowledgeService::new(&index);
    let first = service.search(&request(1)).expect("first search");
    let second = service.search(&request(1)).expect("second search");
    assert_eq!(first, second);
    assert_eq!(*index.calls.lock().expect("call counter"), 2);
}

#[test]
fn knowledge_index_is_object_safe_send_sync_and_usable_by_service() {
    fn assert_send_sync<T: Send + Sync + ?Sized>() {}
    assert_send_sync::<dyn KnowledgeIndex>();

    let concrete = ScriptedIndex::documents(Vec::new());
    let index: &dyn KnowledgeIndex = &concrete;
    let service: KnowledgeService<'_, dyn KnowledgeIndex> = KnowledgeService::new(index);
    assert!(
        service
            .search(&request(1))
            .expect("trait-object search")
            .hits()
            .is_empty()
    );
}

#[test]
fn crate_source_and_manifest_preserve_the_bounded_framework_free_surface() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("read manifest");
    assert!(manifest.contains("status = \"implemented\""));
    assert!(manifest.contains("default = []"));
    assert!(manifest.contains("static = []"));
    assert!(!manifest.contains("[dependencies]"));
    assert!(!manifest.contains("[dev-dependencies]"));

    let source_names = [
        "lib.rs",
        "model.rs",
        "validation.rs",
        "error.rs",
        "port.rs",
        "service.rs",
        "static.rs",
    ];
    let mut source = String::new();
    for name in source_names {
        source
            .push_str(&std::fs::read_to_string(root.join("src").join(name)).expect("read source"));
    }
    for forbidden in [
        "serde",
        "schemars",
        "rmcp",
        "tantivy",
        "tokio",
        "async fn",
        ".await",
        "std::fs",
        "std::net",
        "TcpStream",
        "UdpSocket",
        "serve_stdio",
        "Settings",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden source symbol {forbidden:?} must remain absent"
        );
    }

    let tests = std::fs::read_to_string(root.join("tests/public_contract.rs")).expect("read tests");
    assert!(
        !tests
            .lines()
            .any(|line| line.trim_start().starts_with("#[ignore"))
    );
    assert!(!tests.lines().any(|line| {
        let attribute = line.trim_start();
        attribute.starts_with("#[cfg_attr") && attribute.contains("ignore")
    }));
}

#[cfg(feature = "static")]
mod static_contract {
    use super::*;
    use knowledge::r#static::StaticKnowledgeIndex;

    fn search(
        index: &StaticKnowledgeIndex,
        request: &SearchRequest,
    ) -> Result<Vec<KnowledgeDocument>, KnowledgeError> {
        KnowledgeIndex::search(index, request)
    }

    #[test]
    fn empty_corpus_returns_empty_results() {
        let index = StaticKnowledgeIndex::new(Vec::new()).expect("empty corpus");
        assert!(search(&index, &request(64)).expect("search").is_empty());
    }

    #[test]
    fn duplicate_scoped_key_is_rejected_but_same_id_in_distinct_scopes_is_allowed() {
        let duplicate = vec![
            document("tenant", "namespace", "id", "one"),
            document("tenant", "namespace", "id", "two"),
        ];
        assert!(matches!(
            StaticKnowledgeIndex::new(duplicate),
            Err(KnowledgeError::InvalidRequest)
        ));

        let distinct = vec![
            document("tenant-a", "namespace", "id", "needle"),
            document("tenant-b", "namespace", "id", "needle"),
            document("tenant-a", "other", "id", "needle"),
        ];
        assert!(StaticKnowledgeIndex::new(distinct).is_ok());
    }

    #[test]
    fn corpus_document_count_accepts_exact_limit_and_rejects_one_over() {
        let make = |count: usize| {
            (0..count)
                .map(|number| document("tenant", "namespace", &format!("d{number:05}"), "x"))
                .collect::<Vec<_>>()
        };
        assert!(StaticKnowledgeIndex::new(make(MAX_STATIC_DOCUMENTS)).is_ok());
        assert!(matches!(
            StaticKnowledgeIndex::new(make(MAX_STATIC_DOCUMENTS + 1)),
            Err(KnowledgeError::LimitExceeded)
        ));
    }

    #[test]
    fn corpus_text_bytes_accept_exact_64_mib_and_reject_one_over() {
        let full_document_count = MAX_STATIC_TEXT_BYTES / MAX_DOCUMENT_TEXT_BYTES;
        let make_exact = || {
            (0..full_document_count)
                .map(|number| {
                    document(
                        "tenant",
                        "namespace",
                        &format!("d{number:05}"),
                        "x".repeat(MAX_DOCUMENT_TEXT_BYTES),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(full_document_count, 4096);
        assert!(StaticKnowledgeIndex::new(make_exact()).is_ok());

        let mut over = make_exact();
        over.push(document("tenant", "namespace", "overflow", "x"));
        assert!(matches!(
            StaticKnowledgeIndex::new(over),
            Err(KnowledgeError::LimitExceeded)
        ));
    }

    #[test]
    fn matching_is_case_sensitive_and_exact_without_normalization() {
        let index = StaticKnowledgeIndex::new(vec![
            document("tenant", "namespace", "a", "Needle café"),
            document("tenant", "namespace", "b", "needle cafe\u{301}"),
        ])
        .expect("corpus");
        assert!(
            search(&index, &request_with("tenant", "namespace", "NEEDLE", 64))
                .expect("uppercase search")
                .is_empty()
        );
        let composed = search(&index, &request_with("tenant", "namespace", "café", 64))
            .expect("composed search");
        assert_eq!(composed.len(), 1);
        assert_eq!(composed[0].document_id().as_str(), "a");
        let decomposed = search(
            &index,
            &request_with("tenant", "namespace", "cafe\u{301}", 64),
        )
        .expect("decomposed search");
        assert_eq!(decomposed.len(), 1);
        assert_eq!(decomposed[0].document_id().as_str(), "b");
    }

    #[test]
    fn tenant_and_namespace_isolation_precede_matching() {
        let index = StaticKnowledgeIndex::new(vec![
            document("tenant", "namespace", "a", "needle allowed"),
            document("other", "namespace", "b", "needle foreign tenant"),
            document("tenant", "other", "c", "needle foreign namespace"),
        ])
        .expect("corpus");
        let results = search(&index, &request(64)).expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].document_id().as_str(), "a");
    }

    #[test]
    fn principal_does_not_partition_static_corpus() {
        let index = StaticKnowledgeIndex::new(vec![document("tenant", "namespace", "a", "needle")])
            .expect("corpus");
        let first = request(64);
        let second = SearchRequest::new(
            SearchContext::new(tenant("tenant"), principal("different-principal")),
            namespace("namespace"),
            Query::new("needle").expect("query"),
            SearchLimit::new(64).expect("limit"),
        );
        assert_eq!(search(&index, &first), search(&index, &second));
    }

    #[test]
    fn canonical_first_n_is_independent_of_input_order_and_honors_limit() {
        let index = StaticKnowledgeIndex::new(vec![
            document("tenant", "namespace", "z", "needle"),
            document("tenant", "namespace", "a", "needle"),
            document("tenant", "namespace", "m", "needle"),
        ])
        .expect("corpus");
        let results = search(&index, &request(2)).expect("search");
        let ids = results
            .iter()
            .map(|value| value.document_id().as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["a", "m"]);
    }

    #[test]
    fn repeated_static_search_is_deterministic_through_service_and_trait_object() {
        let concrete = StaticKnowledgeIndex::new(vec![
            document("tenant", "namespace", "b", "needle"),
            document("tenant", "namespace", "a", "needle"),
        ])
        .expect("corpus");
        let index: &dyn KnowledgeIndex = &concrete;
        let service = KnowledgeService::new(index);
        let first = service.search(&request(64)).expect("first");
        let second = service.search(&request(64)).expect("second");
        assert_eq!(first, second);
    }
}
