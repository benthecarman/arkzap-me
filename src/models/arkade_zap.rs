use crate::models::schema::arkade_zaps;
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
}
