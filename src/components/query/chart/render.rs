#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss, clippy::cast_lossless, clippy::too_many_lines)]

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Bar, BarChart, BarGroup, Block, Borders, Paragraph,
    canvas::{Canvas, Line as CanvasLine},
};

use crate::theme;

use super::types::{ChartSeries, PreparedChart, XValue};

/// Series colors for multi-series charts
fn series_colors() -> [ratatui::style::Color; 6] {
    [
        theme::accent(),
        theme::green(),
        theme::yellow(),
        theme::red(),
        theme::chart_purple(),
        theme::chart_mint(),
    ]
}

fn series_color(index: usize) -> ratatui::style::Color {
    let colors = series_colors();
    colors[index % colors.len()]
}

/// Format a Y value for display (K/M/B suffixes for large numbers)
#[allow(clippy::float_cmp)]
fn format_y_value(val: f64) -> String {
    let abs = val.abs();
    if abs >= 1_000_000_000.0 {
        format!("{:.1}B", val / 1_000_000_000.0)
    } else if abs >= 1_000_000.0 {
        format!("{:.1}M", val / 1_000_000.0)
    } else if abs >= 1_000.0 {
        format!("{:.1}K", val / 1_000.0)
    } else if abs == abs.floor() {
        format!("{val:.0}")
    } else {
        format!("{val:.1}")
    }
}

/// Format a date epoch ms value based on the range span
fn format_date_label(ms: f64, range_ms: f64) -> String {
    let secs = (ms / 1000.0) as i64;
    let dt = chrono::DateTime::from_timestamp(secs, 0);
    match dt {
        Some(dt) => {
            let two_days = 2.0 * 86400.0 * 1000.0;
            let ninety_days = 90.0 * 86400.0 * 1000.0;
            let two_years = 730.0 * 86400.0 * 1000.0;

            if range_ms < two_days {
                dt.format("%H:%M").to_string()
            } else if range_ms < ninety_days {
                dt.format("%b %d").to_string()
            } else if range_ms < two_years {
                dt.format("%b %Y").to_string()
            } else {
                dt.format("%Y").to_string()
            }
        }
        None => format!("{ms:.0}"),
    }
}

/// Render a prepared chart into the given area
pub fn render_chart(frame: &mut Frame, area: Rect, prepared: &PreparedChart) {
    if area.height < 3 || area.width < 10 {
        return;
    }

    match prepared.chart_type {
        super::config::ChartType::VerticalBar => render_vertical_bar(frame, area, prepared),
        super::config::ChartType::HorizontalBar => render_horizontal_bar(frame, area, prepared),
        super::config::ChartType::Line => render_line_chart(frame, area, prepared),
    }

    // Render legend if multiple series
    if prepared.series.len() > 1 {
        render_legend(frame, area, &prepared.series);
    }
}

fn render_vertical_bar(frame: &mut Frame, area: Rect, prepared: &PreparedChart) {
    if prepared.series.is_empty() {
        return;
    }

    let is_grouped = prepared.series.len() > 1;

    if is_grouped {
        render_grouped_vertical_bar(frame, area, prepared);
    } else {
        render_single_vertical_bar(frame, area, prepared);
    }
}

fn render_single_vertical_bar(frame: &mut Frame, area: Rect, prepared: &PreparedChart) {
    let series = &prepared.series[0];
    // Store owned strings so we can borrow them for BarChart
    let labels: Vec<String> = series.points.iter().map(|(x, _)| x_label(x)).collect();
    let values: Vec<u64> = series
        .points
        .iter()
        .map(|(_, y)| (*y).max(0.0) as u64)
        .collect();

    if labels.is_empty() {
        return;
    }

    let max_bars = (area.width as usize / 6).max(1);
    let data: Vec<(&str, u64)> = labels
        .iter()
        .zip(values.iter())
        .take(max_bars)
        .map(|(l, v)| (l.as_str(), *v))
        .collect();

    let bar_width = ((area.width as usize).saturating_sub(2)) / data.len().max(1);
    let bar_width = bar_width.clamp(3, 12) as u16;

    let chart = BarChart::default()
        .data(&data)
        .bar_width(bar_width)
        .bar_gap(1)
        .bar_style(Style::default().fg(theme::accent()))
        .value_style(
            Style::default()
                .fg(theme::fg_bright())
                .add_modifier(Modifier::BOLD),
        )
        .label_style(Style::default().fg(theme::fg_dim()))
        .style(Style::default().bg(theme::bg_surface()));

    frame.render_widget(chart, area);
}

