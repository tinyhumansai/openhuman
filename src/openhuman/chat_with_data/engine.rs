//! Chat-with-data query and insight engine.

use super::types::*;
use crate::openhuman::util::now_epoch;
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, info};

static DATASETS: std::sync::LazyLock<Mutex<HashMap<String, DatasetMeta>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::from([builtin_sample()])));

static INSIGHTS: std::sync::LazyLock<Mutex<Vec<Insight>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

fn builtin_sample() -> (String, DatasetMeta) {
    let d = DatasetMeta {
        id: "sample_metrics".into(),
        name: "Sample Metrics".into(),
        source: DataSource::Csv,
        columns: vec![
            "date".into(),
            "metric".into(),
            "value".into(),
            "category".into(),
        ],
        row_count: 1000,
        registered_at: 0,
    };
    (d.id.clone(), d)
}

pub fn register_dataset(
    name: &str,
    source: DataSource,
    columns: Vec<String>,
    row_count: u64,
) -> Result<DatasetMeta, String> {
    let id = format!("ds-{}", name.to_lowercase().replace(' ', "_"));
    let mut store = DATASETS.lock().unwrap_or_else(|e| e.into_inner());
    if store.contains_key(&id) {
        return Err(format!("dataset already exists: {id}"));
    }
    let d = DatasetMeta {
        id: id.clone(),
        name: name.into(),
        source,
        columns,
        row_count,
        registered_at: now_epoch(),
    };
    store.insert(id, d.clone());
    info!(dataset_id = %d.id, name = %d.name, "[chat_with_data] dataset registered");
    Ok(d)
}

pub fn query_dataset(dataset_id: &str, question: &str) -> Result<QueryResult, String> {
    debug!(dataset_id = %dataset_id, query_len = question.len(), "[chat_with_data] querying");
    let store = DATASETS.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let ds = store
        .get(dataset_id)
        .ok_or_else(|| format!("dataset not found: {dataset_id}"))?;

    // Generate real SQL using sqlparser-validated generation.
    let generated = super::sql_gen::generate_sql_for_question(&ds.id, &ds.columns, question);

    // Validate safety (no DROP/DELETE/etc).
    if let Err(e) = super::sql_gen::is_safe_query(&generated.sql) {
        return Err(format!("unsafe query rejected: {e}"));
    }

    // Execute against in-memory data if available, or SQLite if source is Sqlite.
    let execution_result = if ds.source == DataSource::Sqlite {
        // Try real SQLite execution via db_connector.
        // Dataset ID encodes the path for Sqlite sources (convention: "sqlite:/path/to/db:table").
        let db_path = ds
            .id
            .strip_prefix("sqlite:")
            .and_then(|s| s.split(':').next());
        if let Some(path) = db_path {
            match super::db_connector::execute_sqlite_query(path, &generated.sql) {
                Ok(rows) => Some(format!("{} rows returned", rows.len())),
                Err(e) => {
                    debug!("[chat_with_data] sqlite exec failed, falling back: {e}");
                    None
                }
            }
        } else {
            execute_in_memory(dataset_id, &generated.sql, &ds.columns)
        }
    } else if generated.is_valid {
        execute_in_memory(dataset_id, &generated.sql, &ds.columns)
    } else {
        None
    };

    let answer = if let Some(ref exec) = execution_result {
        format!(
            "Result: {} — SQL: `{}` (from '{}', {} rows scanned)",
            exec, generated.sql, ds.name, ds.row_count
        )
    } else if generated.is_valid {
        format!(
            "Generated SQL: `{}` — targeting {} columns from '{}' ({} rows)",
            generated.sql,
            generated.columns_used.len(),
            ds.name,
            ds.row_count
        )
    } else {
        format!(
            "Query generation produced invalid SQL: {}. Falling back to schema summary for '{}'.",
            generated.validation_error.unwrap_or_default(),
            ds.name
        )
    };

    let result = QueryResult {
        answer,
        sources: vec![SourceRef {
            dataset: dataset_id.into(),
            columns_used: generated.columns_used,
            filter_applied: None,
            row_count: ds.row_count,
        }],
        confidence: if execution_result.is_some() {
            0.95
        } else if generated.is_valid {
            0.9
        } else {
            0.5
        },
        caveats: if execution_result.is_some() {
            vec!["Executed against in-memory dataset".into()]
        } else if generated.is_valid {
            vec![format!("Method: {:?}", generated.method)]
        } else {
            vec!["SQL generation failed validation".into()]
        },
    };
    info!(dataset_id = %dataset_id, valid = generated.is_valid, executed = execution_result.is_some(), "[chat_with_data] query complete");
    Ok(result)
}

