CREATE TABLE arkade_invoice
(
    id                 SERIAL PRIMARY KEY,
    recipient_address  TEXT          NOT NULL,
    bolt11             VARCHAR(2048) NOT NULL,
    amount_msats       BIGINT        NOT NULL,
    payment_hash       VARCHAR(64),
    preimage           VARCHAR(64)   NOT NULL,
    swap_id            TEXT          NOT NULL,
    lnurlp_comment     VARCHAR(100),
    state              INTEGER       NOT NULL DEFAULT 0,
    created_at         TIMESTAMP     NOT NULL DEFAULT NOW(),
    expires_at         TIMESTAMP,
    settled_at         TIMESTAMP
);

CREATE INDEX idx_arkade_invoice_state ON arkade_invoice (state);
CREATE INDEX idx_arkade_invoice_pending_expires_at ON arkade_invoice (expires_at)
    WHERE state = 0;
CREATE UNIQUE INDEX idx_arkade_invoice_payment_hash ON arkade_invoice (payment_hash)
    WHERE payment_hash IS NOT NULL;
CREATE UNIQUE INDEX idx_arkade_invoice_swap_id ON arkade_invoice (swap_id);

CREATE TABLE arkade_zaps
(
    id       INTEGER NOT NULL PRIMARY KEY references arkade_invoice (id),
    request  TEXT    NOT NULL,
    event_id VARCHAR(64)
);

CREATE INDEX idx_arkade_zaps_event_id ON arkade_zaps (event_id);

CREATE TABLE arkade_swap_storage
(
    swap_id    TEXT      NOT NULL,
    swap_type  TEXT      NOT NULL,
    data       JSONB     NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY (swap_type, swap_id)
);

CREATE INDEX idx_arkade_swap_storage_swap_id ON arkade_swap_storage (swap_id);
