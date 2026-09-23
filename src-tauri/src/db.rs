use crate::{
    edits::{self, TextParameter},
    model::*,
};
use futures_util::{pin_mut, TryStreamExt};
use postgres_native_tls::MakeTlsConnector;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex as SyncMutex},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock};
use tokio_postgres::{types::ToSql, CancelToken, Client, Config, SimpleQueryMessage, Transaction};

const MAX_ROWS: usize = 1000;
const MAX_BYTES: usize = 8 * 1024 * 1024;
type ActiveOperation = SyncMutex<Option<(String, CancelToken)>>;
struct Session {
    client: Mutex<Client>,
    profile: Profile,
    active: ActiveOperation,
    snapshots: Mutex<VecDeque<Snapshot>>,
    driver: tokio::task::JoinHandle<()>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
struct ActiveGuard<'a>(&'a ActiveOperation);
impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.lock() {
            *active = None;
        }
    }
}
#[derive(Default)]
pub struct Database {
    sessions: RwLock<HashMap<String, Arc<Session>>>,
}

fn error(err: tokio_postgres::Error) -> String {
    if let Some(db) = err.as_db_error() {
        if db.code().code() == "57014" {
            return "Query canceled or its time limit was reached.".into();
        }
        format!("{} (PostgreSQL {})", db.message(), db.code().code())
    } else {
        format!("Database connection error: {err}")
    }
}
fn tls() -> Result<MakeTlsConnector, String> {
    native_tls::TlsConnector::builder()
        .build()
        .map(MakeTlsConnector::new)
        .map_err(|_| "Could not initialize secure connections.".into())
}

