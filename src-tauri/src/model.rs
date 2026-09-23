use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub tls: String,
    pub read_only: bool,
    pub remember_password: bool,
}

impl Profile {
    pub fn validate(&self) -> Result<(), String> {
        uuid::Uuid::parse_str(&self.id).map_err(|_| "Invalid connection ID")?;
        if self.host.trim().is_empty()
            || self.database.trim().is_empty()
            || self.username.trim().is_empty()
            || self.port == 0
        {
            return Err("Host, port, database, and username are required.".into());
        }
        if !["verify-full", "disable"].contains(&self.tls.as_str()) {
            return Err("Unknown TLS mode.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultColumn {
    pub name: String,
    pub data_type: String,
    pub editable: bool,
    pub primary_key: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub id: String,
    pub columns: Vec<ResultColumn>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected_rows: u64,
    pub elapsed_ms: u128,
    pub truncated: bool,
    pub read_only_reason: Option<String>,
    pub table: Option<String>,
}

#[derive(Clone)]
pub struct SourceColumn {
    pub name: String,
    pub attribute: i16,
    pub type_oid: u32,
    pub key: bool,
    pub editable: bool,
}

#[derive(Clone)]
pub struct Target {
    pub oid: u32,
    pub schema: String,
    pub table: String,
    pub columns: Vec<SourceColumn>,
}

#[derive(Clone)]
pub struct Snapshot {
    pub result: QueryResult,
    pub target: Option<Target>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellEdit {
    pub row: usize,
    pub column: usize,
    pub value: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedUpdate {
    pub sql: String,
    pub parameters: Vec<Option<String>>,
    pub row: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub oid: u32,
    pub schema: String,
    pub name: String,
    pub kind: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub primary_key: bool,
}

#[derive(Serialize)]
pub struct IndexInfo {
    pub name: String,
    pub definition: String,
}

#[derive(Serialize)]
pub struct TableDetails {
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
}