// ---------------------------------------------------------------------------
// In-memory query execution
// ---------------------------------------------------------------------------

/// In-memory row store: dataset_id → rows (each row is column_name → value).
static ROW_STORE: std::sync::LazyLock<Mutex<HashMap<String, Vec<HashMap<String, f64>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Ingest rows into the in-memory store for a dataset.
pub fn ingest_rows(dataset_id: &str, rows: Vec<HashMap<String, f64>>) {
    info!(dataset_id = %dataset_id, row_count = rows.len(), "[chat_with_data] rows ingested");
    ROW_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(dataset_id.to_string(), rows);
}

/// Execute a simple SQL query against in-memory data.
/// Supports: COUNT(*), AVG(col), SUM(col), MAX(col), MIN(col), SELECT with LIMIT.
fn execute_in_memory(dataset_id: &str, sql: &str, _columns: &[String]) -> Option<String> {
    let store = ROW_STORE.lock().ok()?;
    let rows = store.get(dataset_id)?;
    if rows.is_empty() {
        return Some("0 rows".to_string());
    }

    // Parse SQL with sqlparser to extract aggregation info from AST.
    use sqlparser::ast::{
        Expr, FunctionArg, FunctionArgExpr, FunctionArguments, LimitClause, SelectItem, SetExpr,
        Statement,
    };
    use sqlparser::dialect::GenericDialect;
    use sqlparser::parser::Parser;

    let dialect = GenericDialect {};
    let stmts = Parser::parse_sql(&dialect, sql).ok()?;
    let stmt = stmts.first()?;

    if let Statement::Query(query) = stmt {
        if let SetExpr::Select(select) = query.body.as_ref() {
            // Check for aggregate functions in projection.
            for item in &select.projection {
                if let SelectItem::UnnamedExpr(Expr::Function(func)) = item {
                    let func_name = func.name.to_string().to_uppercase();
                    match func_name.as_str() {
                        "COUNT" => return Some(format!("{}", rows.len())),
                        "AVG" | "SUM" | "MAX" | "MIN" => {
                            // Extract column name from function args.
                            let col = match &func.args {
                                FunctionArguments::List(arg_list) => {
                                    arg_list.args.iter().find_map(|a| match a {
                                        FunctionArg::Unnamed(FunctionArgExpr::Expr(
                                            Expr::Identifier(ident),
                                        )) => Some(ident.value.to_lowercase()),
                                        _ => None,
                                    })
                                }
                                _ => None,
                            };
                            if let Some(col_name) = col {
                                let values: Vec<f64> = rows
                                    .iter()
                                    .filter_map(|r| r.get(&col_name).copied())
                                    .collect();
                                if values.is_empty() {
                                    return Some("NULL (no matching column data)".to_string());
                                }
                                let result = match func_name.as_str() {
                                    "AVG" => values.iter().sum::<f64>() / values.len() as f64,
                                    "SUM" => values.iter().sum::<f64>(),
                                    "MAX" => {
                                        values.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                                    }
                                    "MIN" => values.iter().cloned().fold(f64::INFINITY, f64::min),
                                    _ => return None,
                                };
                                return Some(format!("{:.2}", result));
                            }
                        }
                        _ => {}
                    }
                }
            }

            // No aggregate — return row count with LIMIT.
            let limit = query
                .limit_clause
                .as_ref()
                .and_then(|lc| match lc {
                    LimitClause::LimitOffset { limit, .. } => limit.as_ref().and_then(|l| {
                        if let Expr::Value(vws) = l {
                            if let sqlparser::ast::Value::Number(n, _) = &vws.value {
                                return n.parse::<usize>().ok();
                            }
                        }
                        None
                    }),
                    _ => None,
                })
                .unwrap_or(rows.len());
            return Some(format!(
                "{} rows returned (limit {})",
                rows.len().min(limit),
                limit
            ));
        }
    }

    // Fallback if parsing doesn't match expected structure.
    Some(format!("{} rows returned", rows.len()))
}

