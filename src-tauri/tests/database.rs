use hey_db_lib::{
    db::Database,
    model::{CellEdit, Profile},
};
use tokio_postgres::NoTls;

async fn setup() -> (Database, Profile, tokio_postgres::Client) {
    let port = std::env::var("HEY_DB_TEST_PORT")
        .expect("Run npm run test:database")
        .parse()
        .unwrap();
    let password = std::env::var("HEY_DB_TEST_PASSWORD").unwrap();
    let profile = Profile {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Ephemeral integration test".into(),
        host: "127.0.0.1".into(),
        port,
        database: "heydb_test".into(),
        username: "heydb_test".into(),
        tls: "disable".into(),
        read_only: false,
        remember_password: false,
    };
    let mut config = tokio_postgres::Config::new();
    config
        .host("127.0.0.1")
        .port(port)
        .user("heydb_test")
        .password(&password)
        .dbname("heydb_test");
    let (client, connection) = config.connect(NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client.batch_execute("DROP SCHEMA IF EXISTS fixture CASCADE; CREATE SCHEMA fixture; CREATE TABLE fixture.products (id bigint PRIMARY KEY, name text, status text NOT NULL DEFAULT 'active', stock integer NOT NULL CHECK (stock>=0), doubled integer GENERATED ALWAYS AS (stock*2) STORED); INSERT INTO fixture.products(id,name,stock) VALUES (1001,'Desk lamp',42),(1002,'Notebook',128),(1003,'Monitor stand',16); CREATE TABLE fixture.composite (tenant text, id integer, label text, PRIMARY KEY(tenant,id)); INSERT INTO fixture.composite VALUES ('one',1,'before'),('two',1,'other'); CREATE TABLE fixture.keyless (name text); INSERT INTO fixture.keyless VALUES ('sample'); CREATE VIEW fixture.product_view AS SELECT * FROM fixture.products;").await.unwrap();
    let db = Database::default();
    db.connect(profile.clone(), password).await.unwrap();
    (db, profile, client)
}
const QUERY: &str =
    "SELECT id,name,status,stock FROM fixture.products WHERE status='active' ORDER BY id LIMIT 100";

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; use npm run test:database"]
async fn real_postgresql_workflow() {
    let (db, p, external) = setup().await;
    let result = db.query(&p.id, QUERY, "select").await.unwrap();
    assert_eq!(result.rows[2][1].as_deref(), Some("Monitor stand"));
    assert!(result.columns[1].editable);
    assert!(result.columns[0].primary_key);
    assert!(!result.columns[0].editable);
    let change = CellEdit {
        row: 2,
        column: 1,
        value: Some("Monitor Holder".into()),
    };
    let plan = db
        .preview(&p.id, &result.id, std::slice::from_ref(&change))
        .await
        .unwrap();
    assert_eq!(plan[0].parameters[1].as_deref(), Some("1003"));
    assert_eq!(db.apply(&p.id, &result.id, &[change]).await.unwrap(), 1);
    assert_eq!(
        external
            .query_one("SELECT name FROM fixture.products WHERE id=1003", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "Monitor Holder"
    );
    assert!(
        db.apply(
            &p.id,
            &result.id,
            &[CellEdit {
                row: 2,
                column: 1,
                value: None
            }]
        )
        .await
        .is_err(),
        "a committed result cannot be replayed"
    );

    // A second writer changes one edited cell. Even an earlier update in this
    // same batch must roll back rather than silently overwriting either row.
    let result = db.query(&p.id, QUERY, "conflict").await.unwrap();
    external
        .execute(
            "UPDATE fixture.products SET name='Concurrent edit' WHERE id=1003",
            &[],
        )
        .await
        .unwrap();
    let changes = [
        CellEdit {
            row: 0,
            column: 1,
            value: Some("Should roll back".into()),
        },
        CellEdit {
            row: 2,
            column: 1,
            value: Some("Stale overwrite".into()),
        },
    ];
    assert!(db
        .apply(&p.id, &result.id, &changes)
        .await
        .unwrap_err()
        .contains("rolled back"));
    assert_eq!(
        external
            .query_one("SELECT name FROM fixture.products WHERE id=1001", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "Desk lamp"
    );
    assert_eq!(
        external
            .query_one("SELECT name FROM fixture.products WHERE id=1003", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "Concurrent edit"
    );

    // Parameter values must remain data, including quotes and SQL-looking text.
    let result = db.query(&p.id, QUERY, "quote").await.unwrap();
    let malicious = "Holder'; DROP TABLE fixture.products; --";
    db.apply(
        &p.id,
        &result.id,
        &[CellEdit {
            row: 2,
            column: 1,
            value: Some(malicious.into()),
        }],
    )
    .await
    .unwrap();
    assert_eq!(
        external
            .query_one("SELECT name FROM fixture.products WHERE id=1003", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        malicious
    );

    // Explicit NULL and empty string are separate values.
    for value in [None, Some(String::new())] {
        let result = db.query(&p.id, QUERY, "null").await.unwrap();
        db.apply(
            &p.id,
            &result.id,
            &[CellEdit {
                row: 0,
                column: 1,
                value: value.clone(),
            }],
        )
        .await
        .unwrap();
        assert_eq!(
            external
                .query_one("SELECT name FROM fixture.products WHERE id=1001", &[])
                .await
                .unwrap()
                .get::<_, Option<String>>(0),
            value
        );
    }
    let result = db.query(&p.id, QUERY, "constraint").await.unwrap();
    assert!(db
        .apply(
            &p.id,
            &result.id,
            &[
                CellEdit {
                    row: 0,
                    column: 1,
                    value: Some("rollback".into())
                },
                CellEdit {
                    row: 2,
                    column: 3,
                    value: Some("-1".into())
                }
            ]
        )
        .await
        .is_err());
    assert_eq!(
        external
            .query_one("SELECT name FROM fixture.products WHERE id=1001", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        ""
    );

    let composite = db
        .query(
            &p.id,
            "SELECT tenant,id,label FROM fixture.composite ORDER BY tenant",
            "composite",
        )
        .await
        .unwrap();
    db.apply(
        &p.id,
        &composite.id,
        &[CellEdit {
            row: 0,
            column: 2,
            value: Some("updated".into()),
        }],
    )
    .await
    .unwrap();
    assert_eq!(
        external
            .query_one(
                "SELECT label FROM fixture.composite WHERE tenant='two'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, String>(0),
        "other"
    );
    for sql in [
        "SELECT name FROM fixture.products",
        "SELECT * FROM fixture.keyless",
        "SELECT * FROM fixture.product_view",
        "SELECT p.id,p.name FROM fixture.products p JOIN fixture.composite c ON true",
        "SELECT DISTINCT id,name FROM fixture.products",
        "SELECT id,name||'suffix' FROM fixture.products",
        "WITH p AS (SELECT * FROM fixture.products) SELECT * FROM p",
    ] {
        let result = db.query(&p.id, sql, "readonly").await.unwrap();
        assert!(result.read_only_reason.is_some(), "{sql}");
        assert!(result.columns.iter().all(|c| !c.editable));
    }
    let generated = db
        .query(
            &p.id,
            "SELECT * FROM fixture.products ORDER BY id",
            "generated",
        )
        .await
        .unwrap();
    assert!(!generated.columns[4].editable);

    // Preserve values beyond JavaScript's safe-integer range.
    external
        .execute(
            "INSERT INTO fixture.products(id,name,stock) VALUES (9007199254740993,'Large key',2)",
            &[],
        )
        .await
        .unwrap();
    let result = db
        .query(
            &p.id,
            "SELECT id,name FROM fixture.products WHERE id=9007199254740993",
            "bigint",
        )
        .await
        .unwrap();
    assert_eq!(result.rows[0][0].as_deref(), Some("9007199254740993"));
    db.apply(
        &p.id,
        &result.id,
        &[CellEdit {
            row: 0,
            column: 1,
            value: Some("Exact key".into()),
        }],
    )
    .await
    .unwrap();

    // Invalid inputs and a server-side timeout/cancel must leave the session usable.
    assert!(db
        .query(&p.id, "SELECT 1; DELETE FROM fixture.products", "multi")
        .await
        .is_err());
    assert!(db
        .query(&p.id, "SELECT no_such_column FROM fixture.products", "bad")
        .await
        .is_err());
    assert_eq!(
        db.query(&p.id, "SELECT 1 AS alive", "after-error")
            .await
            .unwrap()
            .rows[0][0]
            .as_deref(),
        Some("1")
    );
    let (cancelled, cancel_result) =
        tokio::join!(db.query(&p.id, "SELECT pg_sleep(15)", "slow"), async {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            db.cancel(&p.id, "slow").await
        });
    cancel_result.unwrap();
    assert!(cancelled.unwrap_err().contains("canceled"));
    assert!(db.query(&p.id, "SELECT 1", "after-cancel").await.is_ok());

    let limited = db
        .query(&p.id, "SELECT generate_series(1,10000) AS n", "cap")
        .await
        .unwrap();
    assert_eq!(limited.rows.len(), 1000);
    assert!(limited.truncated);
    assert!(db.query(&p.id, "SELECT 1", "after-cap").await.is_ok());

    // Read-only mode is enforced by a PostgreSQL transaction, not only by UI.
    let mut readonly = p.clone();
    readonly.id = uuid::Uuid::new_v4().to_string();
    readonly.read_only = true;
    db.connect(
        readonly.clone(),
        std::env::var("HEY_DB_TEST_PASSWORD").unwrap(),
    )
    .await
    .unwrap();
    assert!(db
        .query(
            &readonly.id,
            "UPDATE fixture.products SET stock=0",
            "write-denied"
        )
        .await
        .is_err());
    assert!(db.query(&readonly.id, QUERY, "read-allowed").await.is_ok());
    db.disconnect(&readonly.id).await.unwrap();
    db.disconnect(&p.id).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; use npm run test:database"]
async fn scripts_commit_in_order_and_stop_on_failure() {
    let (db, p, external) = setup().await;
    let result = db
        .script(
            &p.id,
            r#"
        -- Separate commits, plus semicolons that are part of values.
        CREATE TABLE fixture.transactions (id bigint, label text);
        INSERT INTO fixture.transactions VALUES (txid_current(), 'one; it''s fine');
        INSERT INTO fixture.transactions VALUES (txid_current(), $tag$two; →$tag$);
        ALTER TABLE fixture.transactions ADD COLUMN IF NOT EXISTS fulfillment_error varchar;
        ALTER TABLE fixture.transactions DROP COLUMN IF EXISTS fulfillment_error;
        SELECT id, label FROM fixture.transactions ORDER BY id;
    "#,
            "script",
        )
        .await
        .unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.total_statements, 6);
    assert_eq!(result.statements.len(), 6);
    assert!(result.statements.iter().all(|s| s.committed));
    let rows = &result.result.as_ref().unwrap().rows;
    assert_ne!(
        rows[0][0], rows[1][0],
        "each INSERT uses a different transaction"
    );
    assert_eq!(rows[0][1].as_deref(), Some("one; it's fine"));
    assert_eq!(rows[1][1].as_deref(), Some("two; →"));
    assert_eq!(
        result.refresh_sql.as_deref(),
        Some("SELECT id, label FROM fixture.transactions ORDER BY id")
    );
    let tables = db.tables(&p.id).await.unwrap();
    let table = tables.iter().find(|t| t.name == "transactions").unwrap();
    assert_eq!(table.columns, ["id", "label"]);
    assert!(!table.visible); // fixture is outside the default search_path

    let result = db.script(&p.id, "UPDATE fixture.products SET stock=stock+1 WHERE id=1001; UPDATE fixture.products SET stock=-1 WHERE id=1002; UPDATE fixture.products SET stock=0 WHERE id=1003;", "fail").await.unwrap();
    assert_eq!(result.statements.len(), 1);
    assert!(result.error.unwrap().contains("Statement 2 of 3 failed"));
    assert!(result.result.is_none());
    let stocks: Vec<i32> = external
        .query("SELECT stock FROM fixture.products ORDER BY id", &[])
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(stocks, [43, 128, 16]);
    assert!(db
        .query(&p.id, "SELECT 1", "after-script-error")
        .await
        .is_ok());

    // Final results stay editable, and refreshing them cannot replay the UPDATE.
    let script = db.script(&p.id,
        "UPDATE fixture.products SET stock=stock+1 WHERE id=1001; SELECT id,name,stock FROM fixture.products ORDER BY id;",
        "editable-script").await.unwrap();
    let result = script.result.unwrap();
    assert!(result.columns[1].editable);
    db.apply(
        &p.id,
        &result.id,
        &[CellEdit {
            row: 0,
            column: 1,
            value: Some("Renamed".into()),
        }],
    )
    .await
    .unwrap();
    let refreshed = db
        .query(
            &p.id,
            script.refresh_sql.as_deref().unwrap(),
            "refresh-final",
        )
        .await
        .unwrap();
    assert_eq!(refreshed.rows[0][1].as_deref(), Some("Renamed"));
    assert_eq!(refreshed.rows[0][2].as_deref(), Some("44"));

    let union = db.script(&p.id,
        "UPDATE fixture.products SET status='active' WHERE id=1001; SELECT 'products' AS src,id::text AS row FROM fixture.products WHERE id=1001 UNION ALL SELECT 'composite',tenant || '/' || id::text FROM fixture.composite WHERE (tenant,id) IN (('one',1),('two',1));",
        "verify-union").await.unwrap();
    assert!(union.error.is_none());
    assert_eq!(union.result.unwrap().rows.len(), 3);
    assert!(union.refresh_sql.unwrap().starts_with("SELECT 'products'"));

    // Unsupported transaction control anywhere is rejected before the first write.
    assert!(db
        .script(&p.id, "DELETE FROM fixture.products; COMMIT;", "control")
        .await
        .is_err());
    assert_eq!(
        external
            .query_one("SELECT count(*) FROM fixture.products", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        3
    );
    db.disconnect(&p.id).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; use npm run test:database"]
async fn scripts_honor_cancellation_limits_and_read_only() {
    let (db, p, external) = setup().await;
    let (canceled, cancel_result) = tokio::join!(
        db.script(&p.id, "UPDATE fixture.products SET stock=stock+1 WHERE id=1001; SELECT pg_sleep(15); DELETE FROM fixture.products;", "cancel-script"),
        async { tokio::time::sleep(std::time::Duration::from_millis(300)).await; db.cancel(&p.id, "cancel-script").await }
    );
    cancel_result.unwrap();
    let canceled = canceled.unwrap();
    assert_eq!(canceled.statements.len(), 1);
    assert!(canceled.error.unwrap().contains("canceled"));
    assert_eq!(
        external
            .query_one("SELECT stock FROM fixture.products WHERE id=1001", &[])
            .await
            .unwrap()
            .get::<_, i32>(0),
        43
    );
    assert_eq!(
        external
            .query_one("SELECT count(*) FROM fixture.products", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        3
    );

    let limited = db
        .script(
            &p.id,
            "SELECT generate_series(1,10000); DELETE FROM fixture.products;",
            "limit-script",
        )
        .await
        .unwrap();
    assert_eq!(limited.statements.len(), 1);
    assert!(!limited.statements[0].committed);
    assert!(limited.error.unwrap().contains("result limit"));
    assert!(limited.result.unwrap().truncated);
    assert!(db.query(&p.id, "SELECT 1", "after-limit").await.is_ok());

    let mut readonly = p.clone();
    readonly.id = uuid::Uuid::new_v4().to_string();
    readonly.read_only = true;
    db.connect(
        readonly.clone(),
        std::env::var("HEY_DB_TEST_PASSWORD").unwrap(),
    )
    .await
    .unwrap();
    let denied = db
        .script(
            &readonly.id,
            "SELECT 1; DELETE FROM fixture.products; SELECT 2;",
            "readonly-script",
        )
        .await
        .unwrap();
    assert_eq!(denied.statements.len(), 1);
    assert!(denied.error.unwrap().contains("read-only"));
    assert_eq!(
        external
            .query_one("SELECT count(*) FROM fixture.products", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        3
    );
    db.disconnect(&readonly.id).await.unwrap();
    db.disconnect(&p.id).await.unwrap();
}
