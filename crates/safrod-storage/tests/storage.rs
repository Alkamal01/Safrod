//! Run with `DATABASE_URL=postgres://… cargo test -p safrod-storage`. Each test
//! gets its own freshly migrated database from `sqlx::test`.

use chrono::{Duration, Utc};
use safrod_core::apikey;
use safrod_core::ids::{InvoiceId, PaymentId};
use safrod_core::state::PaymentStatus;
use safrod_storage::invoices::NewInvoice;
use safrod_storage::onchain::Observation;
use safrod_storage::payments::{NewPayment, Resolution};
use safrod_storage::sandbox::NewQuote;
use safrod_storage::{Db, StorageError};
use sqlx::PgPool;

async fn setup(pool: PgPool) -> (Db, String) {
    let db = Db::from_pool(pool);
    let project = db.create_project("test", "regtest").await.unwrap();
    (db, project.id)
}

fn invoice(project: &str, hash: &str) -> NewInvoice {
    NewInvoice {
        id: InvoiceId::new(),
        project_id: project.to_string(),
        idempotency_key: Some(format!("key-{hash}")),
        amount_msat: 50_000,
        description: "coffee".into(),
        bolt11: format!("lnbcrt500n1{hash}"),
        payment_hash: hash.to_string(),
        expires_at: Utc::now() + Duration::hours(1),
        metadata: serde_json::json!({}),
    }
}

