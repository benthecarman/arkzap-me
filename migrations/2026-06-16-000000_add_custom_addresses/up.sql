CREATE TABLE custom_addresses
(
    id          SERIAL PRIMARY KEY,
    name        VARCHAR(32) NOT NULL,
    ark_address TEXT        NOT NULL,
    created_at  TIMESTAMP   NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_custom_addresses_name ON custom_addresses (name);

CREATE TABLE custom_address_invoice
(
    id                  SERIAL PRIMARY KEY,
    name                VARCHAR(32)   NOT NULL,
    ark_address         TEXT          NOT NULL,
    auth_message        TEXT          NOT NULL,
    signature           VARCHAR(128)  NOT NULL,
    fee_receive_address TEXT          NOT NULL,
    bolt11              VARCHAR(2048) NOT NULL,
    amount_msats        BIGINT        NOT NULL,
    payment_hash        VARCHAR(64),
    preimage            VARCHAR(64)   NOT NULL,
    state               INTEGER       NOT NULL DEFAULT 0,
    created_at          TIMESTAMP     NOT NULL DEFAULT NOW(),
    expires_at          TIMESTAMP,
    settled_at          TIMESTAMP
);

CREATE INDEX idx_custom_address_invoice_state ON custom_address_invoice (state);
CREATE INDEX idx_custom_address_invoice_pending_expires_at ON custom_address_invoice (expires_at)
    WHERE state = 0;
CREATE UNIQUE INDEX idx_custom_address_invoice_payment_hash ON custom_address_invoice (payment_hash)
    WHERE payment_hash IS NOT NULL;
CREATE UNIQUE INDEX idx_custom_address_invoice_pending_name ON custom_address_invoice (name)
    WHERE state = 0;
