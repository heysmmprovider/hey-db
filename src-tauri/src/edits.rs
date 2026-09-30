use crate::model::*;
use bytes::BytesMut;
use sqlparser::{
    ast::{Expr, GroupByExpr, Query, SelectItem, SetExpr, Statement, TableFactor},
    dialect::PostgreSqlDialect,
    keywords::Keyword,
    parser::{Parser, ParserError},
    tokenizer::Token,
};
use std::collections::{BTreeMap, HashSet};
use tokio_postgres::types::{Format, IsNull, ToSql, Type};

#[derive(Debug)]
pub enum ParsedStatement {
    Sql(Box<Statement>),
    // sqlparser does not support PostgreSQL DO. Its quoted body is opaque here;
    // PostgreSQL parses the procedural language when the original SQL executes.
    DoBlock,
}

impl ParsedStatement {
    pub fn ast(&self) -> Option<&Statement> {
        match self {
            Self::Sql(ast) => Some(ast),
            Self::DoBlock => None,
        }
    }

    pub fn command(&self) -> String {
        match self {
            Self::DoBlock => "DO".into(),
            Self::Sql(ast) => ast
                .to_string()
                .split_whitespace()
                .next()
                .unwrap_or("SQL")
                .to_owned(),
        }
    }
}

fn parse_do_block(parser: &mut Parser<'_>) -> Result<ParsedStatement, ParserError> {
    fn language(parser: &mut Parser<'_>) -> Result<(), ParserError> {
        let name = parser.next_token();
        match name.token {
            Token::Word(_)
            | Token::SingleQuotedString(_)
            | Token::DollarQuotedString(_)
            | Token::EscapedStringLiteral(_)
            | Token::UnicodeStringLiteral(_) => Ok(()),
            _ => parser.expected("a language name", name),
        }
    }

    let language_first = parser.parse_keyword(Keyword::LANGUAGE);
    if language_first {
        language(parser)?;
    }
    let body = parser.next_token();
    match body.token {
        Token::DollarQuotedString(_)
        | Token::SingleQuotedString(_)
        | Token::EscapedStringLiteral(_)
        | Token::UnicodeStringLiteral(_) => {}
        _ => return parser.expected("a quoted DO body", body),
    }
    if !language_first && parser.parse_keyword(Keyword::LANGUAGE) {
        language(parser)?;
    }
    Ok(ParsedStatement::DoBlock)
}

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

pub fn parse_statement(sql: &str) -> Result<ParsedStatement, String> {
    let mut statements = parse_script(sql)?;
    if statements.len() != 1 {
        return Err(
            "Run one SQL statement at a time. Select the statement you want to execute.".into(),
        );
    }
    Ok(statements.remove(0).1)
}

