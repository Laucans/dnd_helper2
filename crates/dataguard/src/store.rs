//! The rows of Campagne and PJ, in their aggregates' field names. The tables
//! predate the manifests and name their columns in French; this module is the
//! one place that maps one to the other. It inserts, updates and sets
//! tombstones; it never removes a row.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::{AssertSqlSafe, PgConnection, Row};
use uuid::Uuid;

use crate::invariants::{CAMPAGNE, CampaignRow, PJ, PcRow, Snapshot, campaign_of};
use crate::model::{OpKind, Operation};
use crate::text;

/// Aggregate → table.
const TABLES: &[(&str, &str)] = &[(CAMPAGNE, "campagne"), (PJ, "pj")];

/// (aggregate, field, column). Every field of every aggregate.json is here.
pub const COLUMNS: &[(&str, &str, &str)] = &[
    (CAMPAGNE, "name", "nom"),
    (CAMPAGNE, "archivedAt", "\"archiveLe\""),
    (PJ, "campagneId", "\"campagneId\""),
    (PJ, "name", "nom"),
    (PJ, "class", "classe"),
    (PJ, "level", "niveau"),
    (PJ, "archivedAt", "\"archiveLe\""),
];

pub fn table(aggregate: &str) -> Option<&'static str> {
    TABLES
        .iter()
        .find(|(a, _)| *a == aggregate)
        .map(|(_, t)| *t)
}

pub fn column(aggregate: &str, field: &str) -> Option<&'static str> {
    COLUMNS
        .iter()
        .find(|(a, f, _)| *a == aggregate && *f == field)
        .map(|(_, _, c)| *c)
}

fn unknown(what: &str) -> sqlx::Error {
    sqlx::Error::Protocol(format!("no table or column for {what}"))
}

/// One campaign and all its PCs, archived ones included. `for_update` locks
/// them for the applying transaction.
pub async fn load_scope(
    conn: &mut PgConnection,
    campaign: Uuid,
    for_update: bool,
) -> Result<Snapshot, sqlx::Error> {
    let (c_sql, p_sql) = if for_update {
        (
            "SELECT id, nom, \"archiveLe\" FROM campagne WHERE id = $1 FOR UPDATE",
            "SELECT id, \"campagneId\", nom, classe, niveau, \"archiveLe\" FROM pj
             WHERE \"campagneId\" = $1 ORDER BY id FOR UPDATE",
        )
    } else {
        (
            "SELECT id, nom, \"archiveLe\" FROM campagne WHERE id = $1",
            "SELECT id, \"campagneId\", nom, classe, niveau, \"archiveLe\" FROM pj
             WHERE \"campagneId\" = $1",
        )
    };
    let mut s = Snapshot::default();
    if let Some(row) = sqlx::query(c_sql)
        .bind(campaign)
        .fetch_optional(&mut *conn)
        .await?
    {
        s.campaigns.insert(
            campaign,
            CampaignRow {
                id: row.try_get("id")?,
                name: row.try_get("nom")?,
                archived_at: row.try_get("archiveLe")?,
            },
        );
    }
    for row in sqlx::query(p_sql)
        .bind(campaign)
        .fetch_all(&mut *conn)
        .await?
    {
        let id: Uuid = row.try_get("id")?;
        s.pcs.insert(
            id,
            PcRow {
                id,
                campaign: row.try_get("campagneId")?,
                name: row.try_get("nom")?,
                class: row.try_get("classe")?,
                level: i64::from(row.try_get::<i32, _>("niveau")?),
                archived_at: row.try_get("archiveLe")?,
            },
        );
    }
    Ok(s)
}

/// The campaign of a stored PC.
pub async fn campaign_of_pc(
    conn: &mut PgConnection,
    pc: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT \"campagneId\" FROM pj WHERE id = $1")
        .bind(pc)
        .fetch_optional(conn)
        .await
}

/// The campaign whose rows the invariants of `op` read, as stored now.
pub async fn scope_campaign(
    conn: &mut PgConnection,
    op: &Operation,
) -> Result<Option<Uuid>, sqlx::Error> {
    Ok(match (op.aggregate.as_str(), op.kind) {
        (CAMPAGNE, _) => Some(op.id),
        (PJ, OpKind::Insert) => campaign_of(&op.fields),
        (PJ, _) => campaign_of_pc(conn, op.id).await?,
        _ => None,
    })
}

enum Bind {
    Text(String),
    Int(i32),
}

/// The bound value of a validated field: text in its stored form, a level as
/// an integer.
fn bind_of(field: &str, value: &Value) -> Result<Bind, sqlx::Error> {
    match field {
        "level" => value
            .as_i64()
            .and_then(|l| i32::try_from(l).ok())
            .map(Bind::Int)
            .ok_or_else(|| unknown("a level that is not an integer")),
        _ => value
            .as_str()
            .map(|s| Bind::Text(text::normalize(s)))
            .ok_or_else(|| unknown(field)),
    }
}

