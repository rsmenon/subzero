#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_lossless)]

use std::collections::HashMap;

use crate::snowflake::QueryResult;

use super::config::{Aggregation, ChartConfig, ChartType, SortOrder};
use super::types::{ChartSeries, ColumnType, PreparedChart, XValue};

/// Detect the type of a column by scanning all non-null values.
pub fn detect_column_type(result: &QueryResult, col_index: usize) -> ColumnType {
    let mut non_null_count = 0u32;
    let mut numeric_count = 0u32;
    let mut date_count = 0u32;

    for row in &result.rows {
        if let Some(val) = row.get(col_index) {
            if val.is_null() {
                continue;
            }
            non_null_count += 1;

            if val.is_number() {
                numeric_count += 1;
            } else if let Some(s) = val.as_str()
                && try_parse_date(s).is_some()
            {
                date_count += 1;
            }
        }
    }

    if non_null_count == 0 {
        return ColumnType::Categorical;
    }

    if numeric_count == non_null_count {
        return ColumnType::Numeric;
    }

    // >90% of non-null values parse as dates
    let date_threshold = (non_null_count as f64 * 0.9) as u32;
    if date_count >= date_threshold && date_count > 0 {
        return ColumnType::Date;
    }

    ColumnType::Categorical
}

/// Try to parse a string as a date/datetime, returning epoch milliseconds.
fn try_parse_date(s: &str) -> Option<i64> {
    // Try ISO 8601 / RFC 3339 first (handles timezone variants)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }

    // YYYY-MM-DD HH:MM:SS.fff +HH:MM (Snowflake TIMESTAMP_TZ)
    if let Ok(dt) = chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f %:z") {
        return Some(dt.timestamp_millis());
    }

    // YYYY-MM-DD HH:MM:SS
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp_millis());
    }

    // YYYY-MM-DD HH:MM:SS.fff
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(dt.and_utc().timestamp_millis());
    }

    // YYYY-MM-DD
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = d.and_hms_opt(0, 0, 0)?;
        return Some(dt.and_utc().timestamp_millis());
    }

    None
}

/// Detect types for all columns in a query result.
pub fn detect_all_column_types(result: &QueryResult) -> Vec<(String, ColumnType)> {
    result
        .columns
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), detect_column_type(result, i)))
        .collect()
}

/// Prepare chart data from query result and config.
/// Returns Ok(PreparedChart) or Err(error message).
pub fn prepare(result: &QueryResult, config: &ChartConfig) -> Result<PreparedChart, String> {
    let x_col_name = config.x_column.as_ref().ok_or("No X column selected")?;
    let y_col_name = config.y_column.as_ref().ok_or("No Y column selected")?;

    let x_index = result
        .columns
        .iter()
        .position(|c| c == x_col_name)
        .ok_or_else(|| {
            format!(
                "Column \"{x_col_name}\" not found in query results. Edit chart settings to fix."
            )
        })?;
    let y_index = result
        .columns
        .iter()
        .position(|c| c == y_col_name)
        .ok_or_else(|| {
            format!(
                "Column \"{y_col_name}\" not found in query results. Edit chart settings to fix."
            )
        })?;

    let group_index = if let Some(ref group_col) = config.group_by {
        Some(
            result
                .columns
                .iter()
                .position(|c| c == group_col)
                .ok_or_else(|| {
                    format!(
                        "Group By column \"{group_col}\" not found in query results."
                    )
                })?,
        )
    } else {
        None
    };

    let x_type = detect_column_type(result, x_index);
    let y_type = detect_column_type(result, y_index);

    // For HorizontalBar with categorical Y: swap semantics.
    // Y column values become categorical labels, X column values become bar lengths.
    let is_hbar_categorical =
        config.chart_type == ChartType::HorizontalBar && y_type == ColumnType::Categorical;

    let x_is_date = if is_hbar_categorical {
        false
    } else {
        x_type == ColumnType::Date
    };

    // Collect raw data points grouped by series
    let mut series_map: HashMap<String, Vec<(XValue, f64)>> = HashMap::new();
    // Track first occurrence order for categorical X values
    let mut categorical_order: Vec<String> = Vec::new();

    for row in &result.rows {
        let x_val_raw = row.get(x_index);
        let y_val_raw = row.get(y_index);

        if x_val_raw.is_none_or(serde_json::Value::is_null) || y_val_raw.is_none_or(serde_json::Value::is_null) {
            continue; // skip null rows
        }

        let x_val_raw = x_val_raw.unwrap();
        let y_val_raw = y_val_raw.unwrap();

        // For HorizontalBar with categorical Y: Y column = category labels, X column = numeric values
        let (x_value, y_value) = if is_hbar_categorical {
            let category = format_value(y_val_raw);
            let Some(numeric) = x_val_raw.as_f64() else {
                continue;
            };
            (XValue::Categorical(category), numeric)
        } else {
            let x_value = parse_x_value(x_val_raw, x_type);
            let Some(y_value) = y_val_raw.as_f64() else {
                continue;
            };
            (x_value, y_value)
        };

        // Track categorical order
        if let XValue::Categorical(ref s) = x_value
            && !categorical_order.contains(s)
        {
            categorical_order.push(s.clone());
        }

        let series_label = if let Some(gi) = group_index {
            match row.get(gi) {
                Some(v) if !v.is_null() => match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                },
                _ => "(null)".to_string(),
            }
        } else {
            "default".to_string()
        };

        series_map
            .entry(series_label)
            .or_default()
            .push((x_value, y_value));
    }

    if series_map.is_empty() {
        return Err("No valid data points found".to_string());
    }

    // Apply aggregation and build series
    let mut all_series: Vec<ChartSeries> = Vec::new();

    for (label, points) in &series_map {
        let aggregated = aggregate_points(points, &config.aggregation, &categorical_order)?;
        all_series.push(ChartSeries {
            label: label.clone(),
            points: aggregated,
        });
    }

    // Sort series by label for consistent ordering
    all_series.sort_by(|a, b| a.label.cmp(&b.label));

    // Sort each series by the configured sort order
    for series in &mut all_series {
        sort_series_points(&mut series.points, &config.sort_order);
    }

    // Compute bounds
    let (x_min, x_max, mut y_min, y_max) = compute_bounds(&all_series);

    // Only clamp y_min to 0 for bar chart types, not line charts
    match config.chart_type {
        ChartType::VerticalBar | ChartType::HorizontalBar => {
            y_min = y_min.min(0.0);
        }
        ChartType::Line => {}
    }

    Ok(PreparedChart {
        chart_type: config.chart_type.clone(),
        x_label: x_col_name.clone(),
        y_label: y_col_name.clone(),
        x_is_date,
        series: all_series,
        x_min,
        x_max,
        y_min,
        y_max,
    })
}

