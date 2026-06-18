UPDATE custom_address_invoice
SET expires_at = LEAST(
    COALESCE(expires_at, created_at + INTERVAL '1 hour'),
    created_at + INTERVAL '1 hour'
);

ALTER TABLE custom_address_invoice
    ALTER COLUMN expires_at SET NOT NULL;
