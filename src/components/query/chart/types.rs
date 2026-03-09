use super::config::ChartType;

/// The X-axis value type determines spacing and label formatting
#[derive(Debug, Clone)]
pub enum XValue {
    Numeric(f64),
    Date(i64), // epoch milliseconds
    Categorical(String),
}

/// A single data series (one line, or one group in a grouped bar chart)
#[derive(Debug, Clone)]
pub struct ChartSeries {
    pub label: String,
    pub points: Vec<(XValue, f64)>,
}

/// Ready-to-render chart data
#[derive(Debug, Clone)]
pub struct PreparedChart {
    pub chart_type: ChartType,
    #[allow(dead_code)]
    pub x_label: String,
    #[allow(dead_code)]
    pub y_label: String,
    pub x_is_date: bool,
    pub series: Vec<ChartSeries>,
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

/// Column type detected from query results
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    Numeric,
    Date,
    Categorical,
}

impl ColumnType {
    pub fn short_label(self) -> &'static str {
        match self {
            ColumnType::Numeric => "num",
            ColumnType::Date => "date",
            ColumnType::Categorical => "cat",
        }
    }
}