fn render_grouped_vertical_bar(frame: &mut Frame, area: Rect, prepared: &PreparedChart) {
    // Collect all unique X values in order from first series
    let x_values: Vec<String> = if prepared.series.is_empty() {
        return;
    } else {
        prepared.series[0]
            .points
            .iter()
            .map(|(x, _)| x_label(x))
            .collect()
    };

    let max_groups = (area.width as usize / (prepared.series.len() * 4 + 2)).max(1);

    let mut groups: Vec<BarGroup> = Vec::new();
    for x_val in x_values.iter().take(max_groups) {
        let mut bars: Vec<Bar> = Vec::new();
        for (si, series) in prepared.series.iter().enumerate() {
            let y_val = series
                .points
                .iter()
                .find(|(x, _)| x_label(x) == *x_val)
                .map_or(0.0, |(_, y)| *y);

            bars.push(
                Bar::default()
                    .value(y_val.max(0.0) as u64)
                    .style(Style::default().fg(series_color(si)))
                    .value_style(Style::default().fg(theme::fg_bright())),
            );
        }
        groups.push(
            BarGroup::default()
                .label(Line::from(Span::styled(
                    x_val.clone(),
                    Style::default().fg(theme::fg_dim()),
                )))
                .bars(&bars),
        );
    }

    let bar_width = 3u16;
    let mut chart = BarChart::default()
        .bar_width(bar_width)
        .bar_gap(1)
        .group_gap(2)
        .style(Style::default().bg(theme::bg_surface()));

    for group in &groups {
        chart = chart.data(group.clone());
    }

    frame.render_widget(chart, area);
}

fn render_horizontal_bar(frame: &mut Frame, area: Rect, prepared: &PreparedChart) {
    if prepared.series.is_empty() || prepared.series[0].points.is_empty() {
        return;
    }

    let series = &prepared.series[0];
    let labels: Vec<String> = series.points.iter().map(|(x, _)| x_label(x)).collect();
    let values: Vec<u64> = series
        .points
        .iter()
        .map(|(_, y)| (*y).max(0.0) as u64)
        .collect();

    let max_bars = (area.height as usize).max(1);
    let data: Vec<(&str, u64)> = labels
        .iter()
        .zip(values.iter())
        .take(max_bars)
        .map(|(l, v)| (l.as_str(), *v))
        .collect();

    let chart = BarChart::default()
        .data(&data)
        .bar_width(1)
        .bar_gap(1)
        .direction(ratatui::layout::Direction::Horizontal)
        .bar_style(Style::default().fg(theme::accent()))
        .value_style(
            Style::default()
                .fg(theme::fg_bright())
                .add_modifier(Modifier::BOLD),
        )
        .label_style(Style::default().fg(theme::fg_dim()))
        .style(Style::default().bg(theme::bg_surface()));

    frame.render_widget(chart, area);
}

fn render_line_chart(frame: &mut Frame, area: Rect, prepared: &PreparedChart) {
    if prepared.series.is_empty() {
        return;
    }

    let x_min = prepared.x_min;
    let x_max = prepared.x_max;
    let y_min = prepared.y_min;
    let y_max = prepared.y_max;

    // Reserve space for Y axis labels and X axis labels
    let y_label_width = 8u16;
    let x_label_height = 2u16;

    if area.width <= y_label_width + 2 || area.height <= x_label_height + 2 {
        return;
    }

    let chart_area = Rect {
        x: area.x + y_label_width,
        y: area.y,
        width: area.width - y_label_width,
        height: area.height - x_label_height,
    };

    let range_ms = x_max - x_min;

    let canvas = Canvas::default()
        .block(Block::default().style(Style::default().bg(theme::bg_surface())))
        .x_bounds([x_min, x_max])
        .y_bounds([y_min, y_max])
        .paint(move |ctx| {
            // Draw grid lines (horizontal)
            let y_ticks = 5;
            let y_step = (y_max - y_min) / y_ticks as f64;
            for i in 0..=y_ticks {
                let y = y_min + i as f64 * y_step;
                ctx.draw(&CanvasLine {
                    x1: x_min,
                    y1: y,
                    x2: x_max,
                    y2: y,
                    color: theme::fg_dim(),
                });
            }

            // Draw data lines
            for (si, series) in prepared.series.iter().enumerate() {
                let color = series_color(si);
                for pair in series.points.windows(2) {
                    let (x1, y1) = x_y_values(&pair[0]);
                    let (x2, y2) = x_y_values(&pair[1]);
                    ctx.draw(&CanvasLine {
                        x1,
                        y1,
                        x2,
                        y2,
                        color,
                    });
                }
            }
        })
        .marker(ratatui::symbols::Marker::Braille);

    frame.render_widget(canvas, chart_area);

    // Y axis labels
    let y_ticks = 5;
    let y_step = (y_max - y_min) / y_ticks as f64;
    for i in 0..=y_ticks {
        let y_val = y_min + i as f64 * y_step;
        let label = format_y_value(y_val);
        // Map y_val to screen position (inverted: top = max, bottom = min)
        let screen_y = chart_area.y + chart_area.height
            - ((i as f64 / y_ticks as f64) * chart_area.height as f64) as u16;
        if screen_y >= area.y && screen_y < area.y + area.height {
            let max_len = (y_label_width as usize).saturating_sub(1);
            let label_truncated: String = if label.chars().count() > max_len && max_len > 3 {
                let t: String = label.chars().take(max_len - 3).collect();
                format!("{t}...")
            } else {
                label.chars().take(max_len).collect()
            };
            let label_area = Rect::new(area.x, screen_y, y_label_width - 1, 1);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    label_truncated,
                    Style::default().fg(theme::fg_dim()),
                )))
                .alignment(Alignment::Right)
                .style(Style::default().bg(theme::bg_surface())),
                label_area,
            );
        }
    }

    // X axis labels
    let x_label_y = chart_area.y + chart_area.height;
    if x_label_y < area.y + area.height {
        let num_labels = (chart_area.width / 12).max(2) as usize;
        for i in 0..num_labels {
            let frac = i as f64 / (num_labels - 1).max(1) as f64;
            let x_val = x_min + frac * (x_max - x_min);
            let label = if prepared.x_is_date {
                format_date_label(x_val, range_ms)
            } else {
                format_y_value(x_val) // reuse number formatter
            };
            let screen_x = chart_area.x + (frac * (chart_area.width as f64 - 1.0)) as u16;
            let label_len = label.chars().count() as u16;
            if screen_x + label_len <= area.x + area.width {
                let label_area = Rect::new(screen_x, x_label_y, label_len, 1);
                frame.render_widget(
                    Paragraph::new(Span::styled(label, Style::default().fg(theme::fg_dim())))
                        .style(Style::default().bg(theme::bg_surface())),
                    label_area,
                );
            }
        }
    }
}

