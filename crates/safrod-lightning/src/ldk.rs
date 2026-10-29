use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use ldk_node::bitcoin::Network as BtcNetwork;
use ldk_node::lightning::ln::channelmanager::PaymentId;
use ldk_node::lightning::ln::msgs::SocketAddress;
use ldk_node::lightning_invoice::{Bolt11Invoice, Bolt11InvoiceDescription, Description};
use ldk_node::payment::{PaymentDirection, PaymentKind, PaymentStatus};
use ldk_node::{Builder, Event, Node, NodeError};
use safrod_core::network::Network;

use crate::{
    ChannelInfo, CreatedInvoice, LightningBackend, LightningError, NewInvoice, NodeBalances,
    NodeEvent, NodeInfo, NodePayment, NodePaymentStatus, ParsedInvoice,
};

pub struct LdkConfig {
    pub network: Network,
    pub storage_dir: PathBuf,
    pub listen: String,
    pub alias: String,
    pub rpc_host: String,
    pub rpc_port: u16,
    pub rpc_user: String,
    pub rpc_password: String,
}

pub struct LdkBackend {
    node: Arc<Node>,
    network: Network,
}

impl LdkBackend {
    /// Builds and starts the node. Blocking: call from `spawn_blocking`.
    pub fn start(config: LdkConfig) -> Result<Arc<Self>, LightningError> {
        let btc_network = match config.network {
            Network::Regtest => BtcNetwork::Regtest,
            Network::Signet => BtcNetwork::Signet,
            Network::Testnet => BtcNetwork::Testnet,
            Network::Bitcoin => BtcNetwork::Bitcoin,
        };
        let listen = SocketAddress::from_str(&config.listen)
            .map_err(|_| LightningError::Node(format!("bad listen address {}", config.listen)))?;

        let mut builder = Builder::new();
        builder.set_network(btc_network);
        builder.set_storage_dir_path(config.storage_dir.to_string_lossy().into_owned());
        builder.set_chain_source_bitcoind_rpc(
            config.rpc_host,
            config.rpc_port,
            config.rpc_user,
            config.rpc_password,
        );
        builder
            .set_listening_addresses(vec![listen])
            .map_err(|e| LightningError::Node(e.to_string()))?;
        builder
            .set_node_alias(config.alias)
            .map_err(|e| LightningError::Node(e.to_string()))?;
        let node = builder
            .build()
            .map_err(|e| LightningError::Node(e.to_string()))?;
        node.start().map_err(node_err)?;
        Ok(Arc::new(Self {
            node: Arc::new(node),
            network: config.network,
        }))
    }

    /// Waits for the next node event. The event stays queued in the node until
    /// `event_handled` is called, so a crash before that point redelivers it;
    /// handlers must therefore be idempotent.
    pub async fn next_event(&self) -> NodeEvent {
        map_event(self.node.next_event_async().await)
    }

    pub fn event_handled(&self) -> Result<(), LightningError> {
        self.node.event_handled().map_err(node_err)
    }

    pub fn node(&self) -> &Arc<Node> {
        &self.node
    }

    pub fn stop(&self) -> Result<(), LightningError> {
        self.node.stop().map_err(node_err)
    }
}

fn node_err(err: NodeError) -> LightningError {
    match err {
        NodeError::NotRunning => LightningError::NotRunning,
        NodeError::DuplicatePayment => LightningError::DuplicatePayment,
        NodeError::InsufficientFunds => LightningError::NoRoute,
        other => LightningError::Node(other.to_string()),
    }
}

