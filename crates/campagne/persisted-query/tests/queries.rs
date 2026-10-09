//! The three queries through the executor, on seeded rows of a real database.
//! Nothing is mocked: filtering, scoping and order come from the views and the
//! registered SQL.

mod common;

use std::collections::BTreeSet;

use campagne_persisted_query::{QueryError, registry_dir};
use common::{TestDb, campagne, campagne_id, new_id, no_vars, pj, vars};
use serde_json::{Value, json};
use sqlx::AssertSqlSafe;
use uuid::Uuid;

fn ids(answer: &Value, key: &str) -> Vec<String> {
    answer[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect()
}

/// A fixed id whose byte order is its number, to break ties on purpose.
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[tokio::test]
async fn liste_campagnes_newest_first_ties_by_id_and_no_tombstone() {
    let db = TestDb::create().await;
    let exec = db.executor().await;

    assert_eq!(
        exec.run("ListeCampagnes", &no_vars()).await.unwrap(),
        json!({ "campagnes": [] }),
        "an empty database is an empty list"
    );

    let old = campagne(&db.pool, id(9), "Ancienne", "2026-01-01T10:00:00Z", false).await;
    let tie_high = campagne(
        &db.pool,
        id(5),
        "Egalite haute",
        "2026-03-01T10:00:00Z",
        false,
    )
    .await;
    let tie_low = campagne(
        &db.pool,
        id(2),
        "Egalite basse",
        "2026-03-01T10:00:00Z",
        false,
    )
    .await;
    let newest = campagne(&db.pool, id(7), "Recente", "2026-06-01T10:00:00Z", false).await;
    campagne(&db.pool, id(1), "Archivee", "2026-09-01T10:00:00Z", true).await;

    let answer = exec.run("ListeCampagnes", &no_vars()).await.unwrap();
    assert_eq!(
        ids(&answer, "campagnes"),
        [newest, tie_low, tie_high, old].map(|i| i.to_string()),
        "newest first, equal times by id ascending, the tombstoned one absent"
    );
    let first = &answer["campagnes"][0];
    assert_eq!(first["nom"], "Recente");
    assert!(
        first["creeLe"]
            .as_str()
            .unwrap()
            .starts_with("2026-06-01T10:00:00")
    );

    db.drop_db().await;
}

#[tokio::test]
async fn liste_pjs_oldest_first_ties_by_id_and_scoped_to_the_campaign() {
    let db = TestDb::create().await;
    let exec = db.executor().await;
    let a = campagne(&db.pool, new_id(), "Alpha", "2026-01-01T00:00:00Z", false).await;
    let b = campagne(&db.pool, new_id(), "Beta", "2026-01-01T00:00:00Z", false).await;

    let late = pj(
        &db.pool,
        id(1),
        a,
        "Tardif",
        5,
        "2026-02-03T00:00:00Z",
        false,
    )
    .await;
    let tie_high = pj(
        &db.pool,
        id(8),
        a,
        "Egal haut",
        4,
        "2026-02-02T00:00:00Z",
        false,
    )
    .await;
    let tie_low = pj(
        &db.pool,
        id(3),
        a,
        "Egal bas",
        3,
        "2026-02-02T00:00:00Z",
        false,
    )
    .await;
    let first = pj(
        &db.pool,
        id(9),
        a,
        "Premier",
        2,
        "2026-02-01T00:00:00Z",
        false,
    )
    .await;
    pj(
        &db.pool,
        id(2),
        a,
        "Archive",
        8,
        "2026-02-01T12:00:00Z",
        true,
    )
    .await;
    pj(
        &db.pool,
        id(4),
        b,
        "Chez Beta",
        20,
        "2026-02-01T00:00:00Z",
        false,
    )
    .await;

    let answer = exec.run("ListePjs", &campagne_id(a)).await.unwrap();
    assert_eq!(
        ids(&answer, "pjs"),
        [first, tie_low, tie_high, late].map(|i| i.to_string())
    );
    let row = &answer["pjs"][0];
    assert_eq!(
        row.as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        ["classe", "creeLe", "id", "niveau", "nom"]
            .map(String::from)
            .into()
    );
    assert_eq!(
        (row["nom"].as_str(), row["niveau"].as_i64()),
        (Some("Premier"), Some(2))
    );

    let of_b = exec.run("ListePjs", &campagne_id(b)).await.unwrap();
    assert_eq!(ids(&of_b, "pjs"), [id(4).to_string()]);

    db.drop_db().await;
}

#[tokio::test]
async fn niveaux_gives_levels_in_pc_order_and_the_data_version() {
    let db = TestDb::create().await;
    let exec = db.executor().await;
    let empty = campagne(&db.pool, new_id(), "Vide", "2026-01-01T00:00:00Z", false).await;
    let full = campagne(&db.pool, new_id(), "Pleine", "2026-01-02T00:00:00Z", false).await;
    pj(&db.pool, id(2), full, "B", 7, "2026-02-01T00:00:00Z", false).await;
    pj(&db.pool, id(1), full, "A", 3, "2026-02-01T00:00:00Z", false).await;
    pj(&db.pool, id(3), full, "C", 5, "2026-01-15T00:00:00Z", false).await;

    // A campaign with no PC: an empty list and the initial version, not an error.
    assert_eq!(
        exec.run("NiveauxPjsActifs", &campagne_id(empty))
            .await
            .unwrap(),
        json!({ "levels": [], "dataVersion": 0 })
    );

    sqlx::query(AssertSqlSafe(
        "UPDATE dataguard_version SET version = 7".to_owned(),
    ))
    .execute(&db.pool)
    .await
    .unwrap();
    let answer = exec
        .run("NiveauxPjsActifs", &campagne_id(full))
        .await
        .unwrap();
    assert_eq!(answer, json!({ "levels": [5, 3, 7], "dataVersion": 7 }));
    let keys: BTreeSet<_> = answer.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
        ["dataVersion", "levels"].map(String::from).into(),
        "no level, pcCount or model"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn every_unreachable_campaign_id_gets_the_same_not_found() {
    let db = TestDb::create().await;
    let exec = db.executor().await;
    let active = campagne(&db.pool, new_id(), "Active", "2026-01-01T00:00:00Z", false).await;
    let a_pc = pj(
        &db.pool,
        new_id(),
        active,
        "Joueur",
        3,
        "2026-01-02T00:00:00Z",
        false,
    )
    .await;
    let archived = campagne(&db.pool, new_id(), "Archivee", "2026-01-01T00:00:00Z", true).await;
    // A cascade gone wrong: a PC still active under an archived campaign.
    pj(
        &db.pool,
        new_id(),
        archived,
        "Orphelin",
        9,
        "2026-01-02T00:00:00Z",
        false,
    )
    .await;

    let probes: Vec<(&str, Value)> = vec![
        ("an unknown id", json!(new_id().to_string())),
        ("an archived campaign", json!(archived.to_string())),
        ("a PC id", json!(a_pc.to_string())),
        ("not an id", json!("not-a-uuid")),
        ("an empty string", json!("")),
    ];
    for query in ["ListePjs", "NiveauxPjsActifs"] {
        let mut answers = BTreeSet::new();
        for (what, value) in &probes {
            let err = exec
                .run(query, &vars(json!({ "campagneId": value })))
                .await
                .expect_err(what);
            assert!(
                matches!(err, QueryError::NotFound),
                "{query} / {what}: {err:?}"
            );
            answers.insert((format!("{err}"), format!("{err:?}")));
        }
        assert_eq!(answers.len(), 1, "{query}: the answers differ: {answers:?}");
        assert_eq!(
            answers.into_iter().next().unwrap(),
            ("not found".into(), "NotFound".into())
        );
    }

    // The same query on an active campaign with no PC is not a not-found.
    let quiet = campagne(&db.pool, new_id(), "Calme", "2026-01-01T00:00:00Z", false).await;
    assert_eq!(
        exec.run("ListePjs", &campagne_id(quiet)).await.unwrap(),
        json!({ "pjs": [] })
    );

    db.drop_db().await;
}

#[tokio::test]
async fn an_archived_campaign_is_absent_from_the_list() {
    let db = TestDb::create().await;
    let exec = db.executor().await;
    let kept = campagne(&db.pool, new_id(), "Gardee", "2026-01-01T00:00:00Z", false).await;
    campagne(&db.pool, new_id(), "Archivee", "2026-02-01T00:00:00Z", true).await;
    let answer = exec.run("ListeCampagnes", &no_vars()).await.unwrap();
    assert_eq!(ids(&answer, "campagnes"), [kept.to_string()]);
    db.drop_db().await;
}

#[tokio::test]
async fn a_bad_variable_is_invalid_input_never_not_found() {
    let db = TestDb::create().await;
    let exec = db.executor().await;
    let id = campagne(&db.pool, new_id(), "Active", "2026-01-01T00:00:00Z", false).await;

    let bad = [
        json!({}),
        json!({ "campagneId": null }),
        json!({ "campagneId": 42 }),
        json!({ "campagneId": [id.to_string()] }),
        json!({ "campagneId": id.to_string(), "x": 1 }),
        json!({ "autre": id.to_string() }),
    ];
    for query in ["ListePjs", "NiveauxPjsActifs"] {
        for variables in &bad {
            let err = exec.run(query, &vars(variables.clone())).await.unwrap_err();
            assert!(
                matches!(&err, QueryError::InvalidInput { query: q, .. } if q == query),
                "{query} {variables}: {err:?}"
            );
        }
    }
    let err = exec.run("ListePjs", &no_vars()).await.unwrap_err();
    assert!(err.to_string().contains("campagneId"), "{err}");

    // The list takes no variable at all.
    let err = exec
        .run(
            "ListeCampagnes",
            &vars(json!({ "campagneId": id.to_string() })),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, QueryError::InvalidInput { .. }), "{err:?}");

    db.drop_db().await;
}

#[tokio::test]
async fn only_registered_names_run_and_free_text_is_one_error() {
    let db = TestDb::create().await;
    let exec = db.executor().await;
    let mut errors = BTreeSet::new();
    for text in [
        "Nope",
        "SELECT * FROM campagne",
        "query ListePjs { x }",
        "liste-pjs",
        "listepjs",
        "",
    ] {
        let err = exec.run(text, &no_vars()).await.unwrap_err();
        assert!(matches!(err, QueryError::Unknown), "{text:?}: {err:?}");
        errors.insert((format!("{err}"), format!("{err:?}")));
    }
    assert_eq!(errors.len(), 1, "{errors:?}");
    db.drop_db().await;
}

#[tokio::test]
async fn a_database_failure_is_not_a_not_found() {
    let db = TestDb::create().await;
    let exec = db.executor().await;
    let id = campagne(&db.pool, new_id(), "Active", "2026-01-01T00:00:00Z", false).await;
    exec.run("ListePjs", &campagne_id(id)).await.unwrap();

    // The database goes away under the executor (FORCE drops its sessions).
    db.drop_db().await;
    let err = exec.run("ListePjs", &campagne_id(id)).await.unwrap_err();
    assert!(matches!(err, QueryError::Database(_)), "{err:?}");
    assert_eq!(err.to_string(), "database error");
}

// --- rule 6: `fields` is exactly what the SQL reads ------------------------

/// The `alias.column` references of a statement, with the aliases the
/// registered SQL uses: `c` campagne_active, `p` pj_actif, `v` data_version.
fn references(sql: &str) -> BTreeSet<String> {
    let views = [
        ('c', "campagne_active"),
        ('p', "pj_actif"),
        ('v', "data_version"),
    ];
    let chars: Vec<char> = sql.chars().collect();
    let mut found = BTreeSet::new();
    for (i, &ch) in chars.iter().enumerate() {
        let Some((_, view)) = views.iter().find(|(alias, _)| *alias == ch) else {
            continue;
        };
        let before_ok =
            i == 0 || !(chars[i - 1].is_alphanumeric() || "_\"'.".contains(chars[i - 1]));
        if !before_ok || chars.get(i + 1) != Some(&'.') {
            continue;
        }
        let rest: String = chars[i + 2..].iter().collect();
        let column: String = if let Some(quoted) = rest.strip_prefix('"') {
            quoted.chars().take_while(|c| *c != '"').collect()
        } else {
            rest.chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect()
        };
        found.insert(format!("{view}.{column}"));
    }
    found
}

#[tokio::test]
async fn fields_lists_every_view_column_the_sql_reads_and_nothing_else() {
    let db = TestDb::create().await;
    let registry: Value = serde_json::from_str(
        &std::fs::read_to_string(registry_dir().join("registry.json")).unwrap(),
    )
    .unwrap();

    let columns: BTreeSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT table_name || '.' || column_name FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name IN ('campagne_active', 'pj_actif', 'data_version')",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap()
    .into_iter()
    .collect();
    assert!(columns.contains("pj_actif.niveau") && columns.contains("data_version.dataVersion"));

    for entry in registry["queries"].as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let sql =
            std::fs::read_to_string(registry_dir().join(file.replace(".graphql", ".sql"))).unwrap();

        // Only the three views are read.
        for word in sql.split_whitespace().collect::<Vec<_>>().windows(2) {
            if word[0].eq_ignore_ascii_case("FROM") {
                assert!(
                    ["campagne_active", "pj_actif", "data_version"].contains(&word[1]),
                    "{file} reads {}",
                    word[1]
                );
            }
        }

        let read = references(&sql);
        let declared: BTreeSet<String> = entry["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            read, declared,
            "{file}: `fields` must list exactly what the SQL reads"
        );
        for field in &declared {
            assert!(
                columns.contains(field),
                "{file}: {field} is no column of a read view"
            );
        }
    }
    db.drop_db().await;
}