fn payment(project: &str, key: &str, request: u8) -> NewPayment {
    NewPayment {
        id: PaymentId::new(),
        project_id: project.to_string(),
        idempotency_key: key.to_string(),
        request_hash: vec![request; 32],
        bolt11: "lnbcrt1".into(),
        payment_hash: format!("hash-{key}"),
        amount_msat: 10_000,
        metadata: serde_json::json!({}),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn api_key_authenticates_until_revoked(pool: PgPool) {
    let (db, project) = setup(pool).await;
    let key = apikey::generate("test");
    let scopes = vec!["invoices:read".to_string()];
    let record = db
        .create_api_key(&project, "ci", &scopes, &key)
        .await
        .unwrap();

    let ctx = db.authenticate(&key.hash).await.unwrap().unwrap();
    assert_eq!(ctx.project_id, project);
    assert_eq!(ctx.scopes, scopes);
    assert!(
        db.authenticate(&apikey::hash_secret("sfd_test_nope"))
            .await
            .unwrap()
            .is_none()
    );

    db.revoke_api_key(&project, &record.id).await.unwrap();
    assert!(db.authenticate(&key.hash).await.unwrap().is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn duplicate_payment_received_event_pays_invoice_once(pool: PgPool) {
    let (db, project) = setup(pool).await;
    db.create_webhook_endpoint(&project, "https://example.test/hook", "whsec_x")
        .await
        .unwrap();
    db.insert_invoice(invoice(&project, "aa11")).await.unwrap();

    let first = db
        .mark_invoice_paid("recv:aa11", "aa11", 50_000)
        .await
        .unwrap();
    assert_eq!(first.unwrap().status, "paid");
    // The node redelivers the same event after a restart.
    assert!(
        db.mark_invoice_paid("recv:aa11", "aa11", 50_000)
            .await
            .unwrap()
            .is_none()
    );
    // A different dedupe key for an already-paid invoice must not re-emit either.
    assert!(
        db.mark_invoice_paid("recv:aa11:b", "aa11", 50_000)
            .await
            .unwrap()
            .is_none()
    );

    let events = db
        .list_events(&project, None, Some("invoice.paid"), 10)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    let deliveries = db.list_deliveries(&project, 10).await.unwrap();
    assert_eq!(deliveries.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn idempotency_key_returns_the_same_payment_and_rejects_changed_requests(pool: PgPool) {
    let (db, project) = setup(pool).await;
    let first = db.begin_payment(payment(&project, "k1", 1)).await.unwrap();
    assert!(first.created);
    let again = db.begin_payment(payment(&project, "k1", 1)).await.unwrap();
    assert!(!again.created);
    assert_eq!(again.payment.id, first.payment.id);

    let err = db
        .begin_payment(payment(&project, "k1", 2))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, StorageError::IdempotencyMismatch));
}

#[sqlx::test(migrations = "./migrations")]
async fn payment_final_state_cannot_be_rewritten(pool: PgPool) {
    let (db, project) = setup(pool).await;
    let p = db
        .begin_payment(payment(&project, "k2", 1))
        .await
        .unwrap()
        .payment;
    db.transition_payment(None, &p.id, PaymentStatus::Pending, Resolution::default())
        .await
        .unwrap();
    let ok = db
        .transition_payment(
            Some("ok:1"),
            &p.id,
            PaymentStatus::Succeeded,
            Resolution {
                fee_msat: Some(3),
                preimage: Some("ff".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(ok.changed);
    assert_eq!(ok.payment.fee_msat, Some(3));

    let replay = db
        .transition_payment(
            Some("ok:1"),
            &p.id,
            PaymentStatus::Succeeded,
            Resolution::default(),
        )
        .await
        .unwrap();
    assert!(!replay.changed);

    let err = db
        .transition_payment(None, &p.id, PaymentStatus::Failed, Resolution::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(err, StorageError::IllegalTransition { .. }));
    let events = db.list_events(&project, None, None, 10).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "payment.succeeded")
            .count(),
        1
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn unknown_payment_resolves_later(pool: PgPool) {
    let (db, project) = setup(pool).await;
    let p = db
        .begin_payment(payment(&project, "k3", 1))
        .await
        .unwrap()
        .payment;
    for next in [PaymentStatus::Pending, PaymentStatus::Unknown] {
        db.transition_payment(None, &p.id, next, Resolution::default())
            .await
            .unwrap();
    }
    assert_eq!(db.unresolved_payments().await.unwrap().len(), 1);
    db.transition_payment(None, &p.id, PaymentStatus::Failed, Resolution::default())
        .await
        .unwrap();
    assert!(db.unresolved_payments().await.unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn onchain_observations_only_emit_on_change(pool: PgPool) {
    let (db, project) = setup(pool).await;
    let addr = db
        .create_address(&project, "bcrt1qexample", "t", 3)
        .await
        .unwrap();
    let obs = |confirmations| Observation {
        address: &addr,
        txid: "tx1",
        vout: 0,
        amount_sat: 25_000,
        block_height: (confirmations > 0).then_some(100),
        confirmations,
    };
    assert_eq!(
        db.record_observation(obs(0)).await.unwrap().unwrap().status,
        "unconfirmed"
    );
    assert!(db.record_observation(obs(0)).await.unwrap().is_none());
    assert_eq!(
        db.record_observation(obs(1)).await.unwrap().unwrap().status,
        "confirming"
    );
    assert_eq!(
        db.record_observation(obs(3)).await.unwrap().unwrap().status,
        "confirmed"
    );
    assert!(db.record_observation(obs(3)).await.unwrap().is_none());

    let kinds: Vec<String> = db
        .list_events(&project, None, None, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            "bitcoin.transaction.detected",
            "bitcoin.transaction.updated",
            "bitcoin.transaction.confirmed"
        ]
    );
    let totals = db.project_totals(&project).await.unwrap();
    assert_eq!(totals.onchain_confirmed_sat, 25_000);
}

#[sqlx::test(migrations = "./migrations")]
async fn webhook_claims_are_leased(pool: PgPool) {
    let (db, project) = setup(pool).await;
    db.create_webhook_endpoint(&project, "https://example.test/hook", "whsec_x")
        .await
        .unwrap();
    db.emit_standalone(
        &project,
        "test.event",
        "res_1",
        serde_json::json!({}),
        false,
    )
    .await
    .unwrap();
    let first = db.claim_due_deliveries(10, 60).await.unwrap();
    assert_eq!(first.len(), 1);
    // Leased: a second worker sees nothing until the lease runs out.
    assert!(db.claim_due_deliveries(10, 60).await.unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn stale_or_reused_quote_is_refused(pool: PgPool) {
    let (db, project) = setup(pool).await;
    let quote = |expires_at| NewQuote {
        project_id: project.clone(),
        source_currency: "NGN".into(),
        target_currency: "KES".into(),
        source_minor: 500_000,
        target_minor: 30_000,
        rate_ppm: 60_000,
        btc_amount_msat: 100_000,
        expires_at,
    };
    db.credit_sandbox_balance(&project, "NGN", 1_000_000)
        .await
        .unwrap();
    let fresh = db
        .insert_quote(quote(Utc::now() + Duration::minutes(5)))
        .await
        .unwrap();
    let (payout, created) = db
        .reserve_payout(&project, &fresh.id, "p1", "success", None)
        .await
        .unwrap();
    assert!(created);
    let (same, created) = db
        .reserve_payout(&project, &fresh.id, "p1", "success", None)
        .await
        .unwrap();
    assert!(!created);
    assert_eq!(same.id, payout.id);
    assert!(matches!(
        db.reserve_payout(&project, &fresh.id, "p2", "success", None)
            .await
            .err()
            .unwrap(),
        StorageError::IllegalTransition { .. }
    ));

    let stale = db
        .insert_quote(quote(Utc::now() - Duration::minutes(1)))
        .await
        .unwrap();
    assert!(matches!(
        db.reserve_payout(&project, &stale.id, "p3", "success", None)
            .await
            .err()
            .unwrap(),
        StorageError::IllegalTransition { .. }
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn reservation_needs_balance_and_failure_refunds_it(pool: PgPool) {
    use safrod_core::state::PayoutStatus;
    let (db, project) = setup(pool).await;
    let quote = db
        .insert_quote(NewQuote {
            project_id: project.clone(),
            source_currency: "NGN".into(),
            target_currency: "KES".into(),
            source_minor: 500_000,
            target_minor: 30_000,
            rate_ppm: 60_000,
            btc_amount_msat: 100_000,
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .unwrap();
    assert!(matches!(
        db.reserve_payout(&project, &quote.id, "p1", "failure", None)
            .await
            .err()
            .unwrap(),
        StorageError::InsufficientBalance
    ));

    db.credit_sandbox_balance(&project, "NGN", 600_000)
        .await
        .unwrap();
    let (payout, _) = db
        .reserve_payout(&project, &quote.id, "p1", "failure", None)
        .await
        .unwrap();
    assert_eq!(
        db.sandbox_balances(&project).await.unwrap(),
        [("NGN".to_string(), 100_000)]
    );

    db.advance_payout(&payout.id, PayoutStatus::Pending)
        .await
        .unwrap();
    db.advance_payout(&payout.id, PayoutStatus::Failed)
        .await
        .unwrap();
    assert_eq!(
        db.sandbox_balances(&project).await.unwrap(),
        [("NGN".to_string(), 600_000)]
    );

    let (_, changed) = db
        .advance_payout(&payout.id, PayoutStatus::Failed)
        .await
        .unwrap();
    assert!(!changed);
    assert!(matches!(
        db.advance_payout(&payout.id, PayoutStatus::Succeeded)
            .await
            .err()
            .unwrap(),
        StorageError::IllegalTransition { .. }
    ));
}