// Use the parser's statement boundaries, then slice the original SQL. Reprinting
// an AST can change PostgreSQL literals; splitting on ';' breaks quoted bodies.
pub fn parse_script(sql: &str) -> Result<Vec<(&str, ParsedStatement)>, String> {
    if sql.len() > 1024 * 1024 {
        return Err("SQL is limited to 1 MiB per run.".into());
    }
    let dialect = PostgreSqlDialect {};
    let mut parser = Parser::new(&dialect)
        .try_with_sql(sql)
        .map_err(|e| format!("SQL could not be parsed: {e}"))?;
    let mut statements = Vec::new();
    let mut chars = sql.char_indices().peekable();
    let (mut line, mut column) = (1, 1);
    let mut offset = |location: sqlparser::tokenizer::Location| {
        while (line, column) < (location.line, location.column) {
            let Some((_, ch)) = chars.next() else { break };
            if ch == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        chars.peek().map_or(sql.len(), |(index, _)| *index)
    };
    loop {
        while parser.consume_token(&Token::SemiColon) {}
        let first = parser.peek_token();
        if first.token == Token::EOF {
            break;
        }
        if statements.len() == 1000 {
            return Err("Run at most 1,000 statements at a time.".into());
        }
        let start = offset(first.span.start);
        let number = statements.len() + 1;
        let parsed = if parser.parse_keyword(Keyword::DO) {
            parse_do_block(&mut parser)
        } else {
            parser
                .parse_statement()
                .map(|ast| ParsedStatement::Sql(Box::new(ast)))
        };
        let ast = parsed.map_err(|e| {
            format!("Statement {number} could not be parsed: {e}. No statements were executed.")
        })?;
        if let Some(ast) = ast.ast() {
            validate_statement(ast)
                .map_err(|e| format!("Statement {number}: {e} No statements were executed."))?;
        }
        let next = parser.peek_token();
        let end = match next.token {
            Token::EOF => sql.len(),
            Token::SemiColon => offset(next.span.start),
            _ => {
                return Err(format!(
                    "Expected a semicolon after statement {number}. No statements were executed."
                ))
            }
        };
        statements.push((sql[start..end].trim(), ast));
    }
    if statements.is_empty() {
        return Err("Enter at least one SQL statement to run.".into());
    }
    // A run owns its transactions: never leave one open for another tab, schema
    // refresh, or cell edit to accidentally commit. Validate before any writes.
    let mut transaction_start = None;
    for (index, (_, ast)) in statements.iter().enumerate() {
        let number = index + 1;
        match ast.ast() {
            Some(Statement::StartTransaction { .. }) => {
                if transaction_start.is_some() {
                    return Err(format!("Statement {number}: nested transactions are not supported. No statements were executed."));
                }
                transaction_start = Some(number);
            }
            Some(Statement::Commit { .. } | Statement::Rollback { .. }) => {
                transaction_start.take().ok_or_else(|| format!("Statement {number}: COMMIT or ROLLBACK requires a preceding BEGIN in the same run. No statements were executed."))?;
            }
            _ => {}
        }
    }
    if let Some(number) = transaction_start {
        return Err(format!("Transaction started at statement {number} has no COMMIT or ROLLBACK. Include the complete transaction in one run. No statements were executed."));
    }
    Ok(statements)
}

fn validate_statement(stmt: &Statement) -> Result<(), String> {
    match stmt {
        Statement::StartTransaction {
            modifier: None,
            statements,
            exception: None,
            has_end_keyword: false,
            ..
        } if statements.is_empty() => return Ok(()),
        Statement::Commit {
            chain: false,
            modifier: None,
            ..
        }
        | Statement::Rollback {
            chain: false,
            savepoint: None,
        } => return Ok(()),
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::Savepoint { .. }
        | Statement::ReleaseSavepoint { .. } => {
            return Err("Use a complete BEGIN … COMMIT or BEGIN … ROLLBACK block in one run. Savepoints, transaction chaining, and unquoted procedural blocks are not supported yet.".into());
        }
        _ => {}
    }
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
    ]
    .contains(&first.as_str())
    {
        return Err("Session commands, COPY, and CALL are not supported yet. Transactions must use a complete BEGIN … COMMIT or BEGIN … ROLLBACK block in one run.".into());
    }
    Ok(())
}

pub fn can_refresh(stmt: &ParsedStatement) -> bool {
    fn query(value: &Query) -> bool {
        value
            .with
            .as_ref()
            .is_none_or(|with| with.cte_tables.iter().all(|cte| query(&cte.query)))
            && body(&value.body)
    }
    fn body(expr: &SetExpr) -> bool {
        match expr {
            SetExpr::Select(select) => select.into.is_none(),
            SetExpr::Query(value) => query(value),
            SetExpr::SetOperation { left, right, .. } => body(left) && body(right),
            SetExpr::Values(_) | SetExpr::Table(_) => true,
            _ => false,
        }
    }
    matches!(stmt.ast(), Some(Statement::Query(value)) if query(value))
}