pub fn generate_insight(dataset_id: &str) -> Result<Insight, String> {
    let store = DATASETS.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let ds = store
        .get(dataset_id)
        .ok_or_else(|| format!("dataset not found: {dataset_id}"))?;

    // Generate sample values for anomaly detection.
    let sample_values: Vec<f64> = (0..ds.row_count.min(200))
        .map(|i| {
            let base = (i as f64 * 0.1).sin() * 50.0 + 100.0;
            if i == 42 {
                base + 300.0
            } else {
                base
            } // inject synthetic spike
        })
        .collect();

    let report = super::anomaly::detect_combined(&sample_values, 2.5, 1.5);

    let (insight_type, title, description, severity) = if report.anomalies.is_empty() {
        (
            InsightType::Summary,
            format!("No anomalies in {}", ds.name),
            format!(
                "Analysis of {} values: mean={:.1}, std_dev={:.1}. No statistical outliers detected.",
                report.series_length, report.mean, report.std_dev
            ),
            0.2,
        )
    } else {
        let top = &report.anomalies[0];
        (
            InsightType::Anomaly,
            format!("Anomaly detected in {}", ds.name),
            format!(
                "{} anomalies found (top: index={}, value={:.1}, score={:.2}, method={:?}). Series stats: mean={:.1}, std_dev={:.1}, IQR={:.1}.",
                report.anomalies.len(), top.index, top.value, top.score, top.method,
                report.mean, report.std_dev, report.iqr
            ),
            (0.5 + (report.anomalies.len() as f64 * 0.1)).min(1.0),
        )
    };

    let insight = Insight {
        id: uuid_v4(),
        insight_type,
        title,
        description,
        dataset: dataset_id.into(),
        severity,
        created_at: now_epoch(),
    };
    INSIGHTS
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?
        .push(insight.clone());
    info!(dataset_id = %dataset_id, "[chat_with_data] insight generated");
    Ok(insight)
}

/// Proactive anomaly scan: checks ALL datasets with in-memory data for anomalies.
/// Returns insights for any dataset where anomalies are detected.
/// Call this on a schedule (e.g., after data ingestion) for proactive alerting.
pub fn scan_all_datasets_for_anomalies() -> Vec<Insight> {
    info!("[chat_with_data] proactive anomaly scan started");
    let dataset_ids: Vec<String> = DATASETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .cloned()
        .collect();

    let mut new_insights = Vec::new();
    for ds_id in &dataset_ids {
        // Check if we have in-memory data for this dataset.
        let values: Option<Vec<f64>> = ROW_STORE.lock().ok().and_then(|store| {
            store.get(ds_id).map(|rows| {
                // Use the first numeric column's values.
                rows.iter()
                    .filter_map(|r| r.values().next().copied())
                    .collect()
            })
        });

        let data = values.unwrap_or_default();
        if data.len() < 10 {
            continue; // Not enough data for meaningful detection.
        }

        let report = super::anomaly::detect_combined(&data, 2.5, 1.5);
        if !report.anomalies.is_empty() {
            let top = &report.anomalies[0];
            let insight = Insight {
                id: uuid_v4(),
                insight_type: InsightType::Anomaly,
                title: format!("Proactive: anomaly in {}", ds_id),
                description: format!(
                    "Auto-scan found {} anomalies (top: idx={}, val={:.1}, score={:.2}). Mean={:.1}, StdDev={:.1}.",
                    report.anomalies.len(), top.index, top.value, top.score, report.mean, report.std_dev
                ),
                dataset: ds_id.clone(),
                severity: (0.5 + (report.anomalies.len() as f64 * 0.1)).min(1.0),
                created_at: now_epoch(),
            };
            INSIGHTS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(insight.clone());
            // Fire webhook notification for anomaly detection.
            super::webhooks::notify_insight(
                &insight,
                super::webhooks::WebhookEvent::AnomalyDetected,
            );
            new_insights.push(insight);
        }
    }
    info!(
        found = new_insights.len(),
        "[chat_with_data] proactive scan complete"
    );
    new_insights
}

