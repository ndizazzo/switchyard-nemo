// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! This module turns totals and recorded costs into usage summary rows.

use crate::rollup::{Totals, Usage};
use switchyard_desktop::config::Config;
use switchyard_desktop::health::ServerStatus;

/// MAX_MODEL_ROWS limits the number of models shown in the usage summary.
const MAX_MODEL_ROWS: usize = 5;

/// This function builds the menu body for the current day and trailing week.
pub fn build(status: ServerStatus, usage: &Usage, config: &Config) -> Vec<String> {
    let state = match status {
        ServerStatus::Running => "running",
        ServerStatus::Stopped => "not responding",
    };
    let mut rows = vec![
        format!("Server: {state} · {}", config.server_url),
        String::new(),
    ];

    if usage.week.is_empty() {
        rows.push(if usage.all.is_empty() {
            "No requests recorded yet".into()
        } else {
            "No requests recorded in the last 7 days".into()
        });
        return rows;
    }

    rows.extend(period("Today", &usage.today));
    rows.push(String::new());
    rows.extend(period("This week", &usage.week));

    let models = model_shares(&usage.week);
    if !models.is_empty() {
        rows.push(String::new());
        rows.push("This week by model".into());
        rows.extend(models);
    }

    let unknown = usage.week.cost().unknown_calls;
    let uncounted: u64 = usage
        .week
        .routed
        .values()
        .map(|tokens| tokens.uncounted)
        .sum();
    if unknown > 0 {
        rows.push(String::new());
        rows.push(format!(
            "Spending incomplete for this week: {unknown} {} no recorded cost",
            if unknown == 1 {
                "call has"
            } else {
                "calls have"
            }
        ));
    } else if uncounted > 0 {
        rows.push(String::new());
        rows.push(format!("Uncounted answers this week: {uncounted}"));
    }
    rows
}

fn period(name: &str, totals: &Totals) -> Vec<String> {
    let requests = totals.requests();
    let mut rows = vec![format!(
        "{name} — {requests} request{} · {} tokens",
        if requests == 1 { "" } else { "s" },
        tokens(totals.tokens())
    )];
    let cost = totals.cost();
    if let Some(actual) = cost.total_usd {
        rows.push(format!("    Spent {}", money(actual)));
    }
    rows
}

fn model_shares(week: &Totals) -> Vec<String> {
    let total: u64 = week.routed.values().map(|tokens| tokens.total()).sum();
    if total == 0 {
        return Vec::new();
    }
    let mut models: Vec<_> = week
        .routed
        .iter()
        .map(|(model, tokens)| (model, tokens.total()))
        .collect();
    // Models appear by decreasing share; the model ID breaks ties for a stable order.
    models.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    models
        .into_iter()
        .take(MAX_MODEL_ROWS)
        .map(|(model, count)| format!("    {model} — {:.0}%", count as f64 / total as f64 * 100.0))
        .collect()
}

/// This function formats a token count compactly, since exact totals are not useful here.
pub(crate) fn tokens(value: u64) -> String {
    match value {
        0..=9_999 => value.to_string(),
        10_000..=999_999 => format!("{:.0}K", value as f64 / 1_000.0),
        1_000_000..=999_999_999 => format!("{:.1}M", value as f64 / 1_000_000.0),
        _ => format!("{:.1}B", value as f64 / 1_000_000_000.0),
    }
}

/// This function formats dollars, keeping the sign readable when routing cost more than it saved.
pub(crate) fn money(value: f64) -> String {
    if value < 0.0 {
        format!("-${:.2}", -value)
    } else {
        format!("${value:.2}")
    }
}