impl Database {
    async fn session(&self, id: &str) -> Result<Arc<Session>, String> {
        self.sessions
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or("Connect to a database first.".into())
    }
    pub async fn connect(&self, profile: Profile, password: String) -> Result<(), String> {
        profile.validate()?;
        let mut config = Config::new();
        config
            .host(&profile.host)
            .port(profile.port)
            .dbname(&profile.database)
            .user(&profile.username)
            .password(password)
            .application_name("hey db")
            .connect_timeout(Duration::from_secs(10))
            .ssl_mode(if profile.tls == "disable" {
                tokio_postgres::config::SslMode::Disable
            } else {
                tokio_postgres::config::SslMode::Require
            });
        let (client, connection) =
            tokio::time::timeout(Duration::from_secs(15), config.connect(tls()?))
                .await
                .map_err(|_| "Connection timed out after 15 seconds.")?
                .map_err(error)?;
        let driver = tokio::spawn(async move {
            let _ = connection.await;
        });
        client.batch_execute("SET statement_timeout = '120s'; SET lock_timeout = '5s'; SET idle_in_transaction_session_timeout = '30s'; SET standard_conforming_strings = on;").await.map_err(error)?;
        self.sessions.write().await.insert(
            profile.id.clone(),
            Arc::new(Session {
                client: Mutex::new(client),
                profile,
                active: SyncMutex::new(None),
                snapshots: Mutex::new(VecDeque::new()),
                driver,
            }),
        );
        Ok(())
    }
    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        if let Some(session) = self.sessions.write().await.remove(id) {
            let token = session
                .active
                .lock()
                .map_err(|_| "Connection is busy.")?
                .as_ref()
                .map(|(_, token)| token.clone());
            if let Some(token) = token {
                let _ = token.cancel_query(tls()?).await;
            }
        }
        Ok(())
    }
    pub async fn cancel(&self, id: &str, operation: &str) -> Result<(), String> {
        let session = self.session(id).await?;
        let token = session
            .active
            .lock()
            .map_err(|_| "Connection is busy.")?
            .as_ref()
            .and_then(|(active, token)| (active == operation).then(|| token.clone()));
        if let Some(token) = token {
            token.cancel_query(tls()?).await.map_err(error)?;
        }
        Ok(())
    }
    pub async fn tables(&self, id: &str) -> Result<Vec<TableInfo>, String> {
        let session = self.session(id).await?;
        let client = session
            .client
            .try_lock()
            .map_err(|_| "Wait for the running query to finish.")?;
        let rows=client.query("SELECT c.oid,n.nspname,c.relname,c.relkind::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p','v','m') AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%' ORDER BY n.nspname,c.relname LIMIT 10000",&[]).await.map_err(error)?;
        Ok(rows
            .iter()
            .map(|r| TableInfo {
                oid: r.get(0),
                schema: r.get(1),
                name: r.get(2),
                kind: r.get(3),
            })
            .collect())
    }
    pub async fn details(&self, id: &str, oid: u32) -> Result<TableDetails, String> {
        let session = self.session(id).await?;
        let client = session
            .client
            .try_lock()
            .map_err(|_| "Wait for the running query to finish.")?;
        let rows=client.query("SELECT a.attname,format_type(a.atttypid,a.atttypmod),NOT a.attnotnull,pg_get_expr(d.adbin,d.adrelid),EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid=a.attrelid AND i.indisprimary AND a.attnum=ANY(i.indkey)) FROM pg_attribute a LEFT JOIN pg_attrdef d ON a.attrelid=d.adrelid AND a.attnum=d.adnum WHERE a.attrelid=$1 AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",&[&oid]).await.map_err(error)?;
        let indexes=client.query("SELECT c.relname,pg_get_indexdef(i.indexrelid) FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid WHERE i.indrelid=$1 ORDER BY c.relname",&[&oid]).await.map_err(error)?;
        Ok(TableDetails {
            columns: rows
                .iter()
                .map(|r| ColumnInfo {
                    name: r.get(0),
                    data_type: r.get(1),
                    nullable: r.get(2),
                    default_value: r.get(3),
                    primary_key: r.get(4),
                })
                .collect(),
            indexes: indexes
                .iter()
                .map(|r| IndexInfo {
                    name: r.get(0),
                    definition: r.get(1),
                })
                .collect(),
        })
    }
    pub async fn query(&self, id: &str, sql: &str, operation: &str) -> Result<QueryResult, String> {
        if sql.len() > 1024 * 1024 {
            return Err("SQL is limited to 1 MiB per statement.".into());
        }
        let ast = edits::parse_statement(sql)?;
        let is_select = matches!(&ast, sqlparser::ast::Statement::Query(_));
        let session = self.session(id).await?;
        let mut client = session
            .client
            .try_lock()
            .map_err(|_| "A query is already running on this connection.")?;
        let token = client.cancel_token();
        *session.active.lock().map_err(|_| "Connection is busy.")? =
            Some((operation.into(), token.clone()));
        let _guard = ActiveGuard(&session.active);
        let start = Instant::now();
        let tx = client
            .build_transaction()
            .read_only(session.profile.read_only)
            .start()
            .await
            .map_err(error)?;
        let prepared = tx.prepare(sql).await.map_err(error)?;
        let mut columns: Vec<_> = prepared
            .columns()
            .iter()
            .map(|c| ResultColumn {
                name: c.name().into(),
                data_type: c.type_().name().into(),
                editable: false,
                primary_key: false,
            })
            .collect();
        let (mut target, mut reason) = if session.profile.read_only {
            (None, Some("This connection is in read-only mode.".into()))
        } else if !edits::is_plain_select(&ast) {
            (None,Some("Only direct columns from a single table can be edited. Joins, expressions, aggregates and CTEs are read-only.".into()))
        } else {
            match target_for(&tx, prepared.columns()).await {
                Ok(target) => (Some(target), None),
                Err(why) => (None, Some(why)),
            }
        };
        let mut rows = Vec::new();
        let mut affected_rows = 0;
        let mut bytes = 0;
        let mut truncated = false;
        {
            let stream = tx.client().simple_query_raw(sql).await.map_err(error)?;
            pin_mut!(stream);
            loop {
                match stream.try_next().await {
                    Ok(Some(SimpleQueryMessage::Row(row))) => {
                        if truncated {
                            continue;
                        }
                        let values: Vec<Option<String>> = (0..row.len())
                            .map(|i| row.get(i).map(str::to_owned))
                            .collect();
                        let size: usize = values.iter().flatten().map(String::len).sum();
                        if rows.len() >= MAX_ROWS || bytes + size > MAX_BYTES {
                            truncated = true;
                            token.cancel_query(tls()?).await.map_err(error)?;
                        } else {
                            bytes += size;
                            rows.push(values);
                        }
                    }
                    Ok(Some(SimpleQueryMessage::CommandComplete(count))) => affected_rows = count,
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(err) if truncated && err.code().is_some_and(|c| c.code() == "57014") => {
                        break
                    }
                    Err(err) => return Err(error(err)),
                }
            }
        }
        if truncated {
            tx.rollback().await.map_err(error)?;
            if !is_select {
                return Err("The result exceeded the display limit. The statement was rolled back; no changes were committed.".into());
            }
        } else {
            tx.commit().await.map_err(error)?;
        }
        if let Some(ref candidate) = target {
            let keys: Vec<_> = candidate
                .columns
                .iter()
                .enumerate()
                .filter_map(|(i, c)| c.key.then_some(i))
                .collect();
            let mut seen = HashSet::new();
            if rows
                .iter()
                .any(|row| !seen.insert(keys.iter().map(|&i| row[i].clone()).collect::<Vec<_>>()))
            {
                target = None;
                reason =
                    Some("Duplicate primary keys in the result make editing ambiguous.".into());
            }
        }
        if let Some(ref candidate) = target {
            for (column, source) in columns.iter_mut().zip(&candidate.columns) {
                column.editable = source.editable;
                column.primary_key = source.key;
            }
        }
        let result = QueryResult {
            id: uuid::Uuid::new_v4().to_string(),
            columns,
            rows,
            affected_rows,
            elapsed_ms: start.elapsed().as_millis(),
            truncated,
            read_only_reason: reason,
            table: target.as_ref().map(edits::qualified),
        };
        let mut snapshots = session.snapshots.lock().await;
        // Only the most recent result can be edited; the UI keeps one active result.
        snapshots.clear();
        snapshots.push_back(Snapshot {
            result: result.clone(),
            target,
        });
        Ok(result)
    }
    async fn snapshot(&self, session: &Session, result_id: &str) -> Result<Snapshot, String> {
        session
            .snapshots
            .lock()
            .await
            .iter()
            .find(|s| s.result.id == result_id)
            .cloned()
            .ok_or("This result has expired. Run the query again before editing.".into())
    }
    pub async fn preview(
        &self,
        id: &str,
        result_id: &str,
        changes: &[CellEdit],
    ) -> Result<Vec<PlannedUpdate>, String> {
        let session = self.session(id).await?;
        let snapshot = self.snapshot(&session, result_id).await?;
        edits::plan(&snapshot, changes)
    }
    pub async fn apply(
        &self,
        id: &str,
        result_id: &str,
        changes: &[CellEdit],
    ) -> Result<usize, String> {
        let session = self.session(id).await?;
        if session.profile.read_only {
            return Err("This connection is read-only.".into());
        }
        let mut client = session
            .client
            .try_lock()
            .map_err(|_| "Wait for the running query to finish.")?;
        let snapshot = self.snapshot(&session, result_id).await?;
        let updates = edits::plan(&snapshot, changes)?;
        let target = snapshot
            .target
            .as_ref()
            .ok_or("This result is read-only.")?;
        let tx = client.transaction().await.map_err(error)?;
        tx.batch_execute("SET LOCAL statement_timeout = '15s'; SET LOCAL lock_timeout = '5s';")
            .await
            .map_err(error)?;
        tx.batch_execute(&format!(
            "LOCK TABLE ONLY {} IN ROW EXCLUSIVE MODE",
            edits::qualified(target)
        ))
        .await
        .map_err(error)?;
        let current=tx.query_opt("SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2",&[&target.schema,&target.table]).await.map_err(error)?.ok_or("The source table no longer exists.")?;
        if current.get::<_, u32>(0) != target.oid {
            return Err("The table was replaced. Run the query again.".into());
        }
        for column in &target.columns {
            let valid=tx.query_opt("SELECT attname,atttypid,attgenerated::text,EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid=a.attrelid AND i.indisprimary AND a.attnum=ANY(i.indkey)) FROM pg_attribute a WHERE attrelid=$1 AND attnum=$2 AND NOT attisdropped",&[&target.oid,&column.attribute]).await.map_err(error)?.ok_or("The table structure changed. Run the query again.")?;
            if valid.get::<_, String>(0) != column.name
                || valid.get::<_, u32>(1) != column.type_oid
                || valid.get::<_, bool>(3) != column.key
                || (column.editable && !valid.get::<_, String>(2).is_empty())
            {
                return Err("The table structure changed. Run the query again.".into());
            }
        }
        for update in &updates {
            let params: Vec<TextParameter> = update
                .parameters
                .iter()
                .cloned()
                .map(TextParameter)
                .collect();
            let refs: Vec<&(dyn ToSql + Sync)> =
                params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
            let count = tx
                .execute(update.sql.as_str(), &refs)
                .await
                .map_err(error)?;
            if count != 1 {
                tx.rollback().await.map_err(error)?;
                return Err(format!("Row {} changed, was deleted, or could not be updated. All pending updates were rolled back. Discard edits and refresh before trying again.",update.row+1));
            }
        }
        tx.commit().await.map_err(error)?;
        session.snapshots.lock().await.clear();
        Ok(updates.len())
    }
}

