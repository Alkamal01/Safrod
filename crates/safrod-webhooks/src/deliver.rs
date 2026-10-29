use std::time::Duration;

use chrono::Utc;
use safrod_core::webhook::{SIGNATURE_HEADER, sign};
use safrod_storage::events::event_envelope;
use safrod_storage::models::DueDelivery;
use safrod_storage::webhooks::DeliveryOutcome;
use safrod_storage::{Db, StorageError};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use crate::url_guard::{resolve_checked, validate_url};

#[derive(Debug, Clone)]
pub struct DeliveryConfig {
    pub allow_private: bool,
    pub max_attempts: i32,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    pub request_timeout: Duration,
    pub poll_interval: Duration,
    pub lease: Duration,
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self {
            allow_private: false,
            max_attempts: 8,
            base_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_secs(3600),
            request_timeout: Duration::from_secs(10),
            poll_interval: Duration::from_secs(1),
            lease: Duration::from_secs(60),
        }
    }
}

pub struct Worker {
    db: Db,
    config: DeliveryConfig,
}

impl Worker {
    pub fn new(db: Db, config: DeliveryConfig) -> Self {
        Self { db, config }
    }

    /// Delivers everything currently due and returns how many were attempted.
    pub async fn deliver_due(&self) -> Result<usize, StorageError> {
        let due = self
            .db
            .claim_due_deliveries(20, self.config.lease.as_secs() as i64)
            .await?;
        let count = due.len();
        for delivery in due {
            let outcome = self.attempt(&delivery).await;
            self.db.record_delivery(&delivery.id, outcome).await?;
        }
        Ok(count)
    }

    async fn attempt(&self, delivery: &DueDelivery) -> DeliveryOutcome {
        let attempt_number = delivery.attempts + 1;
        match self.send(delivery).await {
            Ok(status) if (200..300).contains(&status) => {
                debug!(delivery = %delivery.id, status, "webhook delivered");
                DeliveryOutcome::Delivered {
                    status: i32::from(status),
                }
            }
            Ok(status) => self.failure(
                attempt_number,
                Some(i32::from(status)),
                format!("http {status}"),
            ),
            Err(message) => self.failure(attempt_number, None, message),
        }
    }

    fn failure(&self, attempt_number: i32, status: Option<i32>, error: String) -> DeliveryOutcome {
        if attempt_number >= self.config.max_attempts {
            warn!(attempt_number, %error, "webhook delivery gave up");
            return DeliveryOutcome::Failed { status, error };
        }
        let delay = backoff(
            attempt_number,
            self.config.base_backoff,
            self.config.max_backoff,
        );
        DeliveryOutcome::Retry {
            status,
            error,
            next_attempt_at: Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default(),
        }
    }

    async fn send(&self, delivery: &DueDelivery) -> Result<u16, String> {
        let url =
            validate_url(&delivery.url, self.config.allow_private).map_err(|e| e.to_string())?;
        let addr = resolve_checked(&url, self.config.allow_private)
            .await
            .map_err(|e| e.to_string())?;
        let host = url
            .host_str()
            .ok_or("missing host")?
            .trim_matches(['[', ']'])
            .to_string();

        // Pin the connection to the address that was just checked and never
        // follow redirects, which could point at an internal destination.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.config.request_timeout)
            .resolve(&host, addr)
            .build()
            .map_err(|e| e.to_string())?;

        let body = serde_json::to_vec(&event_envelope(
            &delivery.event_id,
            &delivery.event_type,
            delivery.event_created_at,
            delivery.simulated,
            &delivery.data,
        ))
        .map_err(|e| e.to_string())?;
        let signature = sign(delivery.secret.as_bytes(), Utc::now().timestamp(), &body);
        let response = client
            .post(url)
            .header("content-type", "application/json")
            .header("user-agent", "safrod-webhooks/0.1")
            .header(SIGNATURE_HEADER, signature)
            .header("Safrod-Event-Id", &delivery.event_id)
            .header("Safrod-Delivery-Id", &delivery.id)
            .body(body)
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        Ok(response.status().as_u16())
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        info!("webhook worker started");
        loop {
            match self.deliver_due().await {
                Ok(_) => {}
                Err(e) => error!(error = %e, "webhook delivery pass failed"),
            }
            tokio::select! {
                _ = tokio::time::sleep(self.config.poll_interval) => {}
                _ = shutdown.changed() => break,
            }
        }
    }
}

fn backoff(attempt_number: i32, base: Duration, max: Duration) -> Duration {
    let exponent = (attempt_number.max(1) - 1).min(20) as u32;
    base.saturating_mul(2u32.saturating_pow(exponent)).min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let base = Duration::from_secs(5);
        let max = Duration::from_secs(60);
        assert_eq!(backoff(1, base, max), Duration::from_secs(5));
        assert_eq!(backoff(2, base, max), Duration::from_secs(10));
        assert_eq!(backoff(3, base, max), Duration::from_secs(20));
        assert_eq!(backoff(10, base, max), max);
        assert_eq!(backoff(1000, base, max), max);
    }
}