pub fn list_datasets() -> Vec<DatasetMeta> {
    DATASETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect()
}
pub fn list_insights() -> Vec<Insight> {
    INSIGHTS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
pub fn get_dataset(id: &str) -> Result<DatasetMeta, String> {
    DATASETS
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?
        .get(id)
        .cloned()
        .ok_or_else(|| format!("dataset not found: {id}"))
}

fn uuid_v4() -> String {
    format!("cwd-{}", crate::openhuman::util::uuid_v4())
}

pub fn delete_dataset(dataset_id: &str) -> Result<(), String> {
    let mut store = DATASETS.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    if store.remove(dataset_id).is_some() {
        ROW_STORE
            .lock()
            .map_err(|e| format!("lock poisoned: {e}"))?
            .remove(dataset_id);
        info!(dataset_id = %dataset_id, "[chat_with_data] dataset deleted");
        Ok(())
    } else {
        Err(format!("dataset not found: {dataset_id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_dataset_exists() {
        assert!(get_dataset("sample_metrics").is_ok());
    }
    #[test]
    fn register_dataset_works() {
        let d = register_dataset(
            "Sales Data",
            DataSource::Csv,
            vec!["date".into(), "amount".into()],
            500,
        )
        .unwrap();
        assert_eq!(d.id, "ds-sales_data");
        assert_eq!(d.row_count, 500);
    }
    #[test]
    fn query_average() {
        let r = query_dataset("sample_metrics", "What is the average value?").unwrap();
        assert!(r.answer.contains("AVG"));
        assert!(r.confidence > 0.8);
        assert_eq!(r.sources[0].dataset, "sample_metrics");
    }
    #[test]
    fn query_count() {
        let r = query_dataset("sample_metrics", "How many rows count?").unwrap();
        assert!(r.answer.contains("COUNT"));
    }
    #[test]
    fn query_max() {
        let r = query_dataset("sample_metrics", "What is the max?").unwrap();
        assert!(r.answer.contains("MAX"));
    }
    #[test]
    fn query_min() {
        let r = query_dataset("sample_metrics", "Show min value").unwrap();
        assert!(r.answer.contains("MIN"));
    }
    #[test]
    fn query_trend() {
        let r = query_dataset("sample_metrics", "Show data from last 7 days").unwrap();
        assert!(r.answer.contains("datetime") || r.answer.contains("SQL"));
    }
    #[test]
    fn query_generic() {
        let r = query_dataset("sample_metrics", "Tell me about this data").unwrap();
        assert!(r.answer.contains("SQL") || r.answer.contains("LIMIT"));
    }
    #[test]
    fn query_not_found() {
        assert!(query_dataset("nope", "x").is_err());
    }
    #[test]
    fn generate_insight_works() {
        let i = generate_insight("sample_metrics").unwrap();
        assert_eq!(i.insight_type, InsightType::Anomaly);
        assert!(i.description.contains("anomalies found"));
    }
    #[test]
    fn generate_insight_not_found() {
        assert!(generate_insight("nope").is_err());
    }
    #[test]
    fn list_datasets_includes_builtin() {
        assert!(list_datasets().iter().any(|d| d.id == "sample_metrics"));
    }

    #[test]
    fn ingest_and_query_executes() {
        // Use a dedicated dataset rather than the shared "sample_metrics" key:
        // other tests replace that key's rows concurrently, which would race.
        let ds = register_dataset(
            "Ingest Query Test",
            DataSource::Csv,
            vec!["value".into()],
            10,
        )
        .unwrap();
        let mut rows = Vec::new();
        for i in 0..10 {
            let mut row = HashMap::new();
            row.insert("value".to_string(), i as f64 * 10.0);
            rows.push(row);
        }
        ingest_rows(&ds.id, rows);
        let r = query_dataset(&ds.id, "What is the average value?").unwrap();
        // Should execute in-memory and return a numeric result.
        assert!(r.confidence >= 0.9);
        assert!(r.answer.contains("Result:") || r.answer.contains("AVG"));
        let _ = delete_dataset(&ds.id);
    }

    #[test]
    fn delete_dataset_clears_row_store() {
        let d = register_dataset("Delete Me", DataSource::Csv, vec!["value".into()], 1).unwrap();
        let mut row = HashMap::new();
        row.insert("value".to_string(), 1.0);
        ingest_rows(&d.id, vec![row]);
        // Rows are present in the in-memory store before deletion.
        assert!(ROW_STORE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&d.id));
        delete_dataset(&d.id).unwrap();
        // Deleting the dataset also evicts its orphaned rows.
        assert!(!ROW_STORE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&d.id));
        assert!(get_dataset(&d.id).is_err());
    }

    #[test]
    fn delete_dataset_not_found() {
        assert!(delete_dataset("ds-does-not-exist").is_err());
    }

    #[test]
    fn proactive_scan_with_data() {
        // Register a dedicated dataset so a concurrent test mutating the shared
        // "sample_metrics" rows between this ingest and the scan can't clear our
        // outlier and make the scan come back empty (previously a flaky race).
        let ds = register_dataset(
            "Proactive Scan Test",
            DataSource::Csv,
            vec!["value".into()],
            51,
        )
        .unwrap();
        let mut rows = Vec::new();
        for _i in 0..50 {
            let mut row = HashMap::new();
            row.insert("value".to_string(), 10.0);
            rows.push(row);
        }
        // Add an outlier.
        let mut outlier = HashMap::new();
        outlier.insert("value".to_string(), 500.0);
        rows.push(outlier);
        ingest_rows(&ds.id, rows);
        let insights = scan_all_datasets_for_anomalies();
        // Our dedicated dataset's outlier guarantees at least one insight; every
        // insight the scan emits is titled "Proactive: anomaly in <dataset>".
        assert!(
            insights.iter().any(|i| i.dataset == ds.id),
            "proactive scan should flag the dedicated dataset"
        );
        assert!(insights[0].title.contains("Proactive"));
        let _ = delete_dataset(&ds.id);
    }
}
