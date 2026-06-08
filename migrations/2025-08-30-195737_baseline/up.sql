CREATE TABLE invoice
(
    id                 SERIAL PRIMARY KEY,
    ark_address        TEXT          NOT NULL,
    bolt11             VARCHAR(2048) NOT NULL,
    amount_msats       BIGINT        NOT NULL,
    payment_hash       VARCHAR(64),
    preimage           VARCHAR(64)   NOT NULL,
    lnurlp_comment     VARCHAR(100),
    state              INTEGER       NOT NULL DEFAULT 0,
    created_at         TIMESTAMP     NOT NULL DEFAULT NOW(),
    expires_at         TIMESTAMP,
    settled_at         TIMESTAMP
);

CREATE INDEX idx_invoice_state ON invoice (state);
CREATE INDEX idx_invoice_pending_expires_at ON invoice (expires_at)
    WHERE state = 0;
CREATE UNIQUE INDEX idx_invoice_payment_hash ON invoice (payment_hash)
    WHERE payment_hash IS NOT NULL;

CREATE TABLE zaps
(
    id       INTEGER NOT NULL PRIMARY KEY references invoice (id),
    request  TEXT    NOT NULL,
    event_id VARCHAR(64)
);

CREATE INDEX idx_zaps_event_id ON zaps (event_id);
