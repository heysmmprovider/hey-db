use crate::model::*;
use bytes::BytesMut;
use sqlparser::{
    ast::{Expr, GroupByExpr, SelectItem, SetExpr, Statement, TableFactor},
    dialect::PostgreSqlDialect,
    parser::Parser,
};
use std::collections::{BTreeMap, HashSet};
use tokio_postgres::types::{Format, IsNull, ToSql, Type};

pub fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
pub fn qualified(target: &Target) -> String {
    format!(
        "{}.{}",
        quote_ident(&target.schema),
        quote_ident(&target.table)
    )
}

pub fn parse_statement(sql: &str) -> Result<Statement, String> {
    let mut statements = Parser::parse_sql(&PostgreSqlDialect {}, sql)
        .map_err(|e| format!("SQL could not be parsed: {e}"))?;
    if statements.len() != 1 {
        return Err(
            "Run one SQL statement at a time. Select the statement you want to execute.".into(),
        );
    }
    let stmt = statements.remove(0);
    let text = stmt.to_string();
    let first = text.split_whitespace().next().unwrap_or("").to_uppercase();
    if [
        "BEGIN",
        "START",
        "COMMIT",
        "ROLLBACK",
        "SAVEPOINT",
        "RELEASE",
        "SET",
        "RESET",
        "DISCARD",
        "END",
        "ABORT",
        "PREPARE",
        "EXECUTE",
        "DEALLOCATE",
        "COPY",
        "CALL",
        "DO",
    ]
    .contains(&first.as_str())
    {
        return Err("This first version runs each statement in its own transaction. Session commands, COPY, and explicit transaction control are not supported yet.".into());
    }
    Ok(stmt)
}

pub fn is_plain_select(stmt: &Statement) -> bool {
    let Statement::Query(query) = stmt else {
        return false;
    };
    if query.with.is_some() {
        return false;
    }
    let SetExpr::Select(select) = query.body.as_ref() else {
        return false;
    };
    if select.from.len() != 1
        || !select.from[0].joins.is_empty()
        || select.distinct.is_some()
        || select.into.is_some()
        || select.having.is_some()
        || select.qualify.is_some()
        || !select.named_window.is_empty()
        || !select.lateral_views.is_empty()
    {
        return false;
    }
    if !matches!(&select.group_by, GroupByExpr::Expressions(items, _) if items.is_empty()) {
        return false;
    }
    if !matches!(
        &select.from[0].relation,
        TableFactor::Table { args: None, .. }
    ) {
        return false;
    }
    select.projection.iter().all(|item| {
        matches!(
            item,
            SelectItem::UnnamedExpr(Expr::Identifier(_) | Expr::CompoundIdentifier(_))
                | SelectItem::ExprWithAlias {
                    expr: Expr::Identifier(_) | Expr::CompoundIdentifier(_),
                    ..
                }
                | SelectItem::Wildcard(_)
                | SelectItem::QualifiedWildcard(_, _)
        )
    })
}

// Parameters use PostgreSQL's text wire format. The server parses values according
// to the inferred column type, preserving big integers, decimals and custom types.
#[derive(Debug)]
pub struct TextParameter(pub Option<String>);
impl ToSql for TextParameter {
    fn to_sql(
        &self,
        _: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match &self.0 {
            Some(value) => {
                out.extend_from_slice(value.as_bytes());
                Ok(IsNull::No)
            }
            None => Ok(IsNull::Yes),
        }
    }
    fn accepts(_: &Type) -> bool {
        true
    }
    fn encode_format(&self, _: &Type) -> Format {
        Format::Text
    }
    tokio_postgres::types::to_sql_checked!();
}

