use crate::models::schema::arkade_invoice;
use chrono::NaiveDateTime;
use diesel::prelude::*;
use lightning_invoice::Bolt11Invoice;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(
    QueryableByName, Queryable, AsChangeset, Serialize, Deserialize, Debug, Clone, PartialEq,
)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(table_name = arkade_invoice)]
pub struct ArkadeInvoice {
    pub id: i32,
    pub recipient_address: String,
    pub bolt11: String,
    pub amount_msats: i64,
    pub payment_hash: Option<String>,
    pub preimage: String,
    pub swap_id: String,
    pub lnurlp_comment: Option<String>,
    pub state: i32,
    pub created_at: NaiveDateTime,
    pub expires_at: Option<NaiveDateTime>,
    pub settled_at: Option<NaiveDateTime>,
}

impl ArkadeInvoice {
    pub fn bolt11(&self) -> Bolt11Invoice {
        Bolt11Invoice::from_str(&self.bolt11).expect("invalid bolt11")
    }

    pub fn get_by_id(
        conn: &mut PgConnection,
        invoice_id: i32,
    ) -> anyhow::Result<Option<ArkadeInvoice>> {
        Ok(arkade_invoice::table
            .filter(arkade_invoice::id.eq(invoice_id))
            .first::<ArkadeInvoice>(conn)
            .optional()?)
    }

    pub fn get_by_payment_hash(
        conn: &mut PgConnection,
        payment_hash: &str,
    ) -> anyhow::Result<Option<ArkadeInvoice>> {
        Ok(arkade_invoice::table
            .filter(arkade_invoice::payment_hash.eq(payment_hash))
            .first::<ArkadeInvoice>(conn)
            .optional()?)
    }

    pub fn get_by_state(conn: &mut PgConnection, state: i32) -> anyhow::Result<Vec<ArkadeInvoice>> {
        Ok(arkade_invoice::table
            .filter(arkade_invoice::state.eq(state))
            .order(arkade_invoice::id.asc())
            .load::<ArkadeInvoice>(conn)?)
    }

    pub fn mark_settled(&self, conn: &mut PgConnection, preimage: String) -> anyhow::Result<bool> {
        let updated = diesel::update(arkade_invoice::table)
            .filter(arkade_invoice::id.eq(self.id))
            .filter(arkade_invoice::state.eq(InvoiceState::Pending as i32))
            .set((
                arkade_invoice::state.eq(InvoiceState::Settled as i32),
                arkade_invoice::preimage.eq(preimage),
                arkade_invoice::settled_at.eq(diesel::dsl::now),
            ))
            .execute(conn)?;

        Ok(updated == 1)
    }

    pub fn mark_cancelled(&self, conn: &mut PgConnection) -> anyhow::Result<bool> {
        let updated = diesel::update(arkade_invoice::table)
            .filter(arkade_invoice::id.eq(self.id))
            .filter(arkade_invoice::state.eq(InvoiceState::Pending as i32))
            .set(arkade_invoice::state.eq(InvoiceState::Cancelled as i32))
            .execute(conn)?;

        Ok(updated == 1)
    }
}

#[derive(Insertable)]
#[diesel(table_name = arkade_invoice)]
pub struct NewArkadeInvoice {
    pub recipient_address: String,
    pub bolt11: String,
    pub amount_msats: i64,
    pub payment_hash: Option<String>,
    pub preimage: String,
    pub swap_id: String,
    pub lnurlp_comment: Option<String>,
    pub state: i32,
    pub expires_at: Option<NaiveDateTime>,
}

impl NewArkadeInvoice {
    pub fn insert(&self, conn: &mut PgConnection) -> anyhow::Result<ArkadeInvoice> {
        diesel::insert_into(arkade_invoice::table)
            .values(self)
            .get_result::<ArkadeInvoice>(conn)
            .map_err(|e| e.into())
    }
}

pub use crate::models::invoice::InvoiceState;
