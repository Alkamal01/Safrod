//! Delivery against a real local HTTP receiver. Needs `DATABASE_URL`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use safrod_core::webhook::{SIGNATURE_HEADER, verify};
use safrod_storage::Db;
use safrod_webhooks::{DeliveryConfig, Worker};
use sqlx::PgPool;

type Hit = (String, Vec<u8>);

#[derive(Clone, Default)]
struct Receiver {
    hits: Arc<Mutex<Vec<Hit>>>,
    fail_first: Arc<Mutex<u32>>,
}

async fn hook(State(rx): State<Receiver>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let sig = headers[SIGNATURE_HEADER].to_str().unwrap().to_string();
    rx.hits.lock().unwrap().push((sig, body.to_vec()));
    let mut remaining = rx.fail_first.lock().unwrap();
    if *remaining > 0 {
        *remaining -= 1;
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    StatusCode::OK
}

async fn serve(rx: Receiver) -> String {
    let app = Router::new().route("/hook", post(hook)).with_state(rx);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/hook")
}

fn config() -> DeliveryConfig {
    DeliveryConfig {
        allow_private: true,
        base_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(100),
        ..Default::default()
    }
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn delivers_signed_event_and_retries_after_server_error(pool: PgPool) {
    let db = Db::from_pool(pool);
    let project = db.create_project("t", "regtest").await.unwrap();
    let rx = Receiver::default();
    *rx.fail_first.lock().unwrap() = 1;
    let url = serve(rx.clone()).await;
    db.create_webhook_endpoint(&project.id, &url, "whsec_test")
        .await
        .unwrap();
    let event = db
        .emit_standalone(
            &project.id,
            "invoice.paid",
            "inv_1",
            serde_json::json!({"x": 1}),
            false,
        )
        .await
        .unwrap();

    let worker = Worker::new(db.clone(), config());
    assert_eq!(worker.deliver_due().await.unwrap(), 1);
    let deliveries = db.list_deliveries(&project.id, 10).await.unwrap();
    assert_eq!(deliveries[0].status, "pending");
    assert_eq!(deliveries[0].last_status, Some(500));

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(worker.deliver_due().await.unwrap(), 1);
    let deliveries = db.list_deliveries(&project.id, 10).await.unwrap();
    assert_eq!(deliveries[0].status, "delivered");
    assert_eq!(deliveries[0].attempts, 2);

    let hits = rx.hits.lock().unwrap();
    assert_eq!(hits.len(), 2);
    let (sig, body) = &hits[1];
    let now = chrono::Utc::now().timestamp();
    assert_eq!(verify(b"whsec_test", sig, body, now, 300), Ok(()));
    let payload: serde_json::Value = serde_json::from_slice(body).unwrap();
    assert_eq!(payload["id"], event.id);
    assert_eq!(payload["type"], "invoice.paid");
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn private_destinations_are_refused_in_production_mode(pool: PgPool) {
    let db = Db::from_pool(pool);
    let project = db.create_project("t", "regtest").await.unwrap();
    let rx = Receiver::default();
    let url = serve(rx.clone()).await;
    db.create_webhook_endpoint(&project.id, &url, "whsec_test")
        .await
        .unwrap();
    db.emit_standalone(
        &project.id,
        "invoice.paid",
        "inv_1",
        serde_json::json!({}),
        false,
    )
    .await
    .unwrap();

    let strict = DeliveryConfig {
        allow_private: false,
        ..config()
    };
    Worker::new(db.clone(), strict).deliver_due().await.unwrap();
    assert!(rx.hits.lock().unwrap().is_empty());
    let deliveries = db.list_deliveries(&project.id, 10).await.unwrap();
    assert_eq!(
        deliveries[0].last_error.as_deref(),
        Some("only https urls are allowed")
    );
}

#[sqlx::test(migrations = "../safrod-storage/migrations")]
async fn gives_up_after_max_attempts(pool: PgPool) {
    let db = Db::from_pool(pool);
    let project = db.create_project("t", "regtest").await.unwrap();
    let rx = Receiver::default();
    *rx.fail_first.lock().unwrap() = 100;
    let url = serve(rx.clone()).await;
    db.create_webhook_endpoint(&project.id, &url, "whsec_test")
        .await
        .unwrap();
    db.emit_standalone(
        &project.id,
        "invoice.paid",
        "inv_1",
        serde_json::json!({}),
        false,
    )
    .await
    .unwrap();
    let worker = Worker::new(
        db.clone(),
        DeliveryConfig {
            max_attempts: 2,
            ..config()
        },
    );
    worker.deliver_due().await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    worker.deliver_due().await.unwrap();
    let deliveries = db.list_deliveries(&project.id, 10).await.unwrap();
    assert_eq!(deliveries[0].status, "failed");
    assert_eq!(deliveries[0].attempts, 2);
}