pub fn plan(snapshot: &Snapshot, edits: &[CellEdit]) -> Result<Vec<PlannedUpdate>, String> {
    let target = snapshot
        .target
        .as_ref()
        .ok_or("This result is read-only.")?;
    if edits.is_empty() || edits.len() > 4000 {
        return Err("Choose between 1 and 4,000 cell changes.".into());
    }
    let keys: Vec<usize> = target
        .columns
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.key.then_some(i))
        .collect();
    if keys.is_empty() {
        return Err("A complete primary key is required.".into());
    }
    let mut grouped: BTreeMap<usize, BTreeMap<usize, &Option<String>>> = BTreeMap::new();
    let mut seen = HashSet::new();
    for edit in edits {
        let original = snapshot
            .result
            .rows
            .get(edit.row)
            .ok_or("The result row no longer exists.")?;
        let column = target
            .columns
            .get(edit.column)
            .ok_or("Unknown result column.")?;
        if !column.editable {
            return Err(
                "Primary keys, generated columns, and unsupported columns are read-only.".into(),
            );
        }
        if !seen.insert((edit.row, edit.column)) {
            return Err("Duplicate cell edit.".into());
        }
        if edit.value.as_ref().is_some_and(|v| v.len() > 1024 * 1024) {
            return Err("Cell values must be smaller than 1 MiB.".into());
        }
        if original[edit.column] != edit.value {
            grouped
                .entry(edit.row)
                .or_default()
                .insert(edit.column, &edit.value);
        }
    }
    let mut updates = Vec::new();
    for (row, changes) in grouped {
        let original = &snapshot.result.rows[row];
        let mut parameters = Vec::new();
        let mut assignments = Vec::new();
        for (&column, &value) in &changes {
            parameters.push(value.clone());
            assignments.push(format!(
                "{} = ${}",
                quote_ident(&target.columns[column].name),
                parameters.len()
            ));
        }
        let mut predicates = Vec::new();
        for &key in &keys {
            if original[key].is_none() {
                return Err("A primary key value is missing.".into());
            }
            parameters.push(original[key].clone());
            predicates.push(format!(
                "{} = ${}",
                quote_ident(&target.columns[key].name),
                parameters.len()
            ));
        }
        // Match the loaded values as well as the key: no silent lost updates.
        for &column in changes.keys() {
            parameters.push(original[column].clone());
            predicates.push(format!(
                "{}::text IS NOT DISTINCT FROM ${}",
                quote_ident(&target.columns[column].name),
                parameters.len()
            ));
        }
        updates.push(PlannedUpdate {
            sql: format!(
                "UPDATE ONLY {}\nSET {}\nWHERE {};",
                qualified(target),
                assignments.join(", "),
                predicates.join("\n  AND ")
            ),
            parameters,
            row,
        });
    }
    if updates.is_empty() {
        return Err("There are no changed values to apply.".into());
    }
    Ok(updates)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        Snapshot {
            result: QueryResult {
                id: "test".into(),
                columns: vec![],
                rows: vec![vec![
                    Some("1003".into()),
                    Some("Monitor stand".into()),
                    Some("eu".into()),
                ]],
                affected_rows: 0,
                elapsed_ms: 0,
                truncated: false,
                read_only_reason: None,
                table: None,
            },
            target: Some(Target {
                oid: 42,
                schema: "odd\"schema".into(),
                table: "products".into(),
                columns: vec![
                    SourceColumn {
                        name: "id".into(),
                        attribute: 1,
                        type_oid: 23,
                        key: true,
                        editable: false,
                    },
                    SourceColumn {
                        name: "name".into(),
                        attribute: 2,
                        type_oid: 25,
                        key: false,
                        editable: true,
                    },
                    SourceColumn {
                        name: "region".into(),
                        attribute: 3,
                        type_oid: 25,
                        key: true,
                        editable: false,
                    },
                ],
            }),
        }
    }
    #[test]
    fn accepts_simple_queries_and_aliases() {
        for sql in ["SELECT id,name,status,stock FROM public.products WHERE status='active' ORDER BY id LIMIT 100", "SELECT p.id AS key, p.name FROM products p", "SELECT * FROM products"] {
            assert!(is_plain_select(&parse_statement(sql).unwrap()), "{sql}");
        }
    }
    #[test]
    fn complex_results_are_read_only() {
        for sql in [
            "SELECT p.id,p.name FROM products p JOIN orders o ON p.id=o.id",
            "SELECT DISTINCT id,name FROM products",
            "SELECT id,count(*) FROM products GROUP BY id",
            "WITH p AS (SELECT * FROM products) SELECT * FROM p",
            "SELECT id,upper(name) FROM products",
            "SELECT * FROM products UNION ALL SELECT * FROM products",
            "SELECT * FROM (SELECT * FROM products) p",
        ] {
            assert!(!is_plain_select(&parse_statement(sql).unwrap()), "{sql}");
        }
    }
    #[test]
    fn rejects_multi_statement_and_session_commands() {
        for sql in [
            "SELECT 1; DELETE FROM products",
            "/* hi */ COMMIT",
            "SET default_transaction_read_only = off",
            "BEGIN",
            "",
        ] {
            assert!(parse_statement(sql).is_err());
        }
    }
    #[test]
    fn parameterizes_values_and_uses_composite_keys_and_original_value() {
        let payload = "Monitor Holder'; DROP TABLE products; --";
        let update = plan(
            &snapshot(),
            &[CellEdit {
                row: 0,
                column: 1,
                value: Some(payload.into()),
            }],
        )
        .unwrap()
        .remove(0);
        assert!(update
            .sql
            .starts_with("UPDATE ONLY \"odd\"\"schema\".\"products\""));
        assert!(!update.sql.contains(payload));
        assert!(update.sql.contains(
            "\"id\" = $2\n  AND \"region\" = $3\n  AND \"name\"::text IS NOT DISTINCT FROM $4"
        ));
        assert_eq!(
            update.parameters,
            vec![
                Some(payload.into()),
                Some("1003".into()),
                Some("eu".into()),
                Some("Monitor stand".into())
            ]
        );
    }
    #[test]
    fn null_and_empty_string_are_distinct() {
        for value in [None, Some(String::new())] {
            let updates = plan(
                &snapshot(),
                &[CellEdit {
                    row: 0,
                    column: 1,
                    value: value.clone(),
                }],
            )
            .unwrap();
            assert_eq!(updates[0].parameters[0], value);
        }
    }
    #[test]
    fn validates_keys_bounds_duplicates_and_noops() {
        assert!(plan(
            &snapshot(),
            &[CellEdit {
                row: 0,
                column: 0,
                value: Some("4".into())
            }]
        )
        .is_err());
        assert!(plan(
            &snapshot(),
            &[CellEdit {
                row: 99,
                column: 1,
                value: None
            }]
        )
        .is_err());
        assert!(plan(
            &snapshot(),
            &[CellEdit {
                row: 0,
                column: 1,
                value: Some("Monitor stand".into())
            }]
        )
        .is_err());
        let edit = CellEdit {
            row: 0,
            column: 1,
            value: None,
        };
        assert!(plan(&snapshot(), &[edit.clone(), edit]).is_err());
    }
}
