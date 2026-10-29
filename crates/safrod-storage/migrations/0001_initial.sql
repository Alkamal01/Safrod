create table projects (
    id          text primary key,
    name        text not null,
    network     text not null,
    created_at  timestamptz not null default now()
);

create table api_keys (
    id          text primary key,
    project_id  text not null references projects(id),
    label       text not null,
    prefix      text not null,
    key_hash    bytea not null unique,
    scopes      text[] not null,
    revoked_at  timestamptz,
    created_at  timestamptz not null default now()
);

create table invoices (
    id                    text primary key,
    project_id            text not null references projects(id),
    idempotency_key       text,
    amount_msat           bigint not null check (amount_msat > 0),
    description           text not null default '',
    bolt11                text not null,
    payment_hash          text not null unique,
    status                text not null check (status in ('open', 'paid', 'expired')),
    expires_at            timestamptz not null,
    paid_at               timestamptz,
    amount_received_msat  bigint,
    metadata              jsonb not null default '{}',
    created_at            timestamptz not null default now(),
    updated_at            timestamptz not null default now()
);
create unique index invoices_idempotency on invoices (project_id, idempotency_key)
    where idempotency_key is not null;
create index invoices_project_created on invoices (project_id, created_at desc, id);

create table outgoing_payments (
    id               text primary key,
    project_id       text not null references projects(id),
    idempotency_key  text not null,
    -- Detects the same idempotency key being reused for a different request.
    request_hash     bytea not null,
    bolt11           text not null,
    payment_hash     text not null,
    amount_msat      bigint not null check (amount_msat > 0),
    fee_msat         bigint,
    status           text not null check (status in ('created', 'pending', 'succeeded', 'failed', 'unknown')),
    failure_reason   text,
    preimage         text,
    metadata         jsonb not null default '{}',
    created_at       timestamptz not null default now(),
    updated_at       timestamptz not null default now(),
    unique (project_id, idempotency_key)
);
create index outgoing_payments_hash on outgoing_payments (payment_hash);
create index outgoing_payments_open on outgoing_payments (status) where status in ('created', 'pending', 'unknown');
create index outgoing_payments_project_created on outgoing_payments (project_id, created_at desc, id);

create table onchain_addresses (
    id                      text primary key,
    project_id              text not null references projects(id),
    address                 text not null unique,
    label                   text not null default '',
    required_confirmations  integer not null check (required_confirmations > 0),
    created_at              timestamptz not null default now()
);

create table onchain_transactions (
    id             text primary key,
    project_id     text not null references projects(id),
    address_id     text not null references onchain_addresses(id),
    txid           text not null,
    vout           integer not null,
    amount_sat     bigint not null check (amount_sat > 0),
    block_height   integer,
    confirmations  integer not null default 0,
    status         text not null check (status in ('unconfirmed', 'confirming', 'confirmed')),
    first_seen_at  timestamptz not null default now(),
    updated_at     timestamptz not null default now(),
    unique (txid, vout)
);
create index onchain_transactions_project on onchain_transactions (project_id, first_seen_at desc, id);

create table events (
    seq          bigserial primary key,
    id           text not null unique,
    project_id   text not null references projects(id),
    type         text not null,
    resource_id  text not null,
    data         jsonb not null,
    simulated    boolean not null default false,
    created_at   timestamptz not null default now()
);
create index events_project_seq on events (project_id, seq);

create table webhook_endpoints (
    id          text primary key,
    project_id  text not null references projects(id),
    url         text not null,
    secret      text not null,
    enabled     boolean not null default true,
    created_at  timestamptz not null default now()
);

create table webhook_deliveries (
    id               text primary key,
    event_seq        bigint not null references events(seq),
    endpoint_id      text not null references webhook_endpoints(id),
    status           text not null check (status in ('pending', 'delivered', 'failed')),
    attempts         integer not null default 0,
    next_attempt_at  timestamptz not null default now(),
    last_status      integer,
    last_error       text,
    delivered_at     timestamptz,
    created_at       timestamptz not null default now(),
    unique (event_seq, endpoint_id)
);
create index webhook_deliveries_due on webhook_deliveries (next_attempt_at) where status = 'pending';

-- LDK Node redelivers events until they are acknowledged, so each one is
-- recorded under a dedupe key before it is applied.
create table node_events (
    dedupe_key    text primary key,
    kind          text not null,
    payload       jsonb not null,
    processed_at  timestamptz not null default now()
);

create table sandbox_quotes (
    id               text primary key,
    project_id       text not null references projects(id),
    source_currency  text not null,
    target_currency  text not null,
    source_minor     bigint not null check (source_minor > 0),
    target_minor     bigint not null check (target_minor > 0),
    rate_ppm         bigint not null,
    btc_amount_msat  bigint not null check (btc_amount_msat > 0),
    status           text not null check (status in ('open', 'used', 'expired')),
    expires_at       timestamptz not null,
    created_at       timestamptz not null default now()
);

create table sandbox_payouts (
    id              text primary key,
    project_id      text not null references projects(id),
    quote_id        text not null references sandbox_quotes(id) unique,
    idempotency_key text not null,
    scenario        text not null check (scenario in ('success', 'failure', 'timeout')),
    -- Lightning invoice from the destination node, paid for real on regtest
    -- only after the simulated payout is confirmed.
    settlement_bolt11 text,
    status          text not null check (status in ('reserved', 'pending', 'succeeded', 'failed', 'unknown')),
    payment_id      text references outgoing_payments(id),
    created_at      timestamptz not null default now(),
    updated_at      timestamptz not null default now(),
    unique (project_id, idempotency_key)
);

create table sandbox_balances (
    project_id  text not null references projects(id),
    currency    text not null,
    minor       bigint not null,
    primary key (project_id, currency)
);
