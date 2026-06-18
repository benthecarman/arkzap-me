CREATE INDEX idx_invoice_pending_id ON invoice (id)
    WHERE state = 0;

CREATE INDEX idx_arkade_invoice_pending_id ON arkade_invoice (id)
    WHERE state = 0;

CREATE INDEX idx_custom_address_invoice_pending_id ON custom_address_invoice (id)
    WHERE state = 0;