fn x_y_values(point: &(XValue, f64)) -> (f64, f64) {
    let x = match &point.0 {
        XValue::Numeric(v) => *v,
        XValue::Date(ms) => *ms as f64,
        XValue::Categorical(_) => 0.0,
    };
    (x, point.1)
}

fn x_label(x: &XValue) -> String {
    match x {
        XValue::Numeric(v) => format_y_value(*v),
        XValue::Date(ms) => {
            let secs = *ms / 1000;
            chrono::DateTime::from_timestamp(secs, 0).map_or_else(|| format!("{ms}"), |dt| dt.format("%Y-%m-%d").to_string())
        }
        XValue::Categorical(s) => {
            if s.chars().count() > 10 {
                let truncated: String = s.chars().take(7).collect();
                format!("{truncated}...")
            } else {
                s.clone()
            }
        }
    }
}

fn render_legend(frame: &mut Frame, area: Rect, series: &[ChartSeries]) {
    let max_label_len = series
        .iter()
        .map(|s| s.label.chars().count())
        .max()
        .unwrap_or(0);
    let legend_width = (max_label_len + 4).min(area.width as usize);
    let legend_height = series.len().min(area.height.saturating_sub(2) as usize);

    if legend_width < 4 || legend_height == 0 {
        return;
    }

    // Position: top-right corner
    let legend_x = area.x + area.width - legend_width as u16 - 1;
    let legend_y = area.y + 1;

    let legend_area = Rect::new(
        legend_x,
        legend_y,
        legend_width as u16,
        legend_height as u16 + 2,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::border()))
        .style(Style::default().bg(theme::bg_surface()));

    let inner = block.inner(legend_area);
    frame.render_widget(block, legend_area);

    for (i, s) in series.iter().enumerate().take(legend_height) {
        if i as u16 >= inner.height {
            break;
        }
        let color = series_color(i);
        let max_len = inner.width.saturating_sub(3) as usize;
        let truncated: String = if s.label.chars().count() > max_len && max_len > 3 {
            let t: String = s.label.chars().take(max_len - 3).collect();
            format!("{t}...")
        } else {
            s.label.chars().take(max_len).collect()
        };
        let line = Line::from(vec![
            Span::styled("\u{25A0} ", Style::default().fg(color)),
            Span::styled(truncated, Style::default().fg(theme::fg())),
        ]);
        let line_area = Rect::new(inner.x, inner.y + i as u16, inner.width, 1);
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(theme::bg_surface())),
            line_area,
        );
    }
}

/// Render an error message in the chart area
pub fn render_error(frame: &mut Frame, area: Rect, message: &str) {
    let text = Paragraph::new(Line::from(Span::styled(
        message,
        Style::default().fg(theme::red()),
    )))
    .alignment(Alignment::Center)
    .style(Style::default().bg(theme::bg_surface()))
    .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(text, area);
}

/// Render the empty/placeholder state
pub fn render_empty(frame: &mut Frame, area: Rect, message: &str) {
    let text = Paragraph::new(Line::from(Span::styled(
        message,
        Style::default().fg(theme::fg_dim()),
    )))
    .alignment(Alignment::Center)
    .style(Style::default().bg(theme::bg_surface()));
    frame.render_widget(text, area);
}
