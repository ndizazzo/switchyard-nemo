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

/// Each period shows its complete cost or labels its known subtotal as incomplete.
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
    } else if cost.estimated_calls > 0 || cost.provider_reported_calls > 0 {
        rows.push(format!(
            "    Known spending {} (incomplete)",
            money(cost.known_usd)
        ));
    } else {
        rows.push("    Spending unknown".into());
    }
    rows
}

/// The summary ranks up to five answer models by token share and excludes classifier calls.
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

#[cfg(test)]
mod tests {
    use super::build;
    use crate::rollup::{ModelTokens, Totals, Usage};
    use switchyard_costs::{CostSource, RecordedCost};
    use switchyard_desktop::config::Config;
    use switchyard_desktop::health::ServerStatus;

    /// The test checks complete, partial, missing, and zero costs in both summary periods.
    #[test]
    fn summary_shows_known_subtotals_and_missing_cost_warnings() {
        let directory = tempfile::tempdir().expect("settings directory");
        let config =
            Config::defaults_for(&directory.path().join("desktop.toml")).expect("default settings");
        for (answer_usd, classifier_usd, expected, unknown) in [
            (Some(1.25), None, "    Known spending $1.25 (incomplete)", 1),
            (None, Some(1.25), "    Known spending $1.25 (incomplete)", 1),
            (Some(1.25), Some(0.25), "    Spent $1.50", 0),
            (None, None, "    Spending unknown", 2),
            (Some(0.0), None, "    Known spending $0.00 (incomplete)", 1),
        ] {
            let mut totals = Totals::default();
            for (bucket, model, usd, source) in [
                (
                    &mut totals.routed,
                    "answer",
                    answer_usd,
                    CostSource::Estimated,
                ),
                (
                    &mut totals.classifier,
                    "classifier",
                    classifier_usd,
                    CostSource::ProviderReported,
                ),
            ] {
                let mut tokens = ModelTokens {
                    requests: 1,
                    input: 100,
                    output: 10,
                    ..ModelTokens::default()
                };
                tokens.cost.add(usd.map(|usd| RecordedCost { usd, source }));
                bucket.insert(model.into(), tokens);
            }
            let usage = Usage {
                today: totals.clone(),
                week: totals,
                ..Usage::default()
            };
            let rows = build(ServerStatus::Running, &usage, &config);
            let spending: Vec<_> = rows
                .iter()
                .filter(|row| {
                    row.starts_with("    Spent ")
                        || row.starts_with("    Known spending ")
                        || row.starts_with("    Spending ")
                })
                .map(String::as_str)
                .collect();
            assert_eq!(
                spending,
                [expected, expected],
                "each period should show the expected spending row"
            );
            let warnings: Vec<_> = rows
                .iter()
                .filter(|row| row.starts_with("Spending incomplete for this week:"))
                .map(String::as_str)
                .collect();
            let expected_warnings = match unknown {
                0 => vec![],
                1 => vec!["Spending incomplete for this week: 1 call has no recorded cost"],
                2 => vec!["Spending incomplete for this week: 2 calls have no recorded cost"],
                _ => unreachable!(),
            };
            assert_eq!(
                warnings, expected_warnings,
                "the weekly summary should report the expected missing-cost warning"
            );
        }
    }
}
