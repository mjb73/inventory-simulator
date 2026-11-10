//! The planning engine: a faithful port of the workbook macros.
//!
//! Order of operations matches `ExecuteAllCalculations`:
//!   1. safety stock (row 11)   — max(static buffer, next N weeks of demand)
//!   2. lower threshold (row 14) — safety stock scaled by the percentage
//!   3. receipts (rows 9 + 20)   — weekly requirement, then consolidation
//!   4. upper threshold (rows 12 + 16) — reference plan, same-period balance
//!   5. coverage weeks (row 15)  — weeks of cover from the stock balance
//!
//! All arrays are 0-based here; the workbook's 1-based period `i` is index `i-1`.

use serde::{Deserialize, Serialize};

pub const PERIODS: usize = 104;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Inputs {
    pub opening_stock: f64,
    pub min_lot: f64,
    pub rounding: f64,
    pub frozen_period: u32,
    pub order_frequency: u32,
    pub static_safety: f64,
    pub safety_weeks: u32,
    pub tolerance_pct: f64,
    pub lower_pct: f64,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            opening_stock: 4200.0,
            min_lot: 1200.0,
            rounding: 100.0,
            frozen_period: 4,
            order_frequency: 2,
            static_safety: 900.0,
            safety_weeks: 2,
            tolerance_pct: 15.0,
            lower_pct: 60.0,
        }
    }
}

impl Inputs {
    pub fn sanitised(&self) -> Self {
        let mut i = *self;
        i.frozen_period = i.frozen_period.min(PERIODS as u32);
        i.order_frequency = i.order_frequency.max(1);
        i.min_lot = i.min_lot.max(0.0);
        if i.rounding <= 0.0 {
            i.rounding = 1.0;
        }
        i.safety_weeks = i.safety_weeks.min(PERIODS as u32);
        i.opening_stock = i.opening_stock.max(0.0);
        i.static_safety = i.static_safety.max(0.0);
        i
    }
}

#[derive(Clone, Debug, Default)]
pub struct Results {
    pub safety_stock: Vec<f64>,
    pub lower_threshold: Vec<f64>,
    pub receipts: Vec<f64>,
    pub unconsolidated: Vec<f64>,
    pub stock_balance: Vec<f64>,
    pub end_stock: Vec<f64>,
    pub planned_receipts: Vec<f64>,
    pub upper_threshold: Vec<f64>,
    pub weekly_requirement: Vec<f64>,
    pub coverage: Vec<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub total_demand: f64,
    pub total_ordered: f64,
    pub order_count: usize,
    pub first_order_week: Option<usize>,
    pub ending_stock: f64,
    pub weeks_below_safety: usize,
    pub lowest_coverage: f64,
    pub lowest_coverage_week: Option<usize>,
    pub average_coverage: f64,
    pub peak_shortfall: f64,
    pub weeks_under_min_lot: usize,
}

/// VBA `Round` is half-to-even, unlike Rust's half-away-from-zero.
fn round_half_even(x: f64) -> f64 {
    if !x.is_finite() {
        return 0.0;
    }
    let floor = x.floor();
    let diff = x - floor;
    if (diff - 0.5).abs() < 1e-9 {
        if (floor as i64) % 2 == 0 {
            floor
        } else {
            floor + 1.0
        }
    } else {
        x.round()
    }
}

/// The safety-stock rule: the larger of the static buffer and the next
/// `safety_weeks` weeks of forward demand.
pub fn safety_stock(demand: &[f64], static_safety: f64, safety_weeks: u32) -> Vec<f64> {
    let weeks = safety_weeks as usize;
    (0..PERIODS)
        .map(|i| {
            let mut forward = 0.0;
            for j in 1..=weeks {
                if i + j < PERIODS {
                    forward += demand[i + j];
                }
            }
            static_safety.max(forward)
        })
        .collect()
}

pub fn lower_threshold(safety: &[f64], pct: f64) -> Vec<f64> {
    safety
        .iter()
        .map(|s| if pct > 0.0 { s * (100.0 - pct) / 100.0 } else { *s })
        .collect()
}

/// Round a quantity up to the packing multiple (workbook: `RoundUp`).
fn round_lot(qty: f64, rounding: f64) -> f64 {
    if rounding > 1.0 {
        (qty / rounding).ceil() * rounding
    } else {
        qty
    }
}