pub fn is_plain_select(stmt: &ParsedStatement) -> bool {
    let Some(Statement::Query(query)) = stmt.ast() else {
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
    #[test]
    fn refreshes_queries_without_replaying_explicit_writes() {
        for sql in [
            "SELECT 1 UNION ALL SELECT 2",
            "WITH p AS (SELECT 1) SELECT * FROM p",
            "SELECT * FROM products",
        ] {
            assert!(can_refresh(&parse_statement(sql).unwrap()));
        }
        for sql in [
            "UPDATE products SET stock=stock+1 RETURNING *",
            "SELECT * INTO copied FROM products",
            "WITH p AS (DELETE FROM products RETURNING *) SELECT * FROM p",
        ] {
            assert!(!can_refresh(&parse_statement(sql).unwrap()));
        }
    }

    #[test]
    fn splits_scripts_without_rewriting_postgresql_literals() {
        let sql = "-- → first;\r\nSELECT 'it''s; okay', E'a\\\';b', $$dollar;value$$ AS \"a;b\"; /* outer; /* inner; */ */ SELECT $tag$→;text$tag$; -- done;";
        let statements = super::parse_script(sql).unwrap();
        assert_eq!(statements.len(), 2);
        assert_eq!(
            statements[0].0,
            "SELECT 'it''s; okay', E'a\\\';b', $$dollar;value$$ AS \"a;b\""
        );
        assert_eq!(statements[1].0, "SELECT $tag$→;text$tag$");
        let statements = super::parse_script(";; SELECT '→'; SELECT 'é'; ; -- trailing").unwrap();
        assert_eq!(statements[1].0, "SELECT 'é'");
    }

    #[test]
    fn validates_the_whole_script_before_execution() {
        for sql in [
            "-- comment only",
            ";;",
            "SELECT 'unterminated",
            "SELECT 1; BEGIN; SELECT 2",
            "SELECT 1; SELECT FROM",
            "SELECT 1 END",
        ] {
            assert!(super::parse_script(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn accepts_complete_transactions_and_preserves_function_bodies() {
        for sql in [
            "BEGIN; SELECT 1; COMMIT;",
            "START TRANSACTION ISOLATION LEVEL SERIALIZABLE READ ONLY; SELECT 1; COMMIT AND NO CHAIN;",
            "BEGIN WORK; SELECT 1; ROLLBACK WORK; SELECT 2; BEGIN; COMMIT;",
            "BEGIN; CREATE FUNCTION demo() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN NEW.name := 'x;y'; RETURN NEW; END $$; COMMIT;",
        ] {
            assert!(parse_script(sql).is_ok(), "{sql}");
        }
    }

    #[test]
    fn accepts_postgresql_do_blocks_without_refresh_or_cell_edits() {
        for sql in [
            "DO $$ BEGIN PERFORM 1; END $$",
            "do $migration$ BEGIN RAISE NOTICE 'begin; commit; →'; END $migration$;",
            "DO LANGUAGE plpgsql $$ BEGIN NULL; END $$;",
            "DO $$ BEGIN NULL; END $$ LANGUAGE plpgsql;",
            "DO LANGUAGE \"plpgsql\" 'BEGIN RAISE NOTICE ''it''''s fine;''; END';",
            "DO 'BEGIN NULL; END' LANGUAGE 'plpgsql';",
            r"DO E'BEGIN\nPERFORM 1;\nEND';",
            "DO U&'BEGIN NULL; END';",
        ] {
            let stmt = parse_statement(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
            assert!(matches!(stmt, ParsedStatement::DoBlock));
            assert_eq!(stmt.command(), "DO");
            assert!(!can_refresh(&stmt));
            assert!(!is_plain_select(&stmt));
        }
    }

    #[test]
    fn preserves_do_bodies_and_surrounding_statement_boundaries() {
        let body = "DO /* header; */ $guard$\r\nBEGIN\r\n  IF NOT EXISTS (SELECT 1 FROM migrations WHERE version=2) THEN\r\n    RAISE EXCEPTION 'Import migrations first; →';\r\n  END IF;\r\n  EXECUTE $sql$SELECT 'nested; quote'$sql$;\r\n  -- BEGIN; COMMIT; inside the body\r\nEND $guard$ LANGUAGE plpgsql";
        let sql = format!("-- migration;\r\nBEGIN;\r\n{body};\r\nSELECT 2;\r\nCOMMIT;");
        let statements = parse_script(&sql).unwrap();
        assert_eq!(statements.len(), 4);
        assert_eq!(statements[1].0, body);
        assert!(matches!(statements[1].1, ParsedStatement::DoBlock));
        assert_eq!(statements[2].0, "SELECT 2");
        assert_eq!(statements[3].0, "COMMIT");
    }

    #[test]
    fn rejects_malformed_do_wrappers_and_unsupported_following_commands() {
        for sql in [
            "DO",
            "DO BEGIN NULL; END;",
            "DO 1;",
            "DO LANGUAGE plpgsql;",
            "DO LANGUAGE 1 $$BEGIN NULL; END$$;",
            "DO $$BEGIN NULL; END$$ LANGUAGE;",
            "DO LANGUAGE plpgsql $$BEGIN NULL; END$$ LANGUAGE plpgsql;",
            "DO $$BEGIN NULL; END$$ SELECT 1;",
            "DO $$BEGIN NULL; END$$; SET default_transaction_read_only = off;",
            "DO $$BEGIN NULL; END$$; CALL some_procedure();",
            "DO $$BEGIN NULL; END$$; COMMIT;",
            "BEGIN; DO $$BEGIN NULL; END$$;",
            "DO $tag$BEGIN NULL; END$wrong$;",
            "DO 'unterminated",
        ] {
            assert!(parse_script(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn rejects_incomplete_and_unsupported_transactions_before_execution() {
        for sql in [
            "SELECT 1; BEGIN; SELECT 2;",
            "SELECT 1; COMMIT;",
            "ROLLBACK;",
            "BEGIN; BEGIN; COMMIT; COMMIT;",
            "BEGIN; COMMIT AND CHAIN; ROLLBACK;",
            "BEGIN; ROLLBACK AND CHAIN; COMMIT;",
            "BEGIN; SAVEPOINT s; COMMIT;",
            "BEGIN; ROLLBACK TO SAVEPOINT s; COMMIT;",
            "BEGIN; RELEASE SAVEPOINT s; COMMIT;",
            "BEGIN; SET TRANSACTION READ WRITE; COMMIT;",
        ] {
            let error = parse_script(sql).unwrap_err();
            assert!(
                error.contains("No statements were executed"),
                "{sql}: {error}"
            );
        }
    }

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