fn map_event(event: Event) -> NodeEvent {
    match event {
        Event::PaymentReceived {
            payment_hash,
            amount_msat,
            ..
        } => NodeEvent::PaymentReceived {
            payment_hash: hex::encode(payment_hash.0),
            amount_msat,
        },
        Event::PaymentSuccessful {
            payment_hash,
            payment_preimage,
            fee_paid_msat,
            ..
        } => NodeEvent::PaymentSuccessful {
            payment_hash: hex::encode(payment_hash.0),
            preimage: payment_preimage.map(|p| hex::encode(p.0)),
            fee_msat: fee_paid_msat,
        },
        Event::PaymentFailed {
            payment_hash,
            reason,
            ..
        } => NodeEvent::PaymentFailed {
            payment_hash: payment_hash.map(|h| hex::encode(h.0)),
            reason: reason
                .map(|r| format!("{r:?}"))
                .unwrap_or_else(|| "unknown".to_string()),
        },
        Event::ChannelReady { channel_id, .. } => NodeEvent::ChannelReady {
            channel_id: channel_id.to_string(),
        },
        Event::ChannelClosed {
            channel_id, reason, ..
        } => NodeEvent::ChannelClosed {
            channel_id: channel_id.to_string(),
            reason: reason
                .map(|r| r.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        },
        other => NodeEvent::Other(format!("{other:?}").chars().take(80).collect()),
    }
}

impl LightningBackend for LdkBackend {
    fn info(&self) -> NodeInfo {
        let status = self.node.status();
        NodeInfo {
            node_id: self.node.node_id().to_string(),
            network: self.network.as_str(),
            listening_addresses: self
                .node
                .listening_addresses()
                .unwrap_or_default()
                .iter()
                .map(|a| a.to_string())
                .collect(),
            best_block_height: status.current_best_block.height,
            running: status.is_running,
        }
    }

    fn create_invoice(&self, request: &NewInvoice) -> Result<CreatedInvoice, LightningError> {
        let description = Description::new(request.description.clone())
            .map_err(|e| LightningError::InvalidInvoice(e.to_string()))?;
        let invoice = self
            .node
            .bolt11_payment()
            .receive(
                request.amount_msat,
                &Bolt11InvoiceDescription::Direct(description),
                request.expiry_secs,
            )
            .map_err(node_err)?;
        Ok(CreatedInvoice {
            bolt11: invoice.to_string(),
            payment_hash: hex::encode(invoice.payment_hash().as_ref() as &[u8]),
            expires_at_unix: invoice
                .expires_at()
                .map(|d| d.as_secs())
                .unwrap_or(invoice.duration_since_epoch().as_secs() + request.expiry_secs as u64),
        })
    }

    fn parse_invoice(&self, bolt11: &str) -> Result<ParsedInvoice, LightningError> {
        parse_for_network(bolt11, self.network)
    }

    fn send_payment(
        &self,
        bolt11: &str,
        amount_msat: Option<u64>,
    ) -> Result<String, LightningError> {
        let parsed = parse_for_network(bolt11, self.network)?;
        let invoice = Bolt11Invoice::from_str(bolt11.trim())
            .map_err(|e| LightningError::InvalidInvoice(e.to_string()))?;
        let payments = self.node.bolt11_payment();
        let id = match (parsed.amount_msat, amount_msat) {
            (Some(_), _) => payments.send(&invoice, None),
            (None, Some(msat)) => payments.send_using_amount(&invoice, msat, None),
            (None, None) => return Err(LightningError::AmountRequired),
        }
        .map_err(node_err)?;
        Ok(hex::encode(id.0))
    }

    fn payment(&self, payment_hash: &str) -> Option<NodePayment> {
        let bytes: [u8; 32] = hex::decode(payment_hash).ok()?.try_into().ok()?;
        let details = self.node.payment(&PaymentId(bytes))?;
        if details.direction != PaymentDirection::Outbound {
            return None;
        }
        let preimage = match details.kind {
            PaymentKind::Bolt11 { preimage, .. } => preimage.map(|p| hex::encode(p.0)),
            _ => None,
        };
        Some(NodePayment {
            status: match details.status {
                PaymentStatus::Pending => NodePaymentStatus::Pending,
                PaymentStatus::Succeeded => NodePaymentStatus::Succeeded,
                PaymentStatus::Failed => NodePaymentStatus::Failed,
            },
            preimage,
            fee_msat: details.fee_paid_msat,
        })
    }

    fn new_onchain_address(&self) -> Result<String, LightningError> {
        self.node
            .onchain_payment()
            .new_address()
            .map(|a| a.to_string())
            .map_err(node_err)
    }

    fn channels(&self) -> Vec<ChannelInfo> {
        self.node
            .list_channels()
            .into_iter()
            .map(|c| ChannelInfo {
                channel_id: c.channel_id.to_string(),
                peer_node_id: c.counterparty_node_id.to_string(),
                capacity_sat: c.channel_value_sats,
                outbound_capacity_msat: c.outbound_capacity_msat,
                inbound_capacity_msat: c.inbound_capacity_msat,
                is_ready: c.is_channel_ready,
                is_usable: c.is_usable,
            })
            .collect()
    }

    fn balances(&self) -> NodeBalances {
        let balances = self.node.list_balances();
        let channels = self.node.list_channels();
        NodeBalances {
            onchain_total_sat: balances.total_onchain_balance_sats,
            onchain_spendable_sat: balances.spendable_onchain_balance_sats,
            lightning_total_sat: balances.total_lightning_balance_sats,
            outbound_capacity_msat: channels.iter().map(|c| c.outbound_capacity_msat).sum(),
            inbound_capacity_msat: channels.iter().map(|c| c.inbound_capacity_msat).sum(),
        }
    }
}

pub(crate) fn parse_for_network(
    bolt11: &str,
    network: Network,
) -> Result<ParsedInvoice, LightningError> {
    let trimmed = bolt11.trim();
    network.check_invoice_prefix(trimmed).map_err(|e| match e {
        safrod_core::CoreError::NetworkMismatch {
            expected, found, ..
        } => LightningError::NetworkMismatch { expected, found },
        other => LightningError::InvalidInvoice(other.to_string()),
    })?;
    let invoice = Bolt11Invoice::from_str(trimmed)
        .map_err(|e| LightningError::InvalidInvoice(e.to_string()))?;
    if invoice.is_expired() {
        return Err(LightningError::InvoiceExpired);
    }
    Ok(ParsedInvoice {
        payment_hash: hex::encode(invoice.payment_hash().as_ref() as &[u8]),
        amount_msat: invoice.amount_milli_satoshis(),
    })
}