/// Consolidate weekly requirements onto the order calendar, placing each
/// order in the first week of its window that actually needs stock.
fn consolidate(weekly: &[f64], demand: &[f64], freq: usize, by_demand: bool) -> Vec<f64> {
    let mut out = vec![0.0; PERIODS];
    let first = match (0..PERIODS).find(|&i| weekly[i] > 0.0) {
        Some(i) => i,
        None => return out,
    };
    let freq = freq.max(1);
    let mut next = first;
    while next < PERIODS {
        let end = (next + freq - 1).min(PERIODS - 1);
        let mut total = 0.0;
        let mut receipt_period: Option<usize> = None;
        for i in next..=end {
            total += weekly[i];
            let trigger = if by_demand { demand[i] > 0.0 } else { weekly[i] > 0.0 };
            if receipt_period.is_none() && trigger {
                receipt_period = Some(i);
            }
        }
        let period = receipt_period.unwrap_or(next);
        if total > 0.0 {
            out[period] += total;
        }
        next += freq;
    }
    out
}

pub fn run(inputs: &Inputs, demand_in: &[f64], manual_in: &[f64]) -> (Results, Summary) {
    let inp = inputs.sanitised();
    let mut demand = vec![0.0; PERIODS];
    let mut manual = vec![0.0; PERIODS];
    for i in 0..PERIODS {
        demand[i] = demand_in.get(i).copied().unwrap_or(0.0).max(0.0);
        manual[i] = manual_in.get(i).copied().unwrap_or(0.0).max(0.0);
    }

    // 1. Safety stock (row 11), and 2. lower threshold (row 14).
    let safety = safety_stock(&demand, inp.static_safety, inp.safety_weeks);
    let lower = lower_threshold(&safety, inp.lower_pct);

    // 3. Receipts (rows 9 and 20). A zero in the safety row falls back to the
    //    static buffer, exactly as the macro does.
    let safety_filled: Vec<f64> = safety
        .iter()
        .map(|&s| if s == 0.0 { inp.static_safety } else { s })
        .collect();

    let mut balance = vec![0.0; PERIODS];
    for i in 0..PERIODS {
        balance[i] = if i == 0 {
            inp.opening_stock + manual[0]
        } else {
            balance[i - 1] + manual[i] - demand[i - 1]
        };
    }

    let mut unconsolidated = vec![0.0; PERIODS];
    for cur in inp.frozen_period as usize..PERIODS {
        if manual[cur] != 0.0 {
            continue;
        }
        let projected = balance[cur] - demand[cur];
        let required = safety_filled[cur] - projected;
        if required <= 0.0 {
            continue;
        }
        let qty = round_lot(required.max(inp.min_lot), inp.rounding);
        unconsolidated[cur] = qty;
        balance[cur] += unconsolidated[cur];
        for j in (cur + 1)..PERIODS {
            balance[j] = balance[j - 1] - demand[j - 1] + unconsolidated[j] + manual[j];
        }
    }

    let freq = inp.order_frequency as usize;
    let receipts = consolidate(&unconsolidated, &demand, freq, true);

    // Row 13: stock at the start of each week, including consolidated orders.
    let mut stock = vec![0.0; PERIODS];
    for i in 0..PERIODS {
        stock[i] = if i == 0 {
            inp.opening_stock + receipts[0] + manual[0]
        } else {
            stock[i - 1] - demand[i - 1] + receipts[i] + manual[i]
        };
    }
    let end_stock: Vec<f64> = (0..PERIODS).map(|i| stock[i] - demand[i]).collect();

    // 4. Upper threshold (rows 16 and 12): a reference plan that ignores
    //    manual receipts and the frozen period.
    let mut plan = vec![0.0; PERIODS];
    for i in 0..PERIODS {
        plan[i] = if i == 0 {
            inp.opening_stock
        } else {
            plan[i - 1] - demand[i - 1]
        };
    }
    let mut weekly_requirement = vec![0.0; PERIODS];
    for cur in 0..PERIODS {
        let projected = plan[cur] - demand[cur];
        let required = safety_filled[cur] - projected;
        if required <= 0.0 {
            continue;
        }
        let qty = round_lot(required.max(inp.min_lot), inp.rounding);
        weekly_requirement[cur] = qty;
        plan[cur] += qty;
        for j in (cur + 1)..PERIODS {
            plan[j] = plan[j - 1] - demand[j - 1] + weekly_requirement[j];
        }
    }
    let planned_receipts = consolidate(&weekly_requirement, &demand, freq, false);

    let mut final_balance = vec![0.0; PERIODS];
    for i in 0..PERIODS {
        let prev = if i == 0 { inp.opening_stock } else { final_balance[i - 1] };
        final_balance[i] = prev + planned_receipts[i] - demand[i];
    }
    let upper: Vec<f64> = final_balance
        .iter()
        .map(|b| b * (1.0 + inp.tolerance_pct / 100.0))
        .collect();

    // 5. Coverage weeks (row 15).
    let mut coverage = vec![0.0; PERIODS];
    for i in 0..PERIODS {
        let mut weeks = 0.0;
        if stock[i] >= inp.min_lot {
            let mut remaining = stock[i];
            let mut cur = i;
            while remaining > 0.0 && cur < PERIODS {
                let d = demand[cur];
                if d > 0.0 {
                    remaining -= d;
                    if remaining >= 0.0 {
                        weeks += 1.0;
                    } else {
                        weeks += (remaining + d) / d;
                    }
                }
                cur += 1;
            }
        }
        coverage[i] = round_half_even(weeks);
    }

    // Summary used by the headline cards.
    let mut summary = Summary {
        total_demand: demand.iter().sum(),
        total_ordered: receipts.iter().sum(),
        order_count: receipts.iter().filter(|&&r| r > 0.0).count(),
        first_order_week: (0..PERIODS).find(|&i| receipts[i] > 0.0).map(|i| i + 1),
        ending_stock: end_stock[PERIODS - 1],
        ..Default::default()
    };
    summary.weeks_below_safety = (0..PERIODS).filter(|&i| end_stock[i] < safety[i]).count();
    summary.average_coverage = coverage.iter().sum::<f64>() / PERIODS as f64;
    let mut lowest = f64::INFINITY;
    let mut lowest_week = None;
    for i in 0..PERIODS {
        if coverage[i] < lowest {
            lowest = coverage[i];
            lowest_week = Some(i + 1);
        }
    }
    summary.lowest_coverage = if lowest.is_finite() { lowest } else { 0.0 };
    summary.lowest_coverage_week = lowest_week;
    summary.peak_shortfall = (0..PERIODS)
        .map(|i| (safety[i] - end_stock[i]).max(0.0))
        .fold(0.0, f64::max);
    // Weeks the workbook leaves without a cover figure: the balance is below the
    // minimum lot, so the coverage rule does not apply.
    summary.weeks_under_min_lot = (0..PERIODS)
        .filter(|&i| stock[i] < inp.min_lot)
        .count();

    (
        Results {
            safety_stock: safety,
            lower_threshold: lower,
            receipts,
            unconsolidated,
            stock_balance: stock,
            end_stock,
            planned_receipts,
            upper_threshold: upper,
            weekly_requirement,
            coverage,
        },
        summary,
    )
}

