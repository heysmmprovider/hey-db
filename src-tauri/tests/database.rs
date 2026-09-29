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

    // An unmatched transaction boundary is rejected before the first write.
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

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; use npm run test:database"]
async fn explicit_transactions_commit_migrations_and_keep_the_final_result() {
    let (db, p, external) = setup().await;
    let migration = db
        .script(
            &p.id,
            r#"
        BEGIN;
        CREATE TABLE fixture.migration_parent (id integer PRIMARY KEY, reward integer NOT NULL);
        INSERT INTO fixture.migration_parent VALUES (1, 25);
        CREATE TABLE fixture.migration_child (
            id integer PRIMARY KEY,
            parent_id integer REFERENCES fixture.migration_parent(id),
            reward integer,
            transaction_id bigint DEFAULT txid_current()
        );
        CREATE FUNCTION fixture.snapshot_reward() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
            SELECT reward INTO NEW.reward FROM fixture.migration_parent WHERE id = NEW.parent_id;
            RETURN NEW;
        END $$;
        CREATE TRIGGER snapshot_reward BEFORE INSERT ON fixture.migration_child
            FOR EACH ROW EXECUTE FUNCTION fixture.snapshot_reward();
        INSERT INTO fixture.migration_child(id, parent_id) VALUES (1, 1);
        INSERT INTO fixture.migration_child(id, parent_id) VALUES (2, 1);
        SELECT id, reward, transaction_id FROM fixture.migration_child ORDER BY id;
        COMMIT;
    "#,
            "migration",
        )
        .await
        .unwrap();
    assert!(migration.error.is_none(), "{:?}", migration.error);
    assert_eq!(migration.statements.len(), 10);
    assert!(migration.statements.iter().all(|s| s.committed));
    let rows = &migration.result.as_ref().unwrap().rows;
    assert_eq!(rows[0][1].as_deref(), Some("25"));
    assert_eq!(rows[0][2], rows[1][2], "both writes share a transaction");
    assert!(migration
        .refresh_sql
        .unwrap()
        .starts_with("SELECT id, reward"));
    assert_eq!(
        external
            .query_one("SELECT count(*) FROM fixture.migration_child", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        2
    );

    let mode = db.script(&p.id,
        "START TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY; SELECT current_setting('transaction_isolation'), current_setting('transaction_read_only'); COMMIT;",
        "transaction-mode").await.unwrap();
    assert!(mode.error.is_none(), "{:?}", mode.error);
    assert_eq!(
        mode.result.unwrap().rows[0],
        [Some("repeatable read".into()), Some("on".into())]
    );

    let script = db.script(&p.id,
        "BEGIN; UPDATE fixture.products SET stock=stock+1 WHERE id=1001; SELECT id,name,stock FROM fixture.products ORDER BY id; COMMIT;",
        "editable-transaction").await.unwrap();
    assert!(script.error.is_none(), "{:?}", script.error);
    let result = script.result.unwrap();
    assert!(result.columns[1].editable);
    db.apply(
        &p.id,
        &result.id,
        &[CellEdit {
            row: 0,
            column: 1,
            value: Some("Edited after commit".into()),
        }],
    )
    .await
    .unwrap();
    let refreshed = db
        .query(&p.id, script.refresh_sql.as_deref().unwrap(), "refresh")
        .await
        .unwrap();
    assert_eq!(refreshed.rows[0][1].as_deref(), Some("Edited after commit"));
    assert_eq!(
        refreshed.rows[0][2].as_deref(),
        Some("43"),
        "refresh must not replay the update"
    );
    db.disconnect(&p.id).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; use npm run test:database"]
async fn explicit_transactions_roll_back_failures_and_preserve_prior_commits() {
    let (db, p, external) = setup().await;
    let failed = db
        .script(
            &p.id,
            r#"
        UPDATE fixture.products SET stock=stock+1 WHERE id=1001;
        BEGIN;
        CREATE TABLE fixture.should_rollback (id integer PRIMARY KEY);
        UPDATE fixture.products SET stock=0 WHERE id=1001;
        UPDATE fixture.products SET stock=-1 WHERE id=1002;
        COMMIT;
        DELETE FROM fixture.products;
    "#,
            "failed-block",
        )
        .await
        .unwrap();
    assert!(failed.error.unwrap().contains("Statement 5 of 7 failed"));
    assert_eq!(failed.statements.len(), 4);
    assert!(failed.statements[0].committed);
    assert!(failed.statements[1..].iter().all(|s| !s.committed));
    assert!(failed.result.is_none());
    assert!(failed.refresh_sql.is_none());
    assert_eq!(
        external
            .query_one("SELECT stock FROM fixture.products WHERE id=1001", &[])
            .await
            .unwrap()
            .get::<_, i32>(0),
        43
    );
    assert!(external
        .query_one("SELECT to_regclass('fixture.should_rollback')::text", &[])
        .await
        .unwrap()
        .get::<_, Option<String>>(0)
        .is_none());
    assert_eq!(
        external
            .query_one("SELECT count(*) FROM fixture.products", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        3
    );

    external.batch_execute("CREATE TABLE fixture.deferred_unique (id integer UNIQUE DEFERRABLE INITIALLY DEFERRED)").await.unwrap();
    let deferred = db.script(&p.id,
        "BEGIN; INSERT INTO fixture.deferred_unique VALUES (1); INSERT INTO fixture.deferred_unique VALUES (1); COMMIT; DELETE FROM fixture.products;",
        "failed-commit").await.unwrap();
    assert!(deferred.error.unwrap().contains("Statement 4 of 5 failed"));
    assert!(deferred.statements.iter().all(|s| !s.committed));
    assert!(deferred.result.is_none());
    assert!(deferred.refresh_sql.is_none());
    assert_eq!(
        external
            .query_one("SELECT count(*) FROM fixture.deferred_unique", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert!(db
        .query(&p.id, "SELECT 1", "after-commit-error")
        .await
        .is_ok());

    let rollback = db.script(&p.id,
        "BEGIN; UPDATE fixture.products SET stock=0; SELECT id,name,stock FROM fixture.products; ROLLBACK;",
        "explicit-rollback").await.unwrap();
    assert!(rollback.error.is_none(), "{:?}", rollback.error);
    assert!(rollback.statements.iter().all(|s| !s.committed));
    assert!(rollback.result.is_none());
    assert!(rollback.refresh_sql.is_none());

    let mixed = db.script(&p.id,
        "BEGIN; UPDATE fixture.products SET stock=0; ROLLBACK; BEGIN; UPDATE fixture.products SET stock=stock+1 WHERE id=1001; COMMIT; SELECT stock FROM fixture.products WHERE id=1001;",
        "mixed-blocks").await.unwrap();
    assert!(mixed.error.is_none(), "{:?}", mixed.error);
    assert!(mixed.statements[..3].iter().all(|s| !s.committed));
    assert!(mixed.statements[3..].iter().all(|s| s.committed));
    assert_eq!(mixed.result.unwrap().rows[0][0].as_deref(), Some("44"));

    for invalid in [
        "DELETE FROM fixture.products; BEGIN; SELECT 1;",
        "DELETE FROM fixture.products; BEGIN; BEGIN; COMMIT; COMMIT;",
        "DELETE FROM fixture.products; BEGIN; SELECT 1; COMMIT AND CHAIN;",
    ] {
        assert!(db.script(&p.id, invalid, "preflight").await.is_err());
    }
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
async fn explicit_transactions_roll_back_on_cancel_limits_and_read_only_errors() {
    let (db, p, external) = setup().await;
    let (canceled, cancel_result) = tokio::join!(
        db.script(&p.id, "BEGIN; UPDATE fixture.products SET stock=0; SELECT pg_sleep(15); COMMIT; DELETE FROM fixture.products;", "cancel-block"),
        async { tokio::time::sleep(std::time::Duration::from_millis(300)).await; db.cancel(&p.id, "cancel-block").await }
    );
    cancel_result.unwrap();
    let canceled = canceled.unwrap();
    assert!(canceled.error.unwrap().contains("canceled"));
    assert!(canceled.statements.iter().all(|s| !s.committed));
    assert!(canceled.result.is_none());
    assert!(db.query(&p.id, "SELECT 1", "after-cancel").await.is_ok());

    let limited = db.script(&p.id,
        "BEGIN; UPDATE fixture.products SET stock=0; SELECT generate_series(1,10000); COMMIT; DELETE FROM fixture.products;",
        "limited-block").await.unwrap();
    assert!(limited.error.unwrap().contains("result limit"));
    assert!(limited.statements.iter().all(|s| !s.committed));
    let result = limited.result.unwrap();
    assert!(result.truncated);
    assert!(result.columns.iter().all(|c| !c.editable));
    assert!(limited.refresh_sql.is_none());
    assert!(db
        .preview(
            &p.id,
            &result.id,
            &[CellEdit {
                row: 0,
                column: 0,
                value: Some("0".into())
            }]
        )
        .await
        .is_err());

    let oversized_write = db.script(&p.id,
        "BEGIN; UPDATE fixture.products SET stock=0; INSERT INTO fixture.keyless SELECT 'new' FROM generate_series(1,1001) RETURNING *; COMMIT;",
        "oversized-write").await.unwrap();
    assert!(oversized_write.error.unwrap().contains("display limit"));
    assert!(oversized_write.statements.iter().all(|s| !s.committed));
    assert!(oversized_write.result.is_none());
    assert_eq!(
        external
            .query_one("SELECT count(*) FROM fixture.keyless", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );

    let stocks: Vec<i32> = external
        .query("SELECT stock FROM fixture.products ORDER BY id", &[])
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(stocks, [42, 128, 16]);

    let mut readonly = p.clone();
    readonly.id = uuid::Uuid::new_v4().to_string();
    readonly.read_only = true;
    db.connect(
        readonly.clone(),
        std::env::var("HEY_DB_TEST_PASSWORD").unwrap(),
    )
    .await
    .unwrap();
    for sql in [
        "BEGIN; SELECT 1; DELETE FROM fixture.products; COMMIT;",
        "BEGIN READ WRITE; DELETE FROM fixture.products; COMMIT;",
    ] {
        let denied = db
            .script(&readonly.id, sql, "readonly-block")
            .await
            .unwrap();
        assert!(denied.error.unwrap().contains("read-only"));
        assert!(denied.statements.iter().all(|s| !s.committed));
        assert!(denied.result.is_none());
    }
    let allowed = db
        .script(
            &readonly.id,
            "BEGIN READ ONLY; SELECT 1; COMMIT;",
            "readonly-select",
        )
        .await
        .unwrap();
    assert!(allowed.error.is_none(), "{:?}", allowed.error);
    assert!(allowed.statements.iter().all(|s| s.committed));
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
