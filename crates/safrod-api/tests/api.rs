//! Exercises the router end to end against a real PostgreSQL database and the
//! in-memory Lightning mock. Run with `DATABASE_URL=postgres://… cargo test`.
//! Real regtest Lightning is covered by `scripts/demo.sh`.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use safrod_api::{ApiConfig, AppState, engine, router};
use safrod_core::apikey;
use safrod_core::network::Network;
use safrod_lightning::mock::MockBackend;
use safrod_lightning::{LightningError, NodeEvent, NodePayment, NodePaymentStatus};
use safrod_storage::Db;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: AppState,
    mock: Arc<MockBackend>,
    key: String,
}

async fn harness(pool: PgPool) -> Harness {
    let db = Db::from_pool(pool);
    let mock = Arc::new(MockBackend::new());
    let state = AppState {
        db: db.clone(),
        ln: mock.clone(),
        rpc: None,
        config: Arc::new(ApiConfig {
            network: Network::Regtest,
            admin_token: Some("admin-token-for-tests-0123456789".into()),
            webhook_allow_private: false,
            default_confirmations: 3,
            enable_dev_endpoints: false,
        }),
    };
    let project = db.create_project("tests", "regtest").await.unwrap();
    let generated = apikey::generate("test");
    let scopes: Vec<String> = apikey::Scope::ALL.iter().map(|s| s.to_string()).collect();
    db.create_api_key(&project.id, "t", &scopes, &generated)
        .await
        .unwrap();
    Harness {
        app: router(state.clone()),
        state,
        mock,
        key: generated.secret,
    }
}