/// How the stock-balance row stands against the upper line.
///
/// The two rows are measured at different moments. Row 13 is the stock as a
/// week opens — orders and manual receipts already added, the *previous* week's
/// demand already taken out — while the upper line is built from a closing
/// balance that has already taken out *that* week's demand. So a week's own
/// demand is still inside the balance figure, and the balance sits above the
/// upper line by roughly that demand even when both plans order identically.
/// `above_close` counts the weeks where the closing stock, the like-for-like
/// figure, is above the upper line.
#[derive(Clone, Copy, Debug, Default)]
pub struct UpperCompare {
    /// Weeks whose opening balance is above the upper line.
    pub above_open: usize,
    /// Weeks whose closing balance is above the upper line.
    pub above_close: usize,
    /// Largest opening-balance overshoot and the week it happens in (1-based).
    pub worst_gap: f64,
    pub worst_week: usize,
    /// Average opening-balance overshoot across the 104 weeks.
    pub mean_gap: f64,
    /// Average weekly demand, so the app can say what the gap is made of.
    pub mean_demand: f64,
}

pub fn compare_upper(res: &Results) -> UpperCompare {
    let mut c = UpperCompare::default();
    let mut total = 0.0;
    let mut demand_total = 0.0;
    for i in 0..PERIODS {
        let upper = res.upper_threshold.get(i).copied().unwrap_or(0.0);
        let open = res.stock_balance.get(i).copied().unwrap_or(0.0);
        let close = res.end_stock.get(i).copied().unwrap_or(0.0);
        let gap = open - upper;
        total += gap;
        demand_total += (open - close).max(0.0);
        if gap > 0.0 {
            c.above_open += 1;
            if gap > c.worst_gap {
                c.worst_gap = gap;
                c.worst_week = i + 1;
            }
        }
        if close > upper {
            c.above_close += 1;
        }
    }
    c.mean_gap = total / PERIODS as f64;
    c.mean_demand = demand_total / PERIODS as f64;
    c
}

