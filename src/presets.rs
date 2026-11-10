//! Category presets — the workbook's Parameters sheet: one row per category,
//! nine numbers that fill the input cells.

use crate::engine::Inputs;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    /// The Parameters-sheet row: the figures this category starts with.
    pub inputs: Inputs,
    /// The last figures typed while this category was selected, kept apart
    /// from the row so switching away and back finds them again and the row
    /// itself is never rewritten behind the person's back.
    #[serde(default)]
    pub draft: Option<Inputs>,
}

/// Column captions for the preset editor, in sheet order (B..J).
pub const FIELDS: [&str; 9] = [
    "Opening stock",
    "Min lot",
    "Rounding",
    "Frozen wks",
    "Order freq",
    "Static SS",
    "Safety wks",
    "Tolerance %",
    "Lower %",
];

fn p(name: &str, i: Inputs) -> Preset {
    Preset {
        name: name.to_owned(),
        inputs: i,
        draft: None,
    }
}

pub fn default_presets() -> Vec<Preset> {
    vec![
        p(
            "Still water 500 ml",
            Inputs {
                opening_stock: 4200.0,
                min_lot: 1200.0,
                rounding: 100.0,
                frozen_period: 4,
                order_frequency: 2,
                static_safety: 900.0,
                safety_weeks: 2,
                tolerance_pct: 15.0,
                lower_pct: 60.0,
            },
        ),
        p(
            "Snack bars 40 g",
            Inputs {
                opening_stock: 2600.0,
                min_lot: 800.0,
                rounding: 50.0,
                frozen_period: 2,
                order_frequency: 1,
                static_safety: 600.0,
                safety_weeks: 3,
                tolerance_pct: 10.0,
                lower_pct: 55.0,
            },
        ),
        p(
            "Surface cleaner 750 ml",
            Inputs {
                opening_stock: 1500.0,
                min_lot: 500.0,
                rounding: 25.0,
                frozen_period: 6,
                order_frequency: 4,
                static_safety: 400.0,
                safety_weeks: 2,
                tolerance_pct: 20.0,
                lower_pct: 65.0,
            },
        ),
        p(
            "Dry pet food 2 kg",
            Inputs {
                opening_stock: 900.0,
                min_lot: 300.0,
                rounding: 10.0,
                frozen_period: 8,
                order_frequency: 3,
                static_safety: 350.0,
                safety_weeks: 4,
                tolerance_pct: 12.0,
                lower_pct: 70.0,
            },
        ),
        p(
            "Spare parts kit",
            Inputs {
                opening_stock: 240.0,
                min_lot: 60.0,
                rounding: 5.0,
                frozen_period: 10,
                order_frequency: 6,
                static_safety: 40.0,
                safety_weeks: 3,
                tolerance_pct: 25.0,
                lower_pct: 50.0,
            },
        ),
    ]
}