impl Harness {
    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        idem: Option<&str>,
    ) -> (StatusCode, Value) {
        self.call_with(method, path, body, idem, Some(&self.key), &[])
            .await
    }

    async fn call_with(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        idem: Option<&str>,
        key: Option<&str>,
        extra: &[(&str, &str)],
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(key) = key {
            req = req.header("authorization", format!("Bearer {key}"));
        }
        if let Some(idem) = idem {
            req = req.header("idempotency-key", idem);
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let body = match body {
            Some(v) => Body::from(v.to_string()),
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, value)
    }

    async fn make_invoice(&self, idem: &str) -> Value {
        let (status, body) = self
            .call(
                "POST",
                "/v1/lightning/invoices",
                Some(json!({ "amount_sat": 10, "description": "coffee" })),
                Some(idem),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body
    }
}

/// An invoice the mock node accepts, with a distinct payment hash per `n`.
fn mock_invoice(n: u64) -> String {
    format!("lnbcrt100n1mock{n:064x}")
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn requests_need_a_valid_key_with_the_right_scope(pool: PgPool) {
    let h = harness(pool).await;
    let (s, body) = h
        .call_with("GET", "/v1/lightning/invoices", None, None, None, &[])
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(
        body["error"]["request_id"]
            .as_str()
            .unwrap()
            .starts_with("req_")
    );

    let (s, _) = h
        .call_with(
            "GET",
            "/v1/lightning/invoices",
            None,
            None,
            Some("sfd_test_bogus"),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let project = h
        .state
        .db
        .create_project("limited", "regtest")
        .await
        .unwrap();
    let narrow = apikey::generate("test");
    h.state
        .db
        .create_api_key(&project.id, "ro", &["invoices:read".to_string()], &narrow)
        .await
        .unwrap();
    let (s, _) = h
        .call_with(
            "GET",
            "/v1/lightning/invoices",
            None,
            None,
            Some(&narrow.secret),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = h
        .call_with(
            "POST",
            "/v1/lightning/invoices",
            Some(json!({ "amount_sat": 1 })),
            None,
            Some(&narrow.secret),
            &[],
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "forbidden");
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn admin_api_needs_the_admin_token(pool: PgPool) {
    let h = harness(pool).await;
    let body = Some(json!({ "name": "acme" }));
    let (s, _) = h
        .call_with("POST", "/v1/projects", body.clone(), None, None, &[])
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = h
        .call_with(
            "POST",
            "/v1/projects",
            body.clone(),
            None,
            None,
            &[("x-admin-token", "wrong")],
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, created) = h
        .call_with(
            "POST",
            "/v1/projects",
            body,
            None,
            None,
            &[("x-admin-token", "admin-token-for-tests-0123456789")],
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let secret = created["api_key"]["secret"].as_str().unwrap();
    assert!(secret.starts_with("sfd_test_"));
    // The new key works, and the secret is never stored in the clear.
    let (s, _) = h
        .call_with("GET", "/v1/project", None, None, Some(secret), &[])
        .await;
    assert_eq!(s, StatusCode::OK);
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn dev_endpoints_are_not_exposed_by_default(pool: PgPool) {
    let h = harness(pool).await;
    let (s, _) = h
        .call_with(
            "POST",
            "/v1/dev/mine",
            Some(json!({ "blocks": 1 })),
            None,
            None,
            &[("x-admin-token", "admin-token-for-tests-0123456789")],
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn invoice_creation_is_idempotent(pool: PgPool) {
    let h = harness(pool).await;
    let first = h.make_invoice("order-1").await;
    let (s, again) = h
        .call(
            "POST",
            "/v1/lightning/invoices",
            Some(json!({ "amount_sat": 10, "description": "coffee" })),
            Some("order-1"),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(again["id"], first["id"]);
    assert_eq!(first["amount_msat"], 10_000);
    assert_eq!(first["real_regtest_lightning"], true);

    let (s, body) = h
        .call(
            "POST",
            "/v1/lightning/invoices",
            Some(json!({ "amount_sat": 11, "description": "coffee" })),
            Some("order-1"),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "idempotency_key_reused");
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn invoice_validation_rejects_bad_amounts(pool: PgPool) {
    let h = harness(pool).await;
    for body in [
        json!({}),
        json!({ "amount_sat": 0 }),
        json!({ "amount_sat": 1, "amount_msat": 1000 }),
        json!({ "amount_sat": 21_000_000u64 * 100_000_000 + 1 }),
        json!({ "amount_sat": 1, "expiry_secs": 0 }),
        json!({ "amount_sat": 1.5 }),
    ] {
        let (s, resp) = h
            .call("POST", "/v1/lightning/invoices", Some(body.clone()), None)
            .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body} -> {resp}");
        assert_eq!(resp["error"]["code"], "validation_error");
    }
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn paid_event_marks_invoice_paid_once_even_if_replayed(pool: PgPool) {
    let h = harness(pool).await;
    let invoice = h.make_invoice("order-2").await;
    let hash = invoice["payment_hash"].as_str().unwrap().to_string();
    let event = || NodeEvent::PaymentReceived {
        payment_hash: hash.clone(),
        amount_msat: 10_000,
    };
    engine::handle_node_event(&h.state, event()).await.unwrap();
    // The node redelivers after a restart.
    engine::handle_node_event(&h.state, event()).await.unwrap();

    let id = invoice["id"].as_str().unwrap();
    let (_, paid) = h
        .call("GET", &format!("/v1/lightning/invoices/{id}"), None, None)
        .await;
    assert_eq!(paid["status"], "paid");
    assert_eq!(paid["amount_received_msat"], 10_000);

    let (_, events) = h
        .call("GET", "/v1/events?type=invoice.paid", None, None)
        .await;
    assert_eq!(events["data"].as_array().unwrap().len(), 1);
    assert_eq!(events["data"][0]["resource_id"], id);
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn payment_lifecycle_from_pending_to_succeeded(pool: PgPool) {
    let h = harness(pool).await;
    let (s, missing) = h
        .call(
            "POST",
            "/v1/lightning/payments",
            Some(json!({ "bolt11": mock_invoice(1) })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(missing["error"]["code"], "idempotency_key_required");

    let (s, pay) = h
        .call(
            "POST",
            "/v1/lightning/payments",
            Some(json!({ "bolt11": mock_invoice(1) })),
            Some("pay-1"),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{pay}");
    assert_eq!(pay["status"], "pending");
    assert_eq!(pay["preimage"], Value::Null);

    // A client retry after a timeout must return the same payment, not pay again.
    let (s, retry) = h
        .call(
            "POST",
            "/v1/lightning/payments",
            Some(json!({ "bolt11": mock_invoice(1) })),
            Some("pay-1"),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(retry["id"], pay["id"]);

    let (s, mismatch) = h
        .call(
            "POST",
            "/v1/lightning/payments",
            Some(json!({ "bolt11": mock_invoice(2) })),
            Some("pay-1"),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(mismatch["error"]["code"], "idempotency_key_reused");

    engine::handle_node_event(
        &h.state,
        NodeEvent::PaymentSuccessful {
            payment_hash: pay["payment_hash"].as_str().unwrap().to_string(),
            preimage: Some("aa".repeat(32)),
            fee_msat: Some(2),
        },
    )
    .await
    .unwrap();
    let id = pay["id"].as_str().unwrap();
    let (_, done) = h
        .call("GET", &format!("/v1/lightning/payments/{id}"), None, None)
        .await;
    assert_eq!(done["status"], "succeeded");
    assert_eq!(done["fee_msat"], 2);
    assert_eq!(done["preimage"], "aa".repeat(32));

    // A late failure event cannot undo a success.
    engine::handle_node_event(
        &h.state,
        NodeEvent::PaymentFailed {
            payment_hash: Some(done["payment_hash"].as_str().unwrap().to_string()),
            reason: "RouteNotFound".into(),
        },
    )
    .await
    .unwrap();
    let (_, still) = h
        .call("GET", &format!("/v1/lightning/payments/{id}"), None, None)
        .await;
    assert_eq!(still["status"], "succeeded");
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn malformed_and_wrong_network_invoices_are_rejected_before_any_record(pool: PgPool) {
    let h = harness(pool).await;
    for bolt11 in ["garbage", "lnbc10u1pmainnetinvoice", ""] {
        let (s, body) = h
            .call(
                "POST",
                "/v1/lightning/payments",
                Some(json!({ "bolt11": bolt11 })),
                Some("bad"),
            )
            .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bolt11}: {body}");
    }
    let (_, list) = h.call("GET", "/v1/lightning/payments", None, None).await;
    assert!(list["data"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn exhausted_liquidity_is_a_definite_failure(pool: PgPool) {
    let h = harness(pool).await;
    h.mock.fail_next_send(LightningError::NoRoute);
    let (s, pay) = h
        .call(
            "POST",
            "/v1/lightning/payments",
            Some(json!({ "bolt11": mock_invoice(7) })),
            Some("pay-7"),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(pay["status"], "failed");
    assert_eq!(pay["failure_reason"], "no_route");
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn restart_reconciliation_never_turns_ambiguity_into_failure(pool: PgPool) {
    let h = harness(pool).await;
    // Payment A: the node later reports success. Payment B: the node has no record.
    let (_, a) = h
        .call(
            "POST",
            "/v1/lightning/payments",
            Some(json!({ "bolt11": mock_invoice(10) })),
            Some("a"),
        )
        .await;
    let (_, b) = h
        .call(
            "POST",
            "/v1/lightning/payments",
            Some(json!({ "bolt11": mock_invoice(11) })),
            Some("b"),
        )
        .await;
    h.mock.set_payment(
        a["payment_hash"].as_str().unwrap(),
        NodePayment {
            status: NodePaymentStatus::Succeeded,
            preimage: Some("bb".repeat(32)),
            fee_msat: Some(1),
        },
    );
    h.mock.forget_payment(b["payment_hash"].as_str().unwrap());

    assert_eq!(engine::reconcile_all(&h.state).await.unwrap(), 2);
    let (_, a) = h
        .call(
            "GET",
            &format!("/v1/lightning/payments/{}", a["id"].as_str().unwrap()),
            None,
            None,
        )
        .await;
    let (_, b) = h
        .call(
            "GET",
            &format!("/v1/lightning/payments/{}", b["id"].as_str().unwrap()),
            None,
            None,
        )
        .await;
    assert_eq!(a["status"], "succeeded");
    assert_eq!(b["status"], "unknown");

    // Running it again changes nothing.
    assert_eq!(engine::reconcile_all(&h.state).await.unwrap(), 0);
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn webhook_urls_are_checked_for_ssrf(pool: PgPool) {
    let h = harness(pool).await;
    for url in [
        "https://127.0.0.1/hook",
        "https://169.254.169.254/latest",
        "http://example.com/hook",
        "https://user:pw@example.com/hook",
        "file:///etc/passwd",
    ] {
        let (s, body) = h
            .call(
                "POST",
                "/v1/webhook-endpoints",
                Some(json!({ "url": url })),
                None,
            )
            .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{url}: {body}");
    }
    let (s, ok) = h
        .call(
            "POST",
            "/v1/webhook-endpoints",
            Some(json!({ "url": "https://example.com/hook" })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    assert!(ok["secret"].as_str().unwrap().starts_with("whsec_"));
    let (_, list) = h.call("GET", "/v1/webhook-endpoints", None, None).await;
    assert!(list["data"][0].get("secret").is_none());
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn sandbox_scenarios_are_deterministic_and_labelled(pool: PgPool) {
    let h = harness(pool).await;
    let (s, bal) = h
        .call(
            "POST",
            "/v1/sandbox/balances",
            Some(json!({ "currency": "NGN", "minor_units": 2_000_000 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(bal["simulated_fiat"], true);

    let quote = |amount: u64| json!({ "source": { "currency": "NGN", "minor_units": amount }, "target_currency": "KES" });
    let (s, q1) = h
        .call("POST", "/v1/sandbox/quotes", Some(quote(500_000)), None)
        .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(q1["simulated_fiat"], true);
    assert_eq!(q1["target"]["minor_units"], 42_570);

    // Failure: simulated funds are refunded.
    let (s, failed) = h
        .call(
            "POST",
            "/v1/sandbox/payouts",
            Some(json!({ "quote_id": q1["id"], "scenario": "failure" })),
            Some("p-fail"),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{failed}");
    assert_eq!(failed["status"], "failed");
    let (_, bal) = h.call("GET", "/v1/sandbox/balances", None, None).await;
    assert_eq!(bal["balances"][0]["minor_units"], 2_000_000);

    // The quote is now used; replaying the same idempotency key is a no-op.
    let (s, again) = h
        .call(
            "POST",
            "/v1/sandbox/payouts",
            Some(json!({ "quote_id": q1["id"], "scenario": "failure" })),
            Some("p-fail"),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(again["id"], failed["id"]);
    let (s, stale) = h
        .call(
            "POST",
            "/v1/sandbox/payouts",
            Some(json!({ "quote_id": q1["id"], "scenario": "success" })),
            Some("p-other"),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{stale}");

    // Timeout: ambiguous until reconciled, and no Bitcoin moves meanwhile.
    let (_, q2) = h
        .call("POST", "/v1/sandbox/quotes", Some(quote(500_000)), None)
        .await;
    let invoice = format!("lnbcrt1mock{:064x}", 99);
    let (s, timeout) = h
        .call(
            "POST",
            "/v1/sandbox/payouts",
            Some(json!({ "quote_id": q2["id"], "scenario": "timeout", "settlement_invoice": invoice })),
            Some("p-timeout"),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{timeout}");
    assert_eq!(timeout["status"], "unknown");
    assert_eq!(timeout["payment_id"], Value::Null);

    let id = timeout["id"].as_str().unwrap();
    let (s, resolved) = h
        .call(
            "POST",
            &format!("/v1/sandbox/payouts/{id}/reconcile"),
            Some(json!({ "outcome": "success" })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{resolved}");
    assert_eq!(resolved["status"], "succeeded");
    assert_eq!(resolved["real_regtest_lightning"], true);
    assert_eq!(resolved["payment"]["status"], "pending");

    let (s, _) = h
        .call(
            "POST",
            &format!("/v1/sandbox/payouts/{id}/reconcile"),
            Some(json!({ "outcome": "failure" })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    let (_, events) = h
        .call("GET", "/v1/events?type=sandbox.payout.unknown", None, None)
        .await;
    assert_eq!(events["data"][0]["simulated"], true);
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn unknown_ids_and_bodies_use_the_standard_error_shape(pool: PgPool) {
    let h = harness(pool).await;
    let (s, body) = h
        .call("GET", "/v1/lightning/invoices/inv_nope", None, None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
    let (s, body) = h
        .call(
            "POST",
            "/v1/lightning/invoices",
            Some(json!("not an object")),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["request_id"].is_string());
    let (s, body) = h
        .call("GET", "/v1/lightning/invoices?limit=1000", None, None)
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn pagination_walks_all_invoices(pool: PgPool) {
    let h = harness(pool).await;
    for n in 0..5 {
        h.make_invoice(&format!("k{n}")).await;
    }
    let (_, first) = h
        .call("GET", "/v1/lightning/invoices?limit=2", None, None)
        .await;
    assert_eq!(first["data"].as_array().unwrap().len(), 2);
    assert_eq!(first["has_more"], true);
    let cursor = first["next_cursor"].as_str().unwrap();
    let (_, second) = h
        .call(
            "GET",
            &format!("/v1/lightning/invoices?limit=2&starting_after={cursor}"),
            None,
            None,
        )
        .await;
    assert_eq!(second["data"].as_array().unwrap().len(), 2);
    let (_, last) = h
        .call(
            "GET",
            &format!(
                "/v1/lightning/invoices?limit=2&starting_after={}",
                second["next_cursor"].as_str().unwrap()
            ),
            None,
            None,
        )
        .await;
    assert_eq!(last["data"].as_array().unwrap().len(), 1);
    assert_eq!(last["has_more"], false);
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn balances_keep_simulated_and_real_amounts_apart(pool: PgPool) {
    let h = harness(pool).await;
    let (_, bal) = h.call("GET", "/v1/balances", None, None).await;
    assert_eq!(bal["ledger"]["simulated"], false);
    assert_eq!(bal["simulated_fiat"]["simulated"], true);
}

/// Every operation in the published OpenAPI document must exist in the router,
/// so the documentation cannot silently drift from the implementation.
#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn openapi_document_matches_the_router(pool: PgPool) {
    let h = harness(pool).await;
    let (status, doc) = h
        .call_with("GET", "/openapi.json", None, None, None, &[])
        .await;
    assert_eq!(status, StatusCode::OK);
    let paths = doc["paths"].as_object().unwrap();
    assert!(paths.len() > 20);
    for (path, ops) in paths {
        for method in ops.as_object().unwrap().keys() {
            let concrete = path
                .replace("{id}", "x")
                .replace("{project_id}", "prj_x")
                .replace("{key_id}", "key_x");
            let response = h
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method.to_uppercase().as_str())
                        .uri(&concrete)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            assert_ne!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
            assert!(
                !(status == StatusCode::NOT_FOUND && bytes.is_empty()),
                "{method} {path} is documented but not routed"
            );
        }
    }
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn dashboard_is_served_with_a_restrictive_csp(pool: PgPool) {
    let h = harness(pool).await;
    let response = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(csp.contains("default-src 'none'") && csp.contains("connect-src 'self'"));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("Safrod"));
}

#[tokio::test]
async fn flow_builder_redirects_to_the_local_next_app() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://safrod:test@127.0.0.1/safrod_test")
        .unwrap();
    let app = router(AppState {
        db: Db::from_pool(pool),
        ln: Arc::new(MockBackend::new()),
        rpc: None,
        config: Arc::new(ApiConfig {
            network: Network::Regtest,
            admin_token: None,
            webhook_allow_private: false,
            default_confirmations: 3,
            enable_dev_endpoints: false,
        }),
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/flow-builder")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers()["location"].to_str().unwrap(),
        "http://127.0.0.1:3001/"
    );
}