/// Deterministic sample demand: a seasonal year with a slow drift and jitter,
/// so the app opens with something meaningful to look at.
pub fn sample_demand(base: f64, season: f64, noise: f64, growth_pct_per_year: f64) -> Vec<f64> {
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    let mut out = Vec::with_capacity(PERIODS);
    for i in 0..PERIODS {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let r = ((seed >> 33) as f64 / (1u64 << 31) as f64) - 0.5;
        let t = i as f64;
        let seasonal = season * (std::f64::consts::TAU * t / 52.0).sin();
        let drift = base * (growth_pct_per_year / 100.0) * (t / 52.0);
        let v = base + seasonal + drift + noise * r;
        out.push((v.max(0.0) / 5.0).round() * 5.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demand() -> Vec<f64> {
        (1..=PERIODS)
            .map(|i| {
                let f = i as f64;
                let v = 300.0
                    + 120.0 * (std::f64::consts::TAU * f / 52.0).sin()
                    + 60.0 * (std::f64::consts::TAU * f / 13.0).sin()
                    + ((i * 37) % 50) as f64;
                v.round()
            })
            .collect()
    }

    #[test]
    fn closing_stock_is_the_balance_after_that_weeks_demand() {
        // What the graph draws. Every week it must equal the workbook's closing
        // row, which is the opening balance for that week less that week's
        // demand — never the opening balance itself.
        let d = demand();
        let mut m = vec![0.0; PERIODS];
        m[9] = 1500.0;
        let inp = Inputs {
            opening_stock: 4200.0,
            min_lot: 1200.0,
            rounding: 100.0,
            frozen_period: 4,
            order_frequency: 2,
            static_safety: 900.0,
            safety_weeks: 2,
            tolerance_pct: 15.0,
            lower_pct: 60.0,
        };
        let (r, _) = run(&inp, &d, &m);
        for i in 0..PERIODS {
            assert!(
                (r.end_stock[i] - (r.stock_balance[i] - d[i])).abs() < 1e-9,
                "week {}: closing {} balance {} demand {}",
                i + 1,
                r.end_stock[i],
                r.stock_balance[i],
                d[i]
            );
        }
        // And the two are genuinely different series, so a graph of the wrong
        // one would be visible: a week that sells stock must close lower.
        let differs = (0..PERIODS).any(|i| (r.end_stock[i] - r.stock_balance[i]).abs() > 1.0);
        assert!(differs);
    }

    #[test]
    fn matches_workbook_reference_scenario_1() {
        let d = demand();
        let mut m = vec![0.0; PERIODS];
        m[9] = 1500.0;
        m[40] = 900.0;
        let inp = Inputs {
            opening_stock: 4200.0,
            min_lot: 1200.0,
            rounding: 100.0,
            frozen_period: 4,
            order_frequency: 2,
            static_safety: 900.0,
            safety_weeks: 2,
            tolerance_pct: 15.0,
            lower_pct: 60.0,
        };
        let (r, s) = run(&inp, &d, &m);
        // Values produced by a literal translation of the macros.
        assert!((r.safety_stock[0] - 900.0).abs() < 1e-9);
        assert!((r.safety_stock[2] - 903.0).abs() < 1e-9);
        assert!((r.stock_balance[0] - 4200.0).abs() < 1e-9);
        assert!((r.stock_balance[7] - 2503.0).abs() < 1e-9);
        assert!((r.receipts.iter().sum::<f64>() - 28800.0).abs() < 1e-6);
        assert!((s.total_demand - 33770.0).abs() < 1e-6);
        assert_eq!(s.lowest_coverage, 1.0);
    }

    #[test]
    fn consolidation_preserves_totals() {
        let d = demand();
        let m = vec![0.0; PERIODS];
        for (freq, frozen) in [(1u32, 0u32), (2, 4), (3, 0), (4, 8), (8, 26)] {
            let inp = Inputs {
                order_frequency: freq,
                frozen_period: frozen,
                ..Inputs::default()
            };
            let (r, _) = run(&inp, &d, &m);
            let a: f64 = r.unconsolidated.iter().sum();
            let b: f64 = r.receipts.iter().sum();
            assert!((a - b).abs() < 1e-6, "freq {freq} frozen {frozen}: {a} vs {b}");
            for r0 in &r.receipts {
                assert!(*r0 == 0.0 || *r0 >= inp.min_lot - 1e-9);
            }
            // No order may land inside the frozen period.
            for i in 0..frozen as usize {
                assert_eq!(r.receipts[i], 0.0);
            }
        }
    }

    #[test]
    fn safety_stock_uses_forward_window() {
        let mut d = vec![0.0; PERIODS];
        d[5] = 400.0;
        d[6] = 250.0;
        let s = safety_stock(&d, 100.0, 2);
        assert!((s[3] - 400.0).abs() < 1e-9);
        assert!((s[4] - 650.0).abs() < 1e-9);
        assert!((s[5] - 250.0).abs() < 1e-9);
        assert!((s[6] - 100.0).abs() < 1e-9);
        assert!((s[103] - 100.0).abs() < 1e-9);
    }

    #[test]
    fn zero_percentage_lower_threshold_equals_safety() {
        let s = vec![500.0; PERIODS];
        assert_eq!(lower_threshold(&s, 0.0), s);
        let l = lower_threshold(&s, 40.0);
        assert!((l[0] - 300.0).abs() < 1e-9);
    }

    #[test]
    fn coverage_counts_partial_weeks_then_rounds() {
        let mut d = vec![0.0; PERIODS];
        d[0] = 100.0;
        d[1] = 100.0;
        d[2] = 100.0;
        d[3] = 100.0;
        let mut stock = vec![0.0; PERIODS];
        stock[0] = 350.0;
        stock[1] = 250.0;
        let inp = Inputs {
            min_lot: 0.0,
            ..Inputs::default()
        };
        // Direct check of the coverage rule via a full run is awkward, so
        // mirror the loop here with the same rule.
        let mut cov = 0.0;
        let mut remaining = stock[0];
        let mut cur = 0;
        while remaining > 0.0 && cur < PERIODS {
            let dd = d[cur];
            if dd > 0.0 {
                remaining -= dd;
                if remaining >= 0.0 {
                    cov += 1.0;
                } else {
                    cov += (remaining + dd) / dd;
                }
            }
            cur += 1;
        }
        assert!((round_half_even(cov) - 4.0).abs() < 1e-9);
        assert!(inp.min_lot == 0.0);
    }

    /// The answer to "why is the balance above the upper line?": on a single
    /// week rhythm, with no frozen period, no manual receipts and no tolerance,
    /// both plans order exactly the same quantities, and the balance row is
    /// above the upper line by exactly that week's demand — the stock it holds
    /// before the week is spent.
    #[test]
    fn opening_balance_is_one_weeks_demand_above_upper() {
        let d = demand();
        let m = vec![0.0; PERIODS];
        let inp = Inputs {
            opening_stock: 4200.0,
            min_lot: 1200.0,
            rounding: 100.0,
            frozen_period: 0,
            order_frequency: 1,
            static_safety: 900.0,
            safety_weeks: 2,
            tolerance_pct: 0.0,
            lower_pct: 60.0,
        };
        let (r, _) = run(&inp, &d, &m);
        for i in 0..PERIODS {
            assert_eq!(r.receipts[i], r.planned_receipts[i], "week {}", i + 1);
            assert!(
                (r.stock_balance[i] - (r.upper_threshold[i] + d[i])).abs() < 1e-6,
                "week {}: balance {} upper {} demand {}",
                i + 1,
                r.stock_balance[i],
                r.upper_threshold[i],
                d[i]
            );
        }
        let cmp = compare_upper(&r);
        assert_eq!(cmp.above_open, PERIODS);
        assert_eq!(cmp.above_close, 0);
    }

    /// The fair comparison: closing stock against the upper line, which is
    /// itself a closing figure of the reference plan. With one order rhythm,
    /// no frozen period, no manual receipts and no tolerance, it lands exactly
    /// on the upper line.
    #[test]
    fn closing_stock_lands_on_the_upper_line() {
        let d = demand();
        let m = vec![0.0; PERIODS];
        let inp = Inputs {
            frozen_period: 0,
            order_frequency: 1,
            tolerance_pct: 0.0,
            ..Inputs::default()
        };
        let (r, _) = run(&inp, &d, &m);
        for i in 0..PERIODS {
            assert!(
                (r.end_stock[i] - r.upper_threshold[i]).abs() < 1e-6,
                "week {}: closing {} upper {}",
                i + 1,
                r.end_stock[i],
                r.upper_threshold[i]
            );
        }
    }

    /// Tolerance lifts the upper line but rarely by a whole week of demand, so
    /// the opening balance still spends much of the plan above it.
    #[test]
    fn tolerance_shrinks_but_does_not_hide_the_gap() {
        let d = demand();
        let m = vec![0.0; PERIODS];
        let inp = Inputs {
            frozen_period: 0,
            order_frequency: 2,
            tolerance_pct: 15.0,
            ..Inputs::default()
        };
        let (r, _) = run(&inp, &d, &m);
        let cmp = compare_upper(&r);
        assert!(cmp.above_open > cmp.above_close, "{cmp:?}");
        assert!(cmp.mean_gap > 0.0 && cmp.mean_gap < cmp.mean_demand, "{cmp:?}");
    }
}