async fn target_for(
    tx: &Transaction<'_>,
    columns: &[tokio_postgres::Column],
) -> Result<Target, String> {
    let oid = columns
        .first()
        .and_then(|c| c.table_oid())
        .ok_or("The result does not identify a source table.")?;
    if columns
        .iter()
        .any(|c| c.table_oid() != Some(oid) || c.column_id().is_none())
    {
        return Err("Some columns are expressions or come from different tables.".into());
    }
    let relation=tx.query_one("SELECT n.nspname,c.relname,c.relkind::text,EXISTS(SELECT 1 FROM pg_inherits WHERE inhrelid=c.oid OR inhparent=c.oid) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.oid=$1",&[&oid]).await.map_err(error)?;
    if relation.get::<_, String>(2) != "r" || relation.get::<_, bool>(3) {
        return Err(
            "Views and inherited or partitioned tables are read-only in this version.".into(),
        );
    }
    let attrs=tx.query("SELECT a.attnum,a.attname,a.atttypid,a.attgenerated::text,a.attidentity::text,EXISTS(SELECT 1 FROM pg_index i WHERE i.indrelid=a.attrelid AND i.indisprimary AND a.attnum=ANY(i.indkey)) FROM pg_attribute a WHERE a.attrelid=$1 AND a.attnum>0 AND NOT a.attisdropped",&[&oid]).await.map_err(error)?;
    let keys: Vec<i16> = attrs
        .iter()
        .filter(|r| r.get::<_, bool>(5))
        .map(|r| r.get(0))
        .collect();
    if keys.is_empty() {
        return Err("This table has no primary key. Its results are read-only.".into());
    }
    let projected: HashSet<i16> = columns.iter().filter_map(|c| c.column_id()).collect();
    if projected.len() != columns.len() {
        return Err("The same source column appears more than once.".into());
    }
    if keys.iter().any(|key| !projected.contains(key)) {
        return Err("Include every primary key column in SELECT to enable cell editing.".into());
    }
    let mut sources = Vec::new();
    for column in columns {
        let attr = attrs
            .iter()
            .find(|r| Some(r.get::<_, i16>(0)) == column.column_id())
            .ok_or("The source column could not be identified.")?;
        let key = attr.get::<_, bool>(5);
        sources.push(SourceColumn {
            name: attr.get(1),
            attribute: attr.get(0),
            type_oid: attr.get(2),
            key,
            editable: !key
                && attr.get::<_, String>(3).is_empty()
                && attr.get::<_, String>(4).is_empty(),
        });
    }
    Ok(Target {
        oid,
        schema: relation.get(0),
        table: relation.get(1),
        columns: sources,
    })
}
