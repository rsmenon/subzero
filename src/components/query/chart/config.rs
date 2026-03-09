use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartType {
    Line,
    VerticalBar,
    HorizontalBar,
}

impl ChartType {
    pub fn label(&self) -> &'static str {
        match self {
            ChartType::Line => "Line",
            ChartType::VerticalBar => "V. Bar",
            ChartType::HorizontalBar => "H. Bar",
        }
    }

    pub fn all() -> &'static [ChartType] {
        &[
            ChartType::Line,
            ChartType::VerticalBar,
            ChartType::HorizontalBar,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aggregation {
    None,
    Sum,
    Min,
    Max,
    Count,
    Median,
    Average,
}

impl Aggregation {
    pub fn label(&self) -> &'static str {
        match self {
            Aggregation::None => "NONE",
            Aggregation::Sum => "SUM",
            Aggregation::Min => "MIN",
            Aggregation::Max => "MAX",
            Aggregation::Count => "COUNT",
            Aggregation::Median => "MEDIAN",
            Aggregation::Average => "AVG",
        }
    }

    pub fn all() -> &'static [Aggregation] {
        &[
            Aggregation::None,
            Aggregation::Sum,
            Aggregation::Min,
            Aggregation::Max,
            Aggregation::Count,
            Aggregation::Median,
            Aggregation::Average,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortOrder {
    None,
    XAscending,
    XDescending,
    YAscending,
    YDescending,
}

impl SortOrder {
    pub fn label(&self) -> &'static str {
        match self {
            SortOrder::None => "None",
            SortOrder::XAscending => "X Ascending",
            SortOrder::XDescending => "X Descending",
            SortOrder::YAscending => "Y Ascending",
            SortOrder::YDescending => "Y Descending",
        }
    }

    pub fn all() -> &'static [SortOrder] {
        &[
            SortOrder::None,
            SortOrder::XAscending,
            SortOrder::XDescending,
            SortOrder::YAscending,
            SortOrder::YDescending,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartConfig {
    pub chart_type: ChartType,
    pub x_column: Option<String>,
    pub y_column: Option<String>,
    pub sort_order: SortOrder,
    pub group_by: Option<String>,
    pub aggregation: Aggregation,
}

impl Default for ChartConfig {
    fn default() -> Self {
        Self {
            chart_type: ChartType::VerticalBar,
            x_column: None,
            y_column: None,
            sort_order: SortOrder::None,
            group_by: None,
            aggregation: Aggregation::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartTab {
    pub label: String,
    pub config: ChartConfig,
    #[serde(skip, default = "default_settings_hidden")]
    pub settings_hidden: bool,
}

fn default_settings_hidden() -> bool {
    true
}

impl ChartTab {
    pub fn new(index: usize) -> Self {
        Self {
            label: format!("Chart {index}"),
            config: ChartConfig::default(),
            settings_hidden: true,
        }
    }
}