/// Inserts the row of an insert, with the id minted at submission.
pub async fn insert(conn: &mut PgConnection, op: &Operation) -> Result<(), sqlx::Error> {
    let fields: Vec<(&str, Bind)> = match op.aggregate.as_str() {
        CAMPAGNE => vec![("name", bind_of("name", &op.fields["name"])?)],
        PJ => vec![
            ("name", bind_of("name", &op.fields["name"])?),
            ("class", bind_of("class", &op.fields["class"])?),
            ("level", bind_of("level", &op.fields["level"])?),
        ],
        other => return Err(unknown(other)),
    };
    let t = table(&op.aggregate).ok_or_else(|| unknown(&op.aggregate))?;
    let mut columns = vec!["id"];
    let mut params = vec!["$1".to_owned()];
    let relation = (op.aggregate == PJ)
        .then(|| campaign_of(&op.fields))
        .flatten();
    if relation.is_some() {
        columns.push("\"campagneId\"");
        params.push("$2".to_owned());
    }
    for (field, _) in &fields {
        columns.push(column(&op.aggregate, field).ok_or_else(|| unknown(field))?);
        params.push(format!("${}", params.len() + 1));
    }
    let sql = format!(
        "INSERT INTO {t} ({}) VALUES ({})",
        columns.join(", "),
        params.join(", ")
    );
    let mut q = sqlx::query(AssertSqlSafe(sql)).bind(op.id);
    if let Some(c) = relation {
        q = q.bind(c);
    }
    for (_, b) in fields {
        q = match b {
            Bind::Text(s) => q.bind(s),
            Bind::Int(i) => q.bind(i),
        };
    }
    q.execute(conn).await?;
    Ok(())
}

/// Sets the fields an update carries; an absent field is unchanged.
pub async fn update(conn: &mut PgConnection, op: &Operation) -> Result<u64, sqlx::Error> {
    if op.fields.is_empty() {
        return Ok(0);
    }
    let t = table(&op.aggregate).ok_or_else(|| unknown(&op.aggregate))?;
    let mut sets = Vec::new();
    let mut binds = Vec::new();
    for (field, value) in &op.fields {
        let c = column(&op.aggregate, field).ok_or_else(|| unknown(field))?;
        sets.push(format!("{c} = ${}", sets.len() + 2));
        binds.push(bind_of(field, value)?);
    }
    let sql = format!("UPDATE {t} SET {} WHERE id = $1", sets.join(", "));
    let mut q = sqlx::query(AssertSqlSafe(sql)).bind(op.id);
    for b in binds {
        q = match b {
            Bind::Text(s) => q.bind(s),
            Bind::Int(i) => q.bind(i),
        };
    }
    Ok(q.execute(conn).await?.rows_affected())
}

/// Sets the tombstone of the rows of `ids` that have none; a tombstone is
/// never rewritten.
pub async fn set_archived(
    conn: &mut PgConnection,
    aggregate: &str,
    ids: &[Uuid],
    at: DateTime<Utc>,
) -> Result<u64, sqlx::Error> {
    let t = table(aggregate).ok_or_else(|| unknown(aggregate))?;
    let sql =
        format!("UPDATE {t} SET \"archiveLe\" = $1 WHERE id = ANY($2) AND \"archiveLe\" IS NULL");
    Ok(sqlx::query(AssertSqlSafe(sql))
        .bind(at)
        .bind(ids)
        .execute(conn)
        .await?
        .rows_affected())
}

/// Active rows of `aggregate` whose `field` is `value`, locked: the targets
/// of a cascade along that relation.
pub async fn active_ids_where(
    conn: &mut PgConnection,
    aggregate: &str,
    field: &str,
    value: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let t = table(aggregate).ok_or_else(|| unknown(aggregate))?;
    let c = column(aggregate, field).ok_or_else(|| unknown(field))?;
    let sql = format!(
        "SELECT id FROM {t} WHERE {c} = $1 AND \"archiveLe\" IS NULL ORDER BY id FOR UPDATE"
    );
    sqlx::query_scalar(AssertSqlSafe(sql))
        .bind(value)
        .fetch_all(conn)
        .await
}

/// The committed values of `fields` on one row, in field names; empty when
/// the row does not exist.
pub async fn committed_values(
    conn: &mut PgConnection,
    aggregate: &str,
    id: Uuid,
    fields: &[&str],
) -> Result<Map<String, Value>, sqlx::Error> {
    let t = table(aggregate).ok_or_else(|| unknown(aggregate))?;
    let pairs: Vec<String> = COLUMNS
        .iter()
        .filter(|(a, f, _)| *a == aggregate && fields.contains(f))
        .map(|(_, f, c)| format!("'{f}', {c}"))
        .collect();
    if pairs.is_empty() {
        return Ok(Map::new());
    }
    let sql = format!(
        "SELECT jsonb_build_object({}) FROM {t} WHERE id = $1",
        pairs.join(", ")
    );
    let row: Option<sqlx::types::Json<Map<String, Value>>> = sqlx::query_scalar(AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(conn)
        .await?;
    Ok(row.map(|j| j.0).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Aggregates;

    #[test]
    fn every_field_of_every_aggregate_has_its_column() {
        let aggregates = Aggregates::embedded().unwrap();
        for a in aggregates.iter() {
            assert!(table(&a.aggregate).is_some(), "{}", a.aggregate);
            for field in a.fields.keys() {
                assert!(
                    column(&a.aggregate, field).is_some(),
                    "{}.{field}",
                    a.aggregate
                );
            }
        }
        assert_eq!(COLUMNS.len(), 7);
    }

    /// No engine path removes a row: an archive is a tombstone.
    #[test]
    fn no_engine_source_issues_a_delete() {
        let delete = ["DELETE", " FROM"].concat();
        let wipe = ["TRUN", "CATE"].concat();
        for (name, source) in [
            ("store.rs", include_str!("store.rs")),
            ("applier.rs", include_str!("applier.rs")),
            ("resolver.rs", include_str!("resolver.rs")),
            ("queue.rs", include_str!("queue.rs")),
            ("guard.rs", include_str!("guard.rs")),
            ("engine.rs", include_str!("engine.rs")),
        ] {
            let upper = source.to_uppercase();
            assert!(!upper.contains(&delete), "{name}");
            assert!(!upper.contains(&wipe), "{name}");
        }
    }
}
