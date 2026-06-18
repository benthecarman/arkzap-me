use crate::models::invoice::InvoiceState;
use crate::models::schema::{custom_address_invoice, custom_addresses};
use chrono::NaiveDateTime;
use diesel::prelude::*;
use lightning_invoice::Bolt11Invoice;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Queryable, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(table_name = custom_addresses)]
pub struct CustomAddress {
    pub id: i32,
    pub name: String,
    pub ark_address: String,
    pub created_at: NaiveDateTime,
}

impl CustomAddress {
    pub fn get_by_name(
        conn: &mut PgConnection,
        address_name: &str,
    ) -> anyhow::Result<Option<Self>> {
        Ok(custom_addresses::table
            .filter(custom_addresses::name.eq(address_name))
            .first::<Self>(conn)
            .optional()?)
    }

    pub fn name_exists(conn: &mut PgConnection, address_name: &str) -> anyhow::Result<bool> {
        Ok(Self::get_by_name(conn, address_name)?.is_some())
    }
}

#[derive(Insertable)]
#[diesel(table_name = custom_addresses)]
pub struct NewCustomAddress {
    pub name: String,
    pub ark_address: String,
}

#[derive(
    QueryableByName, Queryable, AsChangeset, Serialize, Deserialize, Debug, Clone, PartialEq,
)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(table_name = custom_address_invoice)]
pub struct CustomAddressInvoice {
    pub id: i32,
    pub name: String,
    pub ark_address: String,
    pub auth_message: String,
    pub signature: String,
    pub fee_receive_address: String,
    pub bolt11: String,
    pub amount_msats: i64,
    pub payment_hash: Option<String>,
    pub preimage: String,
    pub ark_payment_reference: Option<String>,
    pub state: i32,
    pub created_at: NaiveDateTime,
    pub expires_at: Option<NaiveDateTime>,
    pub settled_at: Option<NaiveDateTime>,
}

impl CustomAddressInvoice {
    pub fn bolt11(&self) -> Bolt11Invoice {
        Bolt11Invoice::from_str(&self.bolt11).expect("invalid bolt11")
    }

    pub fn get_by_id(conn: &mut PgConnection, invoice_id: i32) -> anyhow::Result<Option<Self>> {
        Ok(custom_address_invoice::table
            .filter(custom_address_invoice::id.eq(invoice_id))
            .first::<Self>(conn)
            .optional()?)
    }

    pub fn get_by_payment_hash(
        conn: &mut PgConnection,
        payment_hash: &str,
    ) -> anyhow::Result<Option<Self>> {
        Ok(custom_address_invoice::table
            .filter(custom_address_invoice::payment_hash.eq(payment_hash))
            .first::<Self>(conn)
            .optional()?)
    }

    pub fn get_by_state(conn: &mut PgConnection, state: i32) -> anyhow::Result<Vec<Self>> {
        Ok(custom_address_invoice::table
            .filter(custom_address_invoice::state.eq(state))
            .order(custom_address_invoice::id.asc())
            .load::<Self>(conn)?)
    }

    pub fn pending_name_exists(
        conn: &mut PgConnection,
        address_name: &str,
    ) -> anyhow::Result<bool> {
        Ok(custom_address_invoice::table
            .filter(custom_address_invoice::name.eq(address_name))
            .filter(custom_address_invoice::state.eq(InvoiceState::Pending as i32))
            .first::<Self>(conn)
            .optional()?
            .is_some())
    }

    pub fn mark_lightning_settled_and_activate(
        &self,
        conn: &mut PgConnection,
        preimage: String,
    ) -> anyhow::Result<bool> {
        self.mark_settled_and_activate(conn, Some(preimage), None)
    }

    pub fn mark_ark_settled_and_activate(
        &self,
        conn: &mut PgConnection,
        ark_payment_reference: String,
    ) -> anyhow::Result<bool> {
        self.mark_settled_and_activate(conn, None, Some(ark_payment_reference))
    }

    fn mark_settled_and_activate(
        &self,
        conn: &mut PgConnection,
        preimage: Option<String>,
        ark_payment_reference: Option<String>,
    ) -> anyhow::Result<bool> {
        conn.transaction::<_, anyhow::Error, _>(|conn| {
            if CustomAddress::name_exists(conn, &self.name)? {
                self.mark_cancelled(conn)?;
                return Ok(false);
            }

            let updated = diesel::update(custom_address_invoice::table)
                .filter(custom_address_invoice::id.eq(self.id))
                .filter(custom_address_invoice::state.eq(InvoiceState::Pending as i32))
                .set((
                    custom_address_invoice::state.eq(InvoiceState::Settled as i32),
                    custom_address_invoice::preimage.eq(preimage.unwrap_or_default()),
                    custom_address_invoice::ark_payment_reference.eq(ark_payment_reference),
                    custom_address_invoice::settled_at.eq(diesel::dsl::now),
                ))
                .execute(conn)?;

            if updated == 1 {
                diesel::insert_into(custom_addresses::table)
                    .values(NewCustomAddress {
                        name: self.name.clone(),
                        ark_address: self.ark_address.clone(),
                    })
                    .execute(conn)?;
                Ok(true)
            } else {
                Ok(false)
            }
        })
    }

    pub fn mark_cancelled(&self, conn: &mut PgConnection) -> anyhow::Result<bool> {
        let updated = diesel::update(custom_address_invoice::table)
            .filter(custom_address_invoice::id.eq(self.id))
            .filter(custom_address_invoice::state.eq(InvoiceState::Pending as i32))
            .set(custom_address_invoice::state.eq(InvoiceState::Cancelled as i32))
            .execute(conn)?;

        Ok(updated == 1)
    }
}

#[derive(Insertable)]
#[diesel(table_name = custom_address_invoice)]
pub struct NewCustomAddressInvoice {
    pub name: String,
    pub ark_address: String,
    pub auth_message: String,
    pub signature: String,
    pub fee_receive_address: String,
    pub bolt11: String,
    pub amount_msats: i64,
    pub payment_hash: Option<String>,
    pub preimage: String,
    pub ark_payment_reference: Option<String>,
    pub state: i32,
    pub expires_at: Option<NaiveDateTime>,
}

impl NewCustomAddressInvoice {
    pub fn insert(&self, conn: &mut PgConnection) -> anyhow::Result<CustomAddressInvoice> {
        diesel::insert_into(custom_address_invoice::table)
            .values(self)
            .get_result::<CustomAddressInvoice>(conn)
            .map_err(|e| e.into())
    }
}