fn parse_x_value(val: &serde_json::Value, col_type: ColumnType) -> XValue {
    match col_type {
        ColumnType::Numeric => XValue::Numeric(val.as_f64().unwrap_or(0.0)),
        ColumnType::Date => {
            if let Some(s) = val.as_str()
                && let Some(ms) = try_parse_date(s)
            {
                return XValue::Date(ms);
            }
            // Fallback: treat as categorical
            XValue::Categorical(format_value(val))
        }
        ColumnType::Categorical => XValue::Categorical(format_value(val)),
    }
}

fn format_value(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "NULL".to_string(),
        other => other.to_string(),
    }
}

fn sort_series_points(points: &mut [(XValue, f64)], sort_order: &SortOrder) {
    match sort_order {
        SortOrder::None => {} // preserve insertion/categorical order
        SortOrder::XAscending => {
            points.sort_by(|a, b| {
                let ax = x_sort_key(&a.0);
                let bx = x_sort_key(&b.0);
                ax.partial_cmp(&bx).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        SortOrder::XDescending => {
            points.sort_by(|a, b| {
                let ax = x_sort_key(&a.0);
                let bx = x_sort_key(&b.0);
                bx.partial_cmp(&ax).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        SortOrder::YAscending => {
            points.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        }
        SortOrder::YDescending => {
            points.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        }
    }
}

fn x_sort_key(x: &XValue) -> f64 {
    match x {
        XValue::Numeric(v) => *v,
        XValue::Date(ms) => *ms as f64,
        XValue::Categorical(_) => 0.0, // categorical order preserved by insertion order
    }
}

/// Aggregate points by X value for a single series.
fn aggregate_points(
    points: &[(XValue, f64)],
    aggregation: &Aggregation,
    categorical_order: &[String],
) -> Result<Vec<(XValue, f64)>, String> {
    // Group by X value string key
    let mut groups: HashMap<String, Vec<(XValue, f64)>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for (x, y) in points {
        let key = x_key(x);
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push((x.clone(), *y));
    }

    // Use categorical_order if available for ordering
    if !categorical_order.is_empty() {
        order = categorical_order
            .iter()
            .filter(|k| groups.contains_key(*k))
            .cloned()
            .collect();
    }

    let mut result = Vec::new();
    for key in &order {
        let group = &groups[key];
        if group.len() > 1 && *aggregation == Aggregation::None {
            return Err("Duplicate X values found. Select an aggregation method.".to_string());
        }

        let x_val = group[0].0.clone();
        let y_val = match aggregation {
            Aggregation::None => group[0].1,
            Aggregation::Sum => group.iter().map(|(_, y)| *y).sum(),
            Aggregation::Min => group.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min),
            Aggregation::Max => group
                .iter()
                .map(|(_, y)| *y)
                .fold(f64::NEG_INFINITY, f64::max),
            Aggregation::Count => group.len() as f64,
            Aggregation::Average => {
                let sum: f64 = group.iter().map(|(_, y)| *y).sum();
                sum / group.len() as f64
            }
            Aggregation::Median => {
                let mut vals: Vec<f64> = group.iter().map(|(_, y)| *y).collect();
                vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let mid = vals.len() / 2;
                if vals.len().is_multiple_of(2) {
                    f64::midpoint(vals[mid - 1], vals[mid])
                } else {
                    vals[mid]
                }
            }
        };

        result.push((x_val, y_val));
    }

    Ok(result)
}

fn x_key(x: &XValue) -> String {
    match x {
        XValue::Numeric(v) => format!("{v}"),
        XValue::Date(ms) => format!("{ms}"),
        XValue::Categorical(s) => s.clone(),
    }
}

fn compute_bounds(series: &[ChartSeries]) -> (f64, f64, f64, f64) {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;

    for s in series {
        for (x, y) in &s.points {
            let xv = match x {
                XValue::Numeric(v) => *v,
                XValue::Date(ms) => *ms as f64,
                XValue::Categorical(_) => continue, // skip for categorical
            };
            x_min = x_min.min(xv);
            x_max = x_max.max(xv);
            y_min = y_min.min(*y);
            y_max = y_max.max(*y);
        }
    }

    // For categorical X, set dummy bounds
    if x_min == f64::INFINITY {
        x_min = 0.0;
        x_max = 1.0;
    }

    // Ensure y range is valid
    if y_min == f64::INFINITY {
        y_min = 0.0;
        y_max = 1.0;
    }
    if (y_max - y_min).abs() < f64::EPSILON {
        y_min -= 1.0;
        y_max += 1.0;
    }

    (x_min, x_max, y_min, y_max)
}
