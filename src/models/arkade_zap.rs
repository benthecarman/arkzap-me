use crate::models::arkade_invoice::ArkadeInvoice;
use crate::models::invoice::InvoiceState;
use crate::models::schema::{arkade_invoice, arkade_zaps};
use diesel::dsl::{now, IntervalDsl};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(
    QueryableByName,
    Queryable,
    Insertable,
    AsChangeset,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    PartialEq,
)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(table_name = arkade_zaps)]
pub struct ArkadeZap {
    pub id: i32,
    pub request: String,
    pub event_id: Option<String>,
}

impl ArkadeZap {
    pub fn insert(&self, conn: &mut PgConnection) -> anyhow::Result<ArkadeZap> {
        let res = diesel::insert_into(arkade_zaps::table)
            .values(self)
            .get_result(conn)?;

        Ok(res)
    }

    pub fn set_event_id(&self, conn: &mut PgConnection, event_id: String) -> anyhow::Result<()> {
        diesel::update(arkade_zaps::table)
            .filter(arkade_zaps::id.eq(self.id))
            .set(arkade_zaps::event_id.eq(event_id))
            .execute(conn)?;

        Ok(())
    }

    /// Zaps settled within the last `window_hours` that have no published receipt yet.
    pub fn get_unpublished_settled(
        conn: &mut PgConnection,
        window_hours: i32,
    ) -> anyhow::Result<Vec<(ArkadeZap, ArkadeInvoice)>> {
        Ok(arkade_zaps::table
            .inner_join(arkade_invoice::table)
            .filter(arkade_invoice::state.eq(InvoiceState::Settled as i32))
            .filter(arkade_invoice::settled_at.gt((now - window_hours.hours()).nullable()))
            .filter(arkade_zaps::event_id.is_null())
            .order(arkade_invoice::settled_at.asc())
            .load::<(ArkadeZap, ArkadeInvoice)>(conn)?)
    }
}
