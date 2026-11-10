//! Window, panels, table, chart card and the data tools.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Margin, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, TextStyle, Ui, Vec2,
};
use serde::{Deserialize, Serialize};

use crate::chart::{self, ChartOptions};
use crate::engine::{self, Inputs, Results, Summary, PERIODS};
use crate::fmt;
use crate::presets::{self, Preset};
use crate::store::{self, Prefs, Session, FORMAT};
use crate::theme::{self, Theme};

const ROW_H: f32 = 23.0;
const CELL_W: f32 = 62.0;
const LABEL_W: f32 = 176.0;
const CHART_H: f32 = 202.0;

#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub preset_name: String,
    pub inputs: Inputs,
    pub presets: Vec<Preset>,
    pub demand: Vec<f64>,
    pub manual: Vec<f64>,
    /// The workbook's "1st Period": which week of which year the first column
    /// stands for. Only the column labels use it — the plan itself is 104 weeks
    /// long whatever it is called.
    #[serde(default = "default_first_week")]
    pub first_week: u32,
    #[serde(default = "default_first_year")]
    pub first_year: u32,
}

fn default_first_week() -> u32 {
    45
}

fn default_first_year() -> u32 {
    2026
}

impl Document {
    /// "45/26" for the first column, "01/27" for the ninth.
    pub fn period(&self, i: usize) -> String {
        fmt::period(self.first_week, self.first_year, i)
    }

    pub fn last_period(&self) -> String {
        self.period(PERIODS - 1)
    }
}

impl Default for Document {
    fn default() -> Self {
        let presets = presets::default_presets();
        let mut demand = engine::sample_demand(320.0, 110.0, 60.0, 6.0);
        demand.resize(PERIODS, 0.0);
        let mut manual = vec![0.0; PERIODS];
        manual[8] = 1500.0;
        manual[39] = 900.0;
        Self {
            preset_name: presets[0].name.clone(),
            inputs: presets[0].inputs,
            presets,
            demand,
            manual,
            first_week: default_first_week(),
            first_year: default_first_year(),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum DataTab {
    Demand,
    Categories,
    Files,
}

pub struct App {
    pub doc: Document,
    pub results: Results,
    pub summary: Summary,
    pub theme_idx: usize,
    applied_theme: usize,
    pub status: String,
    pub show_data: bool,
    pub show_help: bool,
    pub data_tab: DataTab,
    pub chart: ChartOptions,
    pub planning_rows: bool,
    /// Shared sideways position of the graph and the weekly table, in pixels.
    pub h_offset: f32,
    pub frames: u64,
    pub shot: Option<PathBuf>,
    shot_requested: bool,
    pub path_buf: String,
    pub paste_buf: String,
    pub paste_target: usize,
    pub pattern: PatternSettings,
    /// Where the automatic copy of everything typed lives.
    pub state_path: PathBuf,
    persist: Persist,
    /// Fingerprint of everything the plan on screen was worked out from. It
    /// moves only when Recalculate runs: typing never rebuilds the plan.
    solved_sig: u64,
    /// The nine values the plan on screen was worked out from — which is not
    /// what is typed in the boxes once an edit is waiting. Keeping them apart
    /// is what holds the graph, the frozen band and the card colours in step
    /// with the last calculation.
    solved: Inputs,
    /// Set by the Recalculate button or F5, acted on once every panel has been
    /// drawn — so a number typed in this same frame is included.
    want_recalc: bool,
    /// When the last rebuild finished. Drives the short pulse on the cards.
    flash: Option<Instant>,
}

/// Book-keeping for the automatic save: what was last written, whether the
/// screen has moved on since, and what went wrong if it did.
struct Persist {
    enabled: bool,
    last_json: String,
    dirty: bool,
    dirty_since: Option<Instant>,
    saved: bool,
    error: Option<String>,
}

impl Persist {
    fn new(enabled: bool, last_json: String, saved: bool) -> Self {
        Self {
            enabled,
            last_json,
            dirty: false,
            dirty_since: None,
            saved,
            error: None,
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct PatternSettings {
    pub base: f64,
    pub season_pct: f64,
    pub growth_pct: f64,
    pub noise_pct: f64,
}

impl Default for PatternSettings {
    fn default() -> Self {
        Self {
            base: 320.0,
            season_pct: 34.0,
            growth_pct: 6.0,
            noise_pct: 18.0,
        }
    }
}

fn default_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    format!("{home}/inventory-plan.json")
}

/// How long the "this just happened" pulse lasts.
const PULSE: Duration = Duration::from_millis(1000);

/// A cheap fingerprint of everything the plan is built from: the nine values
/// plus the two rows of 104 weeks. Comparing it is how the window knows whether
/// the numbers on screen still match the numbers entered.
fn plan_signature(doc: &Document) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let inp = &doc.inputs;
    for v in [
        inp.opening_stock,
        inp.min_lot,
        inp.rounding,
        inp.static_safety,
        inp.tolerance_pct,
        inp.lower_pct,
    ] {
        v.to_bits().hash(&mut h);
    }
    inp.frozen_period.hash(&mut h);
    inp.order_frequency.hash(&mut h);
    inp.safety_weeks.hash(&mut h);
    for v in doc.demand.iter().chain(doc.manual.iter()) {
        v.to_bits().hash(&mut h);
    }
    h.finish()
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.5
}

fn weeks_phrase(v: f64) -> String {
    if same(v, 1.0) {
        "1 week".to_owned()
    } else {
        format!("{} weeks", fmt::qty(v))
    }
}

/// The line the person reads after pressing Recalculate. It never just says
/// "done": either it names the numbers that moved, or it says plainly that the
/// plan already matched everything entered.
fn recalc_report(before: &Summary, after: &Summary) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !same(before.total_ordered, after.total_ordered) {
        parts.push(format!(
            "units to order {} → {}",
            fmt::qty(before.total_ordered),
            fmt::qty(after.total_ordered)
        ));
    }
    if before.order_count != after.order_count {
        parts.push(format!(
            "orders {} → {}",
            before.order_count, after.order_count
        ));
    }
    if !same(before.ending_stock, after.ending_stock) {
        parts.push(format!(
            "ending stock {} → {}",
            fmt::qty(before.ending_stock),
            fmt::qty(after.ending_stock)
        ));
    }
    if !same(before.lowest_coverage, after.lowest_coverage) {
        parts.push(format!(
            "lowest cover {} → {}",
            weeks_phrase(before.lowest_coverage),
            weeks_phrase(after.lowest_coverage)
        ));
    }
    if !same(before.peak_shortfall, after.peak_shortfall) {
        parts.push(format!(
            "peak gap {} → {}",
            fmt::qty(before.peak_shortfall),
            fmt::qty(after.peak_shortfall)
        ));
    }
    if parts.is_empty() {
        format!(
            "Recalculated — the plan came out exactly as before: {} orders, {} units, lowest cover {}.",
            after.order_count,
            fmt::qty(after.total_ordered),
            weeks_phrase(after.lowest_coverage)
        )
    } else {
        format!("Recalculated — {}.", parts.join("; "))
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, shot: Option<PathBuf>, pending_demo: bool) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, &theme::THEMES[0]);

        // Pick up whatever was on screen when the app was last closed. A check
        // run takes a picture and leaves: it starts from the sample plan every
        // time, so the picture cannot depend on whoever last used the app.
        let state_path = store::state_path();
        let loaded = if shot.is_some() {
            Ok(None)
        } else {
            store::load(&state_path)
        };
        let restored = matches!(loaded, Ok(Some(_)));
        let (session, note) = match loaded {
            Ok(Some(s)) => (s, None),
            Ok(None) => (Session::default(), None),
            Err(e) => (Session::default(), Some(e)),
        };

        let mut doc = session.doc;
        doc.demand.resize(PERIODS, 0.0);
        doc.manual.resize(PERIODS, 0.0);
        doc.demand.truncate(PERIODS);
        doc.manual.truncate(PERIODS);
        // A saved plan can hold numbers the sheet would never accept — a zero
        // rounding multiple, an order rhythm of nought weeks. Tidy them once on
        // the way in, so the boxes read exactly what the calculation uses.
        doc.inputs = doc.inputs.sanitised();
        doc.first_week = doc.first_week.clamp(1, 52);
        doc.first_year = doc.first_year.clamp(2000, 2100);
        if doc.presets.is_empty() {
            doc.presets = presets::default_presets();
        }
        if !doc.presets.iter().any(|p| p.name == doc.preset_name) {
            if let Some(first) = doc.presets.first() {
                doc.preset_name = first.name.clone();
            }
        }

        let prefs = session.prefs;
        let theme_idx = prefs.theme_idx.min(theme::THEMES.len() - 1);
        let (results, summary) = engine::run(&doc.inputs, &doc.demand, &doc.manual);

        // The starting point for "has anything changed since the last write".
        let startup = Session {
            format: FORMAT,
            doc: doc.clone(),
            prefs: Prefs {
                theme_idx,
                chart: prefs.chart,
                planning_rows: prefs.planning_rows,
                pattern: prefs.pattern,
                h_offset: prefs.h_offset,
            },
        };
        let last_json = serde_json::to_string(&startup).unwrap_or_default();
        let solved_sig = plan_signature(&doc);
        let solved = doc.inputs;
        // A check run takes a picture and exits: it must leave no trace.
        let persist_enabled = shot.is_none();

        if let Some(path) = &shot {
            // A compositor can stop handing out frames to a window nobody is
            // looking at (a locked screen does it too). Fail with a message
            // instead of waiting for ever.
            let guard = path.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(90));
                if guard.exists() {
                    std::process::exit(0);
                }
                eprintln!("screenshot: the window never painted — is the desktop locked?");
                std::process::exit(1);
            });
        }

        let status = match note {
            Some(e) => format!("Starting from the sample plan — {e}."),
            None if restored => {
                "Your plan from last time is back — change a number, then press Recalculate to work the plan out again."
                    .to_owned()
            }
            None => "Sample plan loaded — change a number, then press Recalculate to work the plan out again. Your changes are kept automatically.".to_owned(),
        };

        let mut app = Self {
            doc,
            results,
            summary,
            theme_idx,
            applied_theme: 0,
            status,
            show_data: false,
            show_help: false,
            data_tab: DataTab::Demand,
            chart: prefs.chart,
            planning_rows: prefs.planning_rows,
            h_offset: prefs.h_offset,
            frames: 0,
            shot,
            shot_requested: false,
            path_buf: default_path(),
            paste_buf: String::new(),
            paste_target: 0,
            pattern: prefs.pattern,
            state_path,
            persist: Persist::new(persist_enabled, last_json, restored),
            solved_sig,
            solved,
            want_recalc: false,
            flash: None,
        };
        if pending_demo {
            // A look-check for the state right after an edit: numbers changed,
            // plan not rebuilt, and the screen says so in three places.
            app.doc.inputs.opening_stock += 1250.0;
            app.doc.demand[12] += 120.0;
            app.status = "Opening stock and week 13 demand were changed — press Recalculate to work the plan out again.".to_owned();
        }
        app
    }

    fn recompute(&mut self) {
        self.doc.demand.resize(PERIODS, 0.0);
        self.doc.manual.resize(PERIODS, 0.0);
        let (results, summary) = engine::run(&self.doc.inputs, &self.doc.demand, &self.doc.manual);
        self.results = results;
        self.summary = summary;
        self.solved = self.doc.inputs;
        self.solved_sig = plan_signature(&self.doc);
    }

    /// True when what is typed no longer matches what the plan on screen was
    /// worked out from. Nothing rebuilds on its own — this is only ever used to
    /// say so and to point at Recalculate.
    fn pending(&self) -> bool {
        self.solved_sig != plan_signature(&self.doc)
    }

    /// The Recalculate button (or F5): work the whole plan out again from
    /// scratch and say, in plain words, what that did.
    fn rebuild_on_demand(&mut self, ctx: &egui::Context) {
        let before = self.summary.clone();
        self.recompute();
        self.flash = Some(Instant::now());
        self.status = recalc_report(&before, &self.summary);
        // One more frame straight away, so the pulse and the new numbers are
        // actually painted instead of waiting for the mouse to move.
        ctx.request_repaint();
    }

    fn apply_preset(&mut self, idx: usize) {
        if let Some(p) = self.doc.presets.get(idx) {
            self.doc.inputs = p.inputs;
            self.doc.preset_name = p.name.clone();
            self.status = format!(
                "Values loaded from category \"{}\" — press Recalculate to work the plan out with them.",
                p.name
            );
        }
    }

    /// The plan plus the window setup, as it should be remembered.
    fn session(&self) -> Session {
        Session {
            format: FORMAT,
            doc: self.doc.clone(),
            prefs: Prefs {
                theme_idx: self.theme_idx,
                chart: self.chart,
                planning_rows: self.planning_rows,
                pattern: self.pattern,
                h_offset: self.h_offset,
            },
        }
    }

    /// Write the automatic copy. Called a moment after the last change so a
    /// drag across the table does not mean hundreds of writes.
    fn autosave(&mut self, ctx: &egui::Context) {
        if !self.persist.enabled {
            return;
        }
        let json = match serde_json::to_string(&self.session()) {
            Ok(j) => j,
            Err(e) => {
                self.persist.error = Some(format!("the plan could not be prepared ({e})"));
                return;
            }
        };
        if json != self.persist.last_json {
            self.persist.last_json = json;
            self.persist.dirty = true;
            if self.persist.dirty_since.is_none() {
                self.persist.dirty_since = Some(Instant::now());
            }
        }
        if !self.persist.dirty {
            return;
        }
        let due = self
            .persist
            .dirty_since
            .map(|t| t.elapsed() >= Duration::from_millis(350))
            .unwrap_or(true);
        if due {
            self.save_now();
            // The dot was drawn before the write happened; one more frame
            // turns "saving…" into "all changes saved".
            ctx.request_repaint();
        } else {
            // Make sure the write still happens if the person stops moving.
            ctx.request_repaint_after(Duration::from_millis(140));
        }
    }

    fn save_now(&mut self) {
        if !self.persist.enabled {
            return;
        }
        let json = std::mem::take(&mut self.persist.last_json);
        match store::save_text(&self.state_path, &json) {
            Ok(()) => {
                self.persist.error = None;
                self.persist.saved = true;
            }
            Err(e) => self.persist.error = Some(e),
        }
        self.persist.last_json = json;
        self.persist.dirty = false;
        self.persist.dirty_since = None;
    }

    /// The little dot and word that tell the person their work is held.
    fn save_chip(&self, ui: &mut Ui, theme: &Theme) {
        let (text, color, hint) = if let Some(e) = &self.persist.error {
            (
                "not saved".to_owned(),
                theme.bad,
                format!("{e}\nThe numbers on screen are still here — the next change tries again."),
            )
        } else if self.persist.dirty {
            (
                "saving…".to_owned(),
                theme.warn,
                format!("Writing to {}", self.state_path.display()),
            )
        } else if self.persist.saved {
            (
                "all changes saved".to_owned(),
                theme.good,
                format!("Everything you enter is kept in\n{}", self.state_path.display()),
            )
        } else {
            (
                "kept automatically".to_owned(),
                theme.faint,
                format!(
                    "Everything you enter is saved to\n{}\na moment after you stop typing.",
                    self.state_path.display()
                ),
            )
        };
        let resp = ui.label(
            RichText::new(format!("●  {text}"))
                .font(theme::mono(10.5))
                .color(color),
        );
        let _ = resp.on_hover_text(hint);
    }

    /// A small always-there sign of whether the plan on screen matches the
    /// numbers entered. It is the plain answer to "why did nothing happen when
    /// I typed?": until Recalculate is pressed, edits wait.
    fn recalc_chip(&self, ui: &mut Ui, theme: &Theme) {
        let (text, color, hint) = if self.pending() {
            (
                "changes not applied",
                theme.warn,
                "Your edits are kept, but the plan on screen was worked out before them.\nPress Recalculate (or F5) to work it out again with the new numbers.",
            )
        } else if self.flash.is_some() {
            ("recalculated", theme.accent, "The plan was just worked out again.")
        } else {
            (
                "up to date",
                theme.faint,
                "The plan on screen was worked out from the numbers above.\nChange anything and press Recalculate to include it.",
            )
        };
        let resp = ui.label(
            RichText::new(text.to_owned())
                .font(theme::mono(10.5))
                .color(color),
        );
        ui.add_space(4.0);
        resp.on_hover_text(hint);
    }

    fn reset_to_sample(&mut self) {
        let kept = store::keep_previous(&self.state_path);
        self.doc = Document::default();
        self.h_offset = 0.0;
        // A different plan was loaded, not a number typed: work it out now so
        // the screen never opens on someone else's figures.
        self.recompute();
        self.status = match kept {
            Some(p) => format!(
                "Sample plan restored. Your previous numbers were kept in {}.",
                p.display()
            ),
            None => "Sample plan restored.".to_owned(),
        };
    }

    fn save_plan(&mut self) {
        let path = PathBuf::from(self.path_buf.clone());
        match serde_json::to_string_pretty(&self.doc) {
            Ok(json) => match std::fs::write(&path, json) {
                Ok(_) => self.status = format!("Plan saved to {}", path.display()),
                Err(e) => self.status = format!("Could not save: {e}"),
            },
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    fn load_plan(&mut self) {
        let path = PathBuf::from(self.path_buf.clone());
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Document>(&text) {
                Ok(mut doc) => {
                    doc.demand.resize(PERIODS, 0.0);
                    doc.manual.resize(PERIODS, 0.0);
                    if doc.presets.is_empty() {
                        doc.presets = presets::default_presets();
                    }
                    self.doc = doc;
                    self.recompute();
                    self.status = format!("Plan opened from {}", path.display());
                }
                Err(e) => self.status = format!("That file is not a saved plan: {e}"),
            },
            Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    fn csv_path(&self) -> PathBuf {
        let mut p = PathBuf::from(self.path_buf.clone());
        p.set_extension("csv");
        p
    }

    fn export_csv(&mut self) {
        let mut out = String::from(
            "Week,Demand,ManualReceipts,Receipts,ClosingStock,SafetyStock,LowerThreshold,UpperThreshold,CoverageWeeks,PlannedReceipts,WeeklyRequirement,Period\n",
        );
        for i in 0..PERIODS {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{}\n",
                i + 1,
                self.doc.demand[i],
                self.doc.manual[i],
                self.results.receipts[i],
                self.results.end_stock[i],
                self.results.safety_stock[i],
                self.results.lower_threshold[i],
                self.results.upper_threshold[i],
                self.results.coverage[i],
                self.results.planned_receipts[i],
                self.results.weekly_requirement[i],
                // Last, so the importer (which reads the first three columns)
                // still reads this file back unchanged.
                self.doc.period(i)
            ));
        }
        let path = self.csv_path();
        let stale = self.pending();
        match std::fs::write(&path, out) {
            Ok(_) => {
                self.status = format!(
                    "Table exported to {}{}",
                    path.display(),
                    if stale {
                        " — the calculated columns are still from before your last edit."
                    } else {
                        ""
                    }
                )
            }
            Err(e) => self.status = format!("Could not export: {e}"),
        }
    }

    fn import_csv(&mut self) {
        let path = self.csv_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("Could not read {}: {e}", path.display());
                return;
            }
        };
        let mut demand = vec![0.0; PERIODS];
        let mut manual = vec![0.0; PERIODS];
        let mut filled = 0usize;
        for line in text.lines().skip(1) {
            let cols: Vec<&str> = line.split(',').collect();
            if cols.len() < 2 {
                continue;
            }
            let week: usize = match cols[0].trim().parse::<f64>() {
                Ok(w) if w >= 1.0 && w <= PERIODS as f64 => w as usize - 1,
                _ => continue,
            };
            demand[week] = cols[1].trim().parse().unwrap_or(0.0);
            if let Some(m) = cols.get(2) {
                manual[week] = m.trim().parse().unwrap_or(0.0);
            }
            filled += 1;
        }
        if filled == 0 {
            self.status = "No rows found — the file needs a Week column and a Demand column.".to_owned();
            return;
        }
        self.doc.demand = demand;
        self.doc.manual = manual;
        self.status = format!(
            "Imported {filled} weeks from {} — press Recalculate to work the plan out with them.",
            path.display()
        );
    }

    fn fill_from_text(&mut self) {
        let values: Vec<f64> = self
            .paste_buf
            .split(|c: char| c == ',' || c == ';' || c == '\t' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect();
        if values.is_empty() {
            self.status = "No numbers found in the pasted text.".to_owned();
            return;
        }
        let target = if self.paste_target == 0 {
            &mut self.doc.demand
        } else {
            &mut self.doc.manual
        };
        *target = vec![0.0; PERIODS];
        for (i, v) in values.iter().take(PERIODS).enumerate() {
            target[i] = v.max(0.0);
        }
        let n = values.len().min(PERIODS);
        self.status = format!(
            "Filled {n} weeks of {} from pasted values — press Recalculate to work the plan out with them.",
            if self.paste_target == 0 { "demand" } else { "manual receipts" }
        );
        self.paste_buf.clear();
    }

    fn generate_pattern(&mut self) {
        let p = self.pattern;
        self.doc.demand = engine::sample_demand(
            p.base,
            p.base * p.season_pct / 100.0,
            p.base * p.noise_pct / 100.0,
            p.growth_pct,
        );
        self.status = "A fresh 104-week demand pattern was written in — press Recalculate to work the plan out with it.".to_owned();
    }

}

// ---------------------------------------------------------------- widgets --

fn card(theme: &Theme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.card)
        .stroke(Stroke::new(1.0, theme.stroke))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(14))
}

fn ghost(ui: &mut Ui, theme: &Theme, label: &str, selected: bool) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(label)
            .font(theme::heading(12.5))
            .color(if selected { theme.accent } else { theme.text }),
    )
    .fill(if selected {
        theme.accent.gamma_multiply(0.14)
    } else {
        theme.card
    })
    .stroke(Stroke::new(
        1.0,
        if selected {
            theme.accent.gamma_multiply(0.55)
        } else {
            theme.stroke
        },
    ))
    .corner_radius(9)
    .min_size(Vec2::new(0.0, 28.0));
    ui.add(btn)
}

fn primary(ui: &mut Ui, theme: &Theme, label: &str) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(label)
            .font(theme::heading(13.0))
            .color(theme.on_accent),
    )
    .fill(theme.accent)
    .stroke(Stroke::NONE)
    .corner_radius(9)
    .min_size(Vec2::new(0.0, 28.0));
    ui.add(btn)
}

fn caption(text: &str, theme: &Theme) -> RichText {
    RichText::new(text.to_uppercase())
        .font(theme::heading(10.5))
        .color(theme.dim)
}

fn section_title(ui: &mut Ui, theme: &Theme, text: &str) {
    ui.label(caption(text, theme));
    ui.add_space(6.0);
}

fn num_row(
    ui: &mut Ui,
    theme: &Theme,
    label: &str,
    hint: &str,
    value: &mut f64,
    suffix: &str,
    decimals: usize,
    speed: f64,
) {
    num_row_min(ui, theme, label, hint, value, suffix, decimals, speed, 0.0);
}

/// Same row, with a floor: a rounding multiple of zero is meaningless, and the
/// box should not be able to hold one.
fn num_row_min(
    ui: &mut Ui,
    theme: &Theme,
    label: &str,
    hint: &str,
    value: &mut f64,
    suffix: &str,
    decimals: usize,
    speed: f64,
    min: f64,
) {
    ui.horizontal(|ui| {
        let resp = ui.label(
            RichText::new(label)
                .font(theme::body(12.5))
                .color(theme.dim),
        );
        resp.on_hover_text(hint);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.style_mut().override_font_id = Some(theme::mono(12.0));
            ui.spacing_mut().interact_size.x = 92.0;
            ui.add(
                egui::DragValue::new(value)
                    .speed(speed)
                    .range(min..=1.0e9)
                    .max_decimals(decimals)
                    .min_decimals(0)
                    .suffix(suffix),
            );
        });
    });
}

fn int_row(ui: &mut Ui, theme: &Theme, label: &str, hint: &str, value: &mut u32, max: u32) {
    int_row_min(ui, theme, label, hint, value, 0, max);
}

/// Same row, with a floor: an order rhythm of zero weeks is meaningless.
fn int_row_min(
    ui: &mut Ui,
    theme: &Theme,
    label: &str,
    hint: &str,
    value: &mut u32,
    min: u32,
    max: u32,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(label)
                .font(theme::heading(12.5))
                .color(theme.dim),
        )
        .on_hover_text(hint);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.style_mut().override_font_id = Some(theme::mono(12.0));
            ui.spacing_mut().interact_size.x = 92.0;
            ui.add(
                egui::DragValue::new(value)
                    .speed(0.25)
                    .range(min..=max)
                    .max_decimals(0),
            );
        });
    });
}

/// The sheet's "1st Period" cell: the week and year the first column stands
/// for. It only renames the columns — the plan is always 104 weeks — but it is
/// how the workbook's owner finds their way around the year, so it is here.
fn first_period_row(ui: &mut Ui, theme: &Theme, week: &mut u32, year: &mut u32) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("First period")
                .font(theme::body(12.5))
                .color(theme.dim),
        )
        .on_hover_text(
            "Which week of which year the first column is — the sheet's 1st Period. 45/26 means week 45 of 2026.",
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.style_mut().override_font_id = Some(theme::mono(12.0));
            ui.spacing_mut().interact_size.x = 52.0;
            ui.add(
                egui::DragValue::new(year)
                    .speed(1.0)
                    .range(2000..=2100)
                    .max_decimals(0),
            )
            .on_hover_text("Year of the first column.");
            ui.label(RichText::new("/").font(theme::mono(12.0)).color(theme.faint));
            ui.spacing_mut().interact_size.x = 36.0;
            ui.add(
                egui::DragValue::new(week)
                    .speed(0.2)
                    .range(1..=52)
                    .max_decimals(0),
            )
            .on_hover_text("Week of the year the first column stands for (1–52).");
        });
    });
}

fn kpi_card(
    ui: &mut Ui,
    theme: &Theme,
    label: &str,
    value: String,
    unit: &str,
    sub: String,
    color: Color32,
    pulse: f32,
) {
    let out = egui::Frame::new()
        .fill(theme.card)
        .stroke(Stroke::new(1.0, theme.stroke))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width((ui.available_width() - 4.0).max(60.0));
            ui.label(caption(label, theme));
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(value)
                        .font(theme::mono(25.0))
                        .color(color),
                );
                ui.add_space(2.0);
                ui.label(RichText::new(unit).font(theme::mono(10.5)).color(theme.dim));
            });
            ui.add_space(1.0);
            ui.label(
                RichText::new(sub)
                    .font(theme::body(11.0))
                    .color(theme.faint),
            );
        });
    // A short bright outline after a rebuild: the visible answer to "did that
    // do anything?". Drawn after the card so it sits on the edge, and faded out
    // over PULSE.
    if pulse > 0.01 {
        ui.painter().rect_stroke(
            out.response.rect,
            CornerRadius::same(14),
            Stroke::new(1.6, theme.accent.gamma_multiply(0.85 * pulse)),
            StrokeKind::Inside,
        );
    }
}

// --------------------------------------------------------------- the app --

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.applied_theme != self.theme_idx {
            theme::apply(&ctx, &theme::THEMES[self.theme_idx]);
            self.applied_theme = self.theme_idx;
        }
        let theme = theme::THEMES[self.theme_idx];
        // No solve here any more: a number typed in an earlier frame waits for
        // Recalculate, exactly like the workbook it came from.
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.want_recalc = true;
        }

        if self.shot.is_some() {
            self.frames += 1;
            ctx.request_repaint();
            if self.frames >= 120 && !self.shot_requested {
                self.shot_requested = true;
                let path = self.shot.clone().unwrap();
                ctx.request_screenshot(move |image| {
                    crate::save_png(&image, &path);
                    std::process::exit(0);
                });
            }
        }

        self.top_bar(ui, &theme);
        self.status_bar(ui, &theme);
        self.side_panel(ui, &theme);
        self.central(ui, &theme);
        self.data_window(&ctx, &theme);
        self.help_window(&ctx, &theme);

        // A number typed while the panels above were drawn lands in the document
        // after the cards and the table have already been painted. Only the
        // Recalculate button (or F5) ever rebuilds: do it here, after
        // everything has been drawn, and ask for one more frame so the new
        // figures and the pulse are actually painted.
        if std::mem::take(&mut self.want_recalc) {
            self.rebuild_on_demand(&ctx);
        }
        if let Some(t) = self.flash {
            if t.elapsed() < PULSE {
                ctx.request_repaint_after(Duration::from_millis(16));
            } else {
                self.flash = None;
                // Paint the settled state once more, so the chip and the cards
                // do not stay frozen in their "just happened" look.
                ctx.request_repaint();
            }
        }

        // Keep what the person typed, without them ever pressing save.
        self.autosave(&ctx);
    }
}

impl Drop for App {
    /// Closing the window must not lose the last few seconds of typing, which
    /// the delay before the automatic write could still be holding.
    fn drop(&mut self) {
        if !self.persist.enabled {
            return;
        }
        if let Ok(json) = serde_json::to_string(&self.session()) {
            if json != self.persist.last_json || self.persist.dirty {
                let _ = store::save_text(&self.state_path, &json);
            }
        }
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut Ui, theme: &Theme) {
        egui::Panel::top("topbar")
            .exact_size(62.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme.panel)
                    .inner_margin(Margin::symmetric(18, 8)),
            )
            .show(ui, |ui| {
                let rect = ui.max_rect();
                ui.painter().line_segment(
                    [
                        Pos2::new(rect.left() - 18.0, rect.bottom() + 8.0),
                        Pos2::new(rect.right() + 18.0, rect.bottom() + 8.0),
                    ],
                    Stroke::new(1.0, theme.stroke),
                );
                ui.horizontal_centered(|ui| {
                    let (logo_rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                    ui.painter()
                        .rect_filled(logo_rect, 10.0, theme.accent.gamma_multiply(0.16));
                    ui.painter().rect_stroke(
                        logo_rect,
                        10.0,
                        Stroke::new(1.0, theme.accent.gamma_multiply(0.5)),
                        StrokeKind::Inside,
                    );
                    for (i, h) in [0.32f32, 0.55, 0.78].iter().enumerate() {
                        let x = logo_rect.left() + 10.0 + i as f32 * 7.0;
                        let bar = Rect::from_min_max(
                            Pos2::new(x, logo_rect.bottom() - 9.0 - 12.0 * h),
                            Pos2::new(x + 4.0, logo_rect.bottom() - 9.0),
                        );
                        ui.painter().rect_filled(bar, 1.0, theme.accent);
                    }
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(
                            RichText::new("Inventory Simulator")
                                .font(theme::heading(19.0))
                                .color(theme.text),
                        );
                        ui.label(
                            RichText::new("104-week cover and replenishment plan")
                                .font(theme::body(11.5))
                                .color(theme.faint),
                        );
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let waiting = self.pending();
                        let recalc = primary(ui, theme, "Recalculate");
                        if recalc.clicked() {
                            // Finish anything half-typed in a box first, so the
                            // rebuild uses the number the person can see.
                            ui.ctx().memory_mut(|m| m.stop_text_input());
                            self.want_recalc = true;
                        }
                        // While edits are waiting, the one button that can act on
                        // them wears an amber ring and a dot, so it is obvious
                        // where the next click goes.
                        if waiting {
                            ui.painter().rect_stroke(
                                recalc.rect.expand(2.5),
                                CornerRadius::same(11),
                                Stroke::new(1.5, theme.warn.gamma_multiply(0.85)),
                                StrokeKind::Outside,
                            );
                            ui.painter().circle_filled(
                                Pos2::new(recalc.rect.right() - 3.0, recalc.rect.top() + 3.0),
                                3.5,
                                theme.warn,
                            );
                        }
                        recalc.on_hover_text(
                            "Works the whole plan out from the nine values and the 104 weeks entered, and tells you in the status bar what changed. Shortcut: F5.\nChanging a number does not update the plan — this button does.",
                        );
                        ui.add_space(2.0);
                        if ghost(ui, theme, "Data & files", self.show_data).clicked() {
                            self.show_data = !self.show_data;
                        }
                        if ghost(ui, theme, "?", self.show_help).clicked() {
                            self.show_help = !self.show_help;
                        }
                        ui.add_space(6.0);
                        let mut idx = self.theme_idx;
                        egui::ComboBox::from_id_salt("theme_pick")
                            .width(112.0)
                            .selected_text(
                                RichText::new(theme.name)
                                    .font(theme::heading(12.0))
                                    .color(theme.text),
                            )
                            .show_ui(ui, |ui| {
                                for (i, t) in theme::THEMES.iter().enumerate() {
                                    ui.selectable_value(&mut idx, i, t.name);
                                }
                            });
                        if idx != self.theme_idx {
                            self.theme_idx = idx;
                        }
                    });
                });
            });
    }

    fn status_bar(&mut self, ui: &mut Ui, theme: &Theme) {
        egui::Panel::bottom("statusbar")
            .exact_size(26.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme.panel)
                    .inner_margin(Margin::symmetric(18, 3)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(self.status.as_str())
                            .font(theme::body(11.5))
                            .color(theme.dim),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "demand {}  ·  {} orders  ·  104 weeks  {}–{}",
                                fmt::qty(self.summary.total_demand),
                                self.summary.order_count,
                                self.doc.period(0),
                                self.doc.last_period()
                            ))
                            .font(theme::mono(10.5))
                            .color(theme.faint),
                        );
                        ui.add_space(4.0);
                        self.save_chip(ui, theme);
                        self.recalc_chip(ui, theme);
                    });
                });
            });
    }

    fn side_panel(&mut self, ui: &mut Ui, theme: &Theme) {
        egui::Panel::left("inputs")
            .resizable(false)
            .exact_size(330.0)
            .frame(
                egui::Frame::new()
                    .fill(theme.bg)
                    .inner_margin(Margin::symmetric(14, 12)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // ---- category
                        card(theme).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            section_title(ui, theme, "Category");
                            let mut sel = self
                                .doc
                                .presets
                                .iter()
                                .position(|p| p.name == self.doc.preset_name)
                                .unwrap_or(usize::MAX);
                            let before = sel;
                            egui::ComboBox::from_id_salt("cat_pick")
                                .width(ui.available_width())
                                .selected_text(
                                    RichText::new(self.doc.preset_name.as_str())
                                        .font(theme::heading(12.5))
                                        .color(theme.text),
                                )
                                .show_ui(ui, |ui| {
                                    for (i, p) in self.doc.presets.iter().enumerate() {
                                        ui.selectable_value(&mut sel, i, p.name.as_str());
                                    }
                                });
                            if sel != before && sel < self.doc.presets.len() {
                                self.apply_preset(sel);
                            }
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ghost(ui, theme, "Apply values", false).clicked() {
                                    if let Some(i) = self
                                        .doc
                                        .presets
                                        .iter()
                                        .position(|p| p.name == self.doc.preset_name)
                                    {
                                        self.apply_preset(i);
                                    } else {
                                        self.status =
                                            "That category is not in the list.".to_owned();
                                    }
                                }
                                if ghost(ui, theme, "Update", false).clicked() {
                                    if let Some(p) = self
                                        .doc
                                        .presets
                                        .iter_mut()
                                        .find(|p| p.name == self.doc.preset_name)
                                    {
                                        p.inputs = self.doc.inputs;
                                        self.status =
                                            format!("Category \"{}\" now holds these values.", p.name);
                                    }
                                }
                            });
                            ui.add_space(6.0);
                            first_period_row(
                                ui,
                                theme,
                                &mut self.doc.first_week,
                                &mut self.doc.first_year,
                            );
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(format!(
                                    "Columns run {} to {}.",
                                    self.doc.period(0),
                                    self.doc.last_period()
                                ))
                                .font(theme::mono(10.0))
                                .color(theme.faint),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new("Nine values per category, like the Parameters sheet.")
                                    .font(theme::body(10.5))
                                    .color(theme.faint),
                            );
                        });
                        ui.add_space(10.0);

                        // ---- stock policy
                        card(theme).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            section_title(ui, theme, "Stock & orders");
                            num_row(
                                ui,
                                theme,
                                "Opening stock",
                                "Stock on hand at the start of week 1.",
                                &mut self.doc.inputs.opening_stock,
                                "",
                                0,
                                10.0,
                            );
                            num_row(
                                ui,
                                theme,
                                "Minimum lot size",
                                "Any order is raised to at least this quantity.",
                                &mut self.doc.inputs.min_lot,
                                "",
                                0,
                                10.0,
                            );
                            num_row_min(
                                ui,
                                theme,
                                "Rounding multiple",
                                "Order quantities are rounded up to a multiple of this (packs, pallets).",
                                &mut self.doc.inputs.rounding,
                                "",
                                0,
                                1.0,
                                1.0,
                            );
                            int_row_min(
                                ui,
                                theme,
                                "Order frequency",
                                "Weeks between order windows: 1 = order every week, 2 = every other week.",
                                &mut self.doc.inputs.order_frequency,
                                1,
                                PERIODS as u32,
                            );
                            int_row(
                                ui,
                                theme,
                                "Frozen period",
                                "Weeks already committed — no new orders are placed inside it.",
                                &mut self.doc.inputs.frozen_period,
                                PERIODS as u32,
                            );
                        });
                        ui.add_space(10.0);

                        // ---- safety
                        card(theme).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            section_title(ui, theme, "Safety stock");
                            num_row(
                                ui,
                                theme,
                                "Static buffer",
                                "A floor under the safety stock for every week.",
                                &mut self.doc.inputs.static_safety,
                                "",
                                0,
                                10.0,
                            );
                            int_row(
                                ui,
                                theme,
                                "Look-ahead weeks",
                                "Safety stock also covers this many weeks of demand that follow each week.",
                                &mut self.doc.inputs.safety_weeks,
                                52,
                            );
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new("Safety for a week = the larger of the static buffer and the next weeks of demand.")
                                    .font(theme::body(10.5))
                                    .color(theme.faint),
                            );
                        });
                        ui.add_space(10.0);

                        // ---- thresholds
                        card(theme).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            section_title(ui, theme, "Thresholds");
                            num_row(
                                ui,
                                theme,
                                "Upper tolerance %",
                                "Headroom added to the reference closing balance to draw the upper line.",
                                &mut self.doc.inputs.tolerance_pct,
                                " %",
                                0,
                                1.0,
                            );
                            num_row(
                                ui,
                                theme,
                                "Lower trigger %",
                                "How much of the safety stock the lower line keeps: 60 means 60 %. Zero copies the safety stock.",
                                &mut self.doc.inputs.lower_pct,
                                " %",
                                0,
                                1.0,
                            );
                        });
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(
                                "Spreadsheet rows: demand 8 · orders 9 · receipts 10 · safety 11 · upper 12 · balance 13 (hidden) · lower 14 · cover 15 · reference 16 · classification 19 · need 20.",
                            )
                            .font(theme::body(10.0))
                            .color(theme.faint),
                        );
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("Changed numbers wait here — press Recalculate (F5) to work the plan out again.")
                                .font(theme::body(10.5))
                                .color(theme.faint),
                        );
                    });
            });
    }

    fn central(&mut self, ui: &mut Ui, theme: &Theme) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme.bg)
                    .inner_margin(Margin {
                        left: 14,
                        right: 16,
                        top: 12,
                        bottom: 10,
                    }),
            )
            .show(ui, |ui| {
                // Nothing may fall off the bottom on a small window — the amber
                // "changes waiting" note adds a row, so the whole column
                // scrolls when it has to.
                egui::ScrollArea::vertical()
                    .id_salt("centre_column")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.pending_banner(ui, theme);
                        self.kpis(ui, theme);
                        ui.add_space(10.0);
                        self.chart_card(ui, theme);
                        ui.add_space(6.0);
                        self.table_card(ui, theme);
                    });
            });
    }

    /// Shown only while a number typed does not yet appear in the plan: one
    /// plain sentence and the button that fixes it, right where the person is
    /// looking.
    fn pending_banner(&mut self, ui: &mut Ui, theme: &Theme) {
        if !self.pending() {
            return;
        }
        egui::Frame::new()
            .fill(theme.warn.gamma_multiply(0.10))
            .stroke(Stroke::new(1.0, theme.warn.gamma_multiply(0.40)))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.5, theme.warn);
                    ui.label(
                        RichText::new("Your changes are waiting — the plan below is the one worked out before them.")
                            .font(theme::body(12.0))
                            .color(theme.warn),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ghost(ui, theme, "Recalculate now", false).clicked() {
                            ui.ctx().memory_mut(|m| m.stop_text_input());
                            self.want_recalc = true;
                        }
                    });
                });
            });
        ui.add_space(10.0);
    }

    fn kpis(&mut self, ui: &mut Ui, theme: &Theme) {
        let s = self.summary.clone();
        // Fades from 1 to 0 over PULSE seconds after a rebuild.
        let pulse = self
            .flash
            .map(|t| 1.0 - (t.elapsed().as_secs_f32() / PULSE.as_secs_f32()).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        ui.columns(4, |cols| {
            let first = s
                .first_order_week
                .map(|w| format!("first receipt {}", self.doc.period(w - 1)))
                .unwrap_or_else(|| "no orders needed".to_owned());
            kpi_card(
                &mut cols[0],
                theme,
                "Units to order",
                fmt::qty(s.total_ordered),
                "units",
                format!("{} orders · {}", s.order_count, first),
                theme.good,
                pulse,
            );
            let end_color = if s.ending_stock < self.solved.static_safety {
                theme.warn
            } else {
                theme.accent
            };
            kpi_card(
                &mut cols[1],
                theme,
                "Ending stock",
                fmt::qty(s.ending_stock),
                "units",
                format!("after {}", self.doc.last_period()),
                end_color,
                pulse,
            );
            kpi_card(
                &mut cols[2],
                theme,
                "Weeks below safety",
                format!("{}", s.weeks_below_safety),
                "of 104",
                format!("peak gap {}", fmt::qty(s.peak_shortfall)),
                if s.weeks_below_safety == 0 {
                    theme.good
                } else if s.weeks_below_safety <= 6 {
                    theme.warn
                } else {
                    theme.bad
                },
                pulse,
            );
            kpi_card(
                &mut cols[3],
                theme,
                "Average cover",
                format!("{:.1}", s.average_coverage),
                "weeks",
                match s.weeks_under_min_lot {
                    0 => "every week carries the minimum lot".to_owned(),
                    1 => "1 week below your minimum lot".to_owned(),
                    n => format!("{n} weeks below your minimum lot"),
                },
                if s.average_coverage < 2.0 {
                    theme.bad
                } else if s.average_coverage < 4.0 {
                    theme.warn
                } else {
                    theme.good
                },
                pulse,
            );
        });
    }

    fn chart_card(&mut self, ui: &mut Ui, theme: &Theme) {
        card(theme).show(ui, |ui| {
            let full_w = ui.available_width();
            ui.set_width(full_w);

            // Which weeks the plot can show at once, and the ones at its left
            // edge right now (the same ones the table shows underneath).
            let view_w = (full_w - LABEL_W).max(CELL_W);
            let per = (view_w / CELL_W).floor().max(1.0) as usize;
            let i0 = (self.h_offset / CELL_W).floor().max(0.0) as usize;
            let i1 = (((self.h_offset + view_w - CELL_W * 0.5) / CELL_W).floor().max(0.0)
                as usize)
                .min(PERIODS - 1);

            let mut page = 0i32;
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Stock cover")
                                .font(theme::heading(15.0))
                                .color(theme.text),
                        );
                        ui.add_space(6.0);
                        // Key for the solid line, so it says what it is without
                        // anybody having to read the caption underneath. Kept
                        // beside the title so a narrow window can never clip it.
                        let (swatch, _) =
                            ui.allocate_exact_size(Vec2::new(18.0, 12.0), Sense::hover());
                        ui.painter().line_segment(
                            [
                                Pos2::new(swatch.left(), swatch.center().y),
                                Pos2::new(swatch.right(), swatch.center().y),
                            ],
                            Stroke::new(2.2, theme.text),
                        );
                        ui.label(
                            RichText::new("closing stock")
                                .font(theme::mono(10.5))
                                .color(theme.text),
                        );
                    });
                    ui.label(
                        RichText::new("One column per week — scrolls sideways with the table")
                            .font(theme::body(11.0))
                            .color(theme.faint),
                    );
                });
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.add_space(3.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if ghost(ui, theme, "<", false).clicked() {
                            page = -1;
                        }
                        ui.label(
                            RichText::new(format!(
                                "{} – {}",
                                self.doc.period(i0),
                                self.doc.period(i1)
                            ))
                            .font(theme::mono(10.5))
                            .color(theme.faint),
                        )
                        .on_hover_text(format!("Weeks {}–{} of the plan", i0 + 1, i1 + 1));
                        if ghost(ui, theme, ">", false).clicked() {
                            page = 1;
                        }
                    });
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let cb = |ui: &mut Ui, on: &mut bool, text: &str, color: Color32| {
                        ui.checkbox(
                            on,
                            RichText::new(text).font(theme::mono(10.5)).color(color),
                        );
                    };
                    cb(ui, &mut self.chart.receipts, "orders", theme.good);
                    cb(ui, &mut self.chart.upper, "upper", theme.accent2);
                    cb(ui, &mut self.chart.lower, "lower", theme.bad);
                    cb(ui, &mut self.chart.safety, "safety", theme.warn);
                });
            });

            if page != 0 {
                let max_start = (PERIODS.saturating_sub(per)) as f32;
                let start = (i0 as f32 + page as f32 * per as f32).clamp(0.0, max_start);
                self.h_offset = start * CELL_W;
            }

            ui.add_space(6.0);
            let frozen = self.solved.frozen_period as usize;
            let chart = self.chart;
            chart::draw(
                ui,
                theme,
                &self.results,
                &chart,
                frozen,
                CELL_W,
                CHART_H,
                LABEL_W,
                &mut self.h_offset,
                self.doc.first_week,
                self.doc.first_year,
            );

            // The one thing the two figures most often get asked about: they
            // are measured at different moments, so say it where they are drawn.
            let cmp = engine::compare_upper(&self.results);
            ui.add_space(5.0);
            ui.label(
                RichText::new(format!(
                    "The solid line is closing stock — the inventory left at the end of each week, after that week's demand. The opening balance is neither drawn nor listed. The upper line is the reference plan's closing balance plus tolerance; {} of 104 weeks close above it.",
                    cmp.above_close
                ))
                .font(theme::body(10.5))
                .color(theme.faint),
            );
        });
    }

    fn table_card(&mut self, ui: &mut Ui, theme: &Theme) {
        let mut new_target: Option<usize> = None;
        let mut copy = false;
        let mut planned = self.planning_rows;
        let mut offset = self.h_offset;
        let waiting = self.pending();
        let frozen = self.solved.frozen_period as usize;
        let res = &self.results;
        let doc = &mut self.doc;
        let tsv = build_tsv(doc, res);
        let metas = row_meta(theme, planned);
        let mut rows: Vec<Rect> = Vec::with_capacity(metas.len());
        let mut header_rect = Rect::NOTHING;
        let mut gutter = Rect::NOTHING;
        let mut scroll_rect = Rect::NOTHING;

        card(theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(
                        RichText::new("Weekly plan")
                            .font(theme::heading(15.0))
                            .color(theme.text),
                    );
                    ui.label(
                        RichText::new(if waiting {
                            "Showing the calculation from before your edit — press Recalculate to fold in the new numbers"
                        } else {
                            "Demand and manual receipts are editable — the calculated rows update when you press Recalculate"
                        })
                        .font(theme::body(11.0))
                        .color(if waiting { theme.warn } else { theme.faint }),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost(ui, theme, "Copy table", false).clicked() {
                        copy = true;
                    }
                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut planned,
                        RichText::new("planning rows")
                            .font(theme::mono(10.5))
                            .color(theme.dim),
                    );
                    ui.add_space(6.0);
                    // Four blocks of 26 weeks. The button carries the period the
                    // block starts on — the same label the columns use — and the
                    // hint spells out the week numbers behind it.
                    for (first, last) in [(78usize, 103usize), (52, 77), (26, 51), (0, 25)] {
                        let label = doc.period(first);
                        let hint = format!(
                            "Show weeks {}–{} — {} to {}",
                            first + 1,
                            last + 1,
                            doc.period(first),
                            doc.period(last)
                        );
                        if ghost(ui, theme, &label, false)
                            .on_hover_text(hint)
                            .clicked()
                        {
                            new_target = Some(first);
                        }
                    }
                });
            });
            ui.add_space(8.0);

            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (_, g) = ui.allocate_space(Vec2::new(
                    LABEL_W,
                    ROW_H * (metas.len() as f32 + 1.0),
                ));
                gutter = g;

                let sa = egui::ScrollArea::horizontal()
                    .id_salt("plan_grid")
                    .auto_shrink([false, false])
                    .horizontal_scroll_offset(offset)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            header_rect = ui
                                .horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    for i in 0..PERIODS {
                                        week_header_cell(
                                            ui,
                                            theme,
                                            i,
                                            frozen,
                                            doc.first_week,
                                            doc.first_year,
                                        );
                                    }
                                })
                                .response
                                .rect;
                            rows.push(editable_row(ui, theme, &mut doc.demand, 0));
                            rows.push(editable_row(ui, theme, &mut doc.manual, 1));
                            rows.push(value_row(ui, theme, &res.receipts, theme.good, 2, true));
                            rows.push(balance_row(ui, theme, &res.end_stock, 3));
                            rows.push(value_row(ui, theme, &res.safety_stock, theme.warn, 4, false));
                            rows.push(value_row(
                                ui,
                                theme,
                                &res.lower_threshold,
                                theme.bad.gamma_multiply(0.9),
                                5,
                                false,
                            ));
                            rows.push(value_row(
                                ui,
                                theme,
                                &res.upper_threshold,
                                theme.accent2,
                                6,
                                false,
                            ));
                            rows.push(coverage_row(ui, theme, &res.coverage, 7));
                            rows.push(classification_row(ui, theme, res, frozen, 8));
                            if planned {
                                rows.push(value_row(
                                    ui,
                                    theme,
                                    &res.planned_receipts,
                                    theme.good.gamma_multiply(0.8),
                                    9,
                                    false,
                                ));
                                rows.push(value_row(
                                    ui,
                                    theme,
                                    &res.weekly_requirement,
                                    theme.dim,
                                    10,
                                    false,
                                ));
                            }
                        });
                    });
                scroll_rect = sa.inner_rect;
                offset = sa.state.clamped_offset().x;
            });

            // Row names are painted into the reserved gutter at each row's own
            // height, so they can never drift out of line with the numbers.
            let painter = ui.painter().clone();
            painter.text(
                Pos2::new(gutter.left() + 4.0, header_rect.center().y),
                Align2::LEFT_CENTER,
                "PERIOD",
                theme::heading(10.0),
                theme.faint,
            );
            for (idx, (name, hint, color)) in metas.iter().enumerate() {
                if idx >= rows.len() {
                    break;
                }
                let r = rows[idx];
                painter.circle_filled(Pos2::new(gutter.left() + 7.0, r.center().y), 3.0, *color);
                painter.text(
                    Pos2::new(gutter.left() + 18.0, r.center().y),
                    Align2::LEFT_CENTER,
                    *name,
                    theme::body(11.5),
                    theme.text.gamma_multiply(0.92),
                );
                let hit = Rect::from_min_size(
                    Pos2::new(gutter.left(), r.top()),
                    Vec2::new(LABEL_W - 10.0, r.height()),
                );
                ui.interact(hit, ui.scope_id().with(("row_lbl", idx)), Sense::hover())
                    .on_hover_text(*hint);
            }

            // Soft edge so a cut-off column reads as "more to the right".
            if scroll_rect.is_positive() {
                for k in 0..10 {
                    let t = k as f32 / 9.0;
                    let x = scroll_rect.right() - 22.0 + k as f32 * 2.2;
                    let strip = Rect::from_min_max(
                        Pos2::new(x, scroll_rect.top()),
                        Pos2::new(x + 2.2, scroll_rect.bottom()),
                    );
                    ui.painter().rect_filled(
                        strip,
                        0.0,
                        theme.card.gamma_multiply(0.10 + 0.85 * t * t),
                    );
                }
            }

            // No footer note: this card grows to fill the window, so anything
            // under the last row lands below the fold. The "scrolls sideways
            // with the table" hint sits in the graph's subtitle instead, where
            // it is always on screen.
        });

        self.planning_rows = planned;
        self.h_offset = offset;
        if let Some(t) = new_target {
            // Jump both the table and the graph to that block of weeks.
            self.h_offset = t as f32 * CELL_W;
        }
        if copy {
            ui.ctx().copy_text(tsv);
            self.status = if waiting {
                "The whole table is on the clipboard — the calculated columns are still from before your last edit.".to_owned()
            } else {
                "The whole table is on the clipboard — paste it into a spreadsheet.".to_owned()
            };
        }
    }

    fn data_window(&mut self, ctx: &egui::Context, theme: &Theme) {
        let mut open = self.show_data;
        egui::Window::new("Data & files")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([880.0, 620.0])
            .min_width(640.0)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .frame(
                egui::Frame::new()
                    .fill(theme.panel)
                    .stroke(Stroke::new(1.0, theme.stroke))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (DataTab::Demand, "Demand"),
                        (DataTab::Categories, "Categories"),
                        (DataTab::Files, "Files"),
                    ] {
                        if ghost(ui, theme, label, self.data_tab == tab).clicked() {
                            self.data_tab = tab;
                        }
                    }
                });
                ui.add_space(10.0);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                match self.data_tab {
                    DataTab::Demand => {
                        ui.label(
                            RichText::new("Generate a pattern")
                                .font(theme::heading(13.5))
                                .color(theme.text),
                        );
                        ui.label(
                            RichText::new(
                                "A starting point when you do not have 104 weeks of numbers yet. It replaces the demand row only.",
                            )
                            .font(theme::body(11.0))
                            .color(theme.faint),
                        );
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.style_mut().override_font_id = Some(theme::mono(12.0));
                            ui.spacing_mut().interact_size.x = 80.0;
                            ui.label(RichText::new("average").color(theme.dim));
                            ui.add(
                                egui::DragValue::new(&mut self.pattern.base)
                                    .speed(5.0)
                                    .range(1.0..=100000.0)
                                    .max_decimals(0),
                            );
                            ui.label(RichText::new("units/week").color(theme.dim));
                            ui.add_space(10.0);
                            ui.label(RichText::new("season ±").color(theme.dim));
                            ui.add(
                                egui::DragValue::new(&mut self.pattern.season_pct)
                                    .speed(1.0)
                                    .range(0.0..=90.0)
                                    .max_decimals(0)
                                    .suffix(" %"),
                            );
                            ui.label(RichText::new("trend").color(theme.dim));
                            ui.add(
                                egui::DragValue::new(&mut self.pattern.growth_pct)
                                    .speed(0.5)
                                    .range(-50.0..=100.0)
                                    .max_decimals(0)
                                    .suffix(" %"),
                            );
                            ui.label(RichText::new("noise ±").color(theme.dim));
                            ui.add(
                                egui::DragValue::new(&mut self.pattern.noise_pct)
                                    .speed(1.0)
                                    .range(0.0..=90.0)
                                    .max_decimals(0)
                                    .suffix(" %"),
                            );
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if primary(ui, theme, "Generate 104 weeks").clicked() {
                                self.generate_pattern();
                            }
                            if ghost(ui, theme, "Clear demand", false).clicked() {
                                self.doc.demand = vec![0.0; PERIODS];
                                self.status = "Demand cleared — press Recalculate to work the plan out without it.".to_owned();
                            }
                        });

                        ui.add_space(14.0);
                        ui.separator();
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("Paste numbers from a spreadsheet")
                                .font(theme::heading(13.5))
                                .color(theme.text),
                        );
                        ui.label(
                            RichText::new(
                                "One row of 104 weekly values, or 104 lines. Commas, tabs and spaces all work; extra values are ignored.",
                            )
                            .font(theme::body(11.0))
                            .color(theme.faint),
                        );
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("paste_target")
                                .width(170.0)
                                .selected_text(if self.paste_target == 0 {
                                    "Weekly demand"
                                } else {
                                    "Manual receipts"
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.paste_target, 0, "Weekly demand");
                                    ui.selectable_value(&mut self.paste_target, 1, "Manual receipts");
                                });
                            if primary(ui, theme, "Fill weeks").clicked() {
                                self.fill_from_text();
                            }
                        });
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .max_height(96.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.paste_buf)
                                        .desired_rows(3)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("320  305  298  310 …")
                                        .font(TextStyle::Monospace),
                                );
                            });
                    }
                    DataTab::Categories => {
                        ui.label(
                            RichText::new("Categories")
                                .font(theme::heading(13.5))
                                .color(theme.text),
                        );
                        ui.label(
                            RichText::new(
                                "The Parameters sheet: each row holds the nine values that fill the inputs. Pick a category on the left to use its numbers.",
                            )
                            .font(theme::body(11.0))
                            .color(theme.faint),
                        );
                        ui.add_space(8.0);
                        let mut remove: Option<usize> = None;
                        egui::ScrollArea::vertical()
                            .max_height(320.0)
                            .show(ui, |ui| {
                                egui::Grid::new("preset_grid")
                                    .num_columns(11)
                                    .spacing([6.0, 4.0])
                                    .striped(true)
                                    .show(ui, |ui| {
                                        ui.label(RichText::new("Category").color(theme.dim).font(theme::heading(11.0)));
                                        for f in presets::FIELDS {
                                            ui.label(
                                                RichText::new(f)
                                                    .color(theme.dim)
                                                    .font(theme::body(10.0)),
                                            );
                                        }
                                        ui.label("");
                                        ui.end_row();

                                        for (idx, p) in self.doc.presets.iter_mut().enumerate() {
                                            ui.add_sized(
                                                [176.0, 24.0],
                                                egui::TextEdit::singleline(&mut p.name)
                                                    .font(TextStyle::Body),
                                            );
                                            preset_fields(ui, theme, &mut p.inputs);
                                            if ghost(ui, theme, "×", false).clicked() {
                                                remove = Some(idx);
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                        if let Some(i) = remove {
                            if self.doc.presets.len() > 1 {
                                let gone = self.doc.presets.remove(i);
                                self.status = format!("Category \"{}\" removed.", gone.name);
                                self.doc.preset_name = self.doc.presets[0].name.clone();
                            }
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ghost(ui, theme, "+ Add category", false).clicked() {
                                let name = format!("Category {}", self.doc.presets.len() + 1);
                                self.doc.presets.push(Preset {
                                    name: name.clone(),
                                    inputs: self.doc.inputs,
                                });
                                self.doc.preset_name = name;
                                self.status = "Category added with the current values.".to_owned();
                            }
                            if ghost(ui, theme, "Use current values → selected", false).clicked() {
                                if let Some(p) = self
                                    .doc
                                    .presets
                                    .iter_mut()
                                    .find(|p| p.name == self.doc.preset_name)
                                {
                                    p.inputs = self.doc.inputs;
                                    self.status = format!("Category \"{}\" updated.", p.name);
                                }
                            }
                        });
                    }
                    DataTab::Files => {
                        ui.label(
                            RichText::new("Kept for you automatically")
                                .font(theme::heading(13.5))
                                .color(theme.text),
                        );
                        ui.label(
                            RichText::new(
                                "Everything you enter — the nine inputs, the categories and all 104 weeks of demand and manual receipts — is written to a file on this computer about half a second after you stop typing. The app opens on that same plan next time, even after a restart or a power cut.",
                            )
                            .font(theme::body(11.0))
                            .color(theme.faint),
                        );
                        ui.add_space(6.0);
                        egui::Frame::new()
                            .fill(theme.card)
                            .stroke(Stroke::new(1.0, theme.stroke))
                            .corner_radius(CornerRadius::same(9))
                            .inner_margin(Margin::symmetric(10, 7))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(
                                    RichText::new(self.state_path.display().to_string())
                                        .font(theme::mono(10.5))
                                        .color(theme.dim),
                                );
                            });

                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("Copies and spreadsheets")
                                .font(theme::heading(12.5))
                                .color(theme.dim),
                        );
                        ui.label(
                            RichText::new(
                                "A copy is a named file you choose — handy for keeping a second version, or for handing the plan to someone else.",
                            )
                            .font(theme::body(10.5))
                            .color(theme.faint),
                        );
                        ui.add_space(4.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.path_buf)
                                .desired_width(ui.available_width())
                                .font(TextStyle::Monospace),
                        );
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if primary(ui, theme, "Save a copy").clicked() {
                                self.save_plan();
                            }
                            if ghost(ui, theme, "Open a copy", false).clicked() {
                                self.load_plan();
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ghost(ui, theme, "Spreadsheet out (CSV)", false).clicked() {
                                self.export_csv();
                            }
                            if ghost(ui, theme, "Spreadsheet in (CSV)", false).clicked() {
                                self.import_csv();
                            }
                        });
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!(
                                "The CSV sits next to the copy path above: {}",
                                self.csv_path().display()
                            ))
                            .font(theme::mono(10.0))
                            .color(theme::THEMES[self.theme_idx].faint),
                        );

                        ui.add_space(14.0);
                        ui.separator();
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ghost(ui, theme, "Start fresh (sample plan)", false).clicked() {
                                self.reset_to_sample();
                            }
                            ui.label(
                                RichText::new(
                                    "Clears your numbers and puts the sample back; the old ones are kept in a file first.",
                                )
                                .font(theme::body(10.5))
                                .color(theme.faint),
                            );
                        });
                    }
                }
                    });
            });
        self.show_data = open;
    }

    fn help_window(&mut self, ctx: &egui::Context, theme: &Theme) {
        let mut open = self.show_help;
        let cmp = engine::compare_upper(&self.results);
        egui::Window::new("How the plan is worked out")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([720.0, 640.0])
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .frame(
                egui::Frame::new()
                    .fill(theme.panel)
                    .stroke(Stroke::new(1.0, theme.stroke))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::same(16)),
            )
            .show(ctx, |ui| {
                // There is more here than fits in one window, so the bar that
                // says so is always drawn rather than fading in on hover.
                ui.spacing_mut().scroll.floating = false;
                egui::ScrollArea::vertical()
                    .id_salt("help_scroll")
                    .auto_shrink([false, false])
                    .max_height(556.0)
                    .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let body = |ui: &mut Ui, title: &str, text: &str| {
                    ui.label(
                        RichText::new(title)
                            .font(theme::heading(13.0))
                            .color(theme.accent),
                    );
                    ui.label(
                        RichText::new(text)
                            .font(theme::body(12.0))
                            .color(theme.text),
                    );
                    ui.add_space(6.0);
                };
                body(
                    ui,
                    "Safety stock (row 11)",
                    "For every week, the larger of your static buffer and the demand of the next few weeks (the look-ahead). This is the level the plan tries never to drop below.",
                );
                body(
                    ui,
                    "Lower trigger (row 14)",
                    "Safety stock multiplied by (100 − lower %). At 60 %, the lower line sits at 60 % of safety stock, so it warns you before the buffer is fully gone. At 0 % it simply copies the safety stock.",
                );
                body(
                    ui,
                    "Order receipts (rows 9 and 20)",
                    "From the week after the frozen period, every week whose projected closing balance would fall under safety stock gets a requirement: at least the minimum lot size, rounded up to the packing multiple. Those weekly requirements are then pulled together onto your order rhythm and booked in the first week of each window that has demand. Manual receipts block automatic orders in their own week.",
                );
                body(
                    ui,
                    "Upper line (rows 12 and 16)",
                    "A reference plan built without manual receipts and without a frozen period. Its closing balance plus the tolerance % is the upper line: stock above it is more than the rhythm really needs.",
                );
                body(
                    ui,
                    "Cover (row 15)",
                    "How many weeks of demand the stock at the start of that week covers, counting partial weeks and rounded to whole weeks. It is computed only where stock is at least one minimum lot.",
                );
                body(
                    ui,
                    "Classification (row 19)",
                    "One word per week, read off the closing stock: frozen while the week is still inside the frozen period, below when the closing stock is under the lower trigger, over when it is above the upper line, and in range between the two. It is a reading of the plan, not an input — nothing changes if you ignore it.",
                );
                body(
                    ui,
                    "Period labels",
                    "The columns are named the way the workbook names them: week of the year, then the year, so 45/26 is week 45 of 2026 and the eighth column after it is 01/27. Set the First period in the Category card when your plan starts on a different week; it renames the columns and nothing else. The plan is always 104 weeks long.",
                );
                body(
                    ui,
                    "Why stock sits above the upper line",
                    &format!(
                        "The two figures are measured at different moments. The opening balance is what you hold as a week opens — that week's orders are already in, last week's demand is already out — and it is neither listed in the table nor drawn on the graph. The upper line is built from a closing balance, so it has already taken out that same week's demand. With a single-week order rhythm, no frozen period, no manual receipts and no tolerance, both plans then order exactly the same quantities and the opening balance is above the upper line by exactly that week's demand, in all 104 weeks. The tolerance % lifts the upper line and eats into that lead, and the two plans can still part company in the weeks where the reference plan books an order at its first week of need while the real plan waits for the first week that actually sells. In this plan, {} of 104 weeks open above the upper line and {} close above it — the closing stock, the solid line on the graph, is the like-for-like comparison.",
                        cmp.above_open, cmp.above_close
                    ),
                );
                body(
                    ui,
                    "When it is worked out",
                    "Typing a number — a value on the left, or a demand or manual receipt in the weekly plan — never changes the plan by itself. It waits, with an amber note on screen, until you press Recalculate (or F5). Then every row is worked out again from scratch and the status bar says what moved.",
                );
                ui.separator();
                ui.label(
                    RichText::new(
                        "Column mapping: demand row 8 · order receipts row 9 · manual receipts row 10 · safety stock row 11 · upper line row 12 · lower line row 14 · cover row 15 · reference receipts row 16 · classification row 19 · weekly requirement row 20. The opening balance (row 13) is not listed any more, but cover weeks are still worked out from it. Closing stock is that opening balance after the week's demand — the solid line on the graph, and the figure to read against the upper line.",
                    )
                    .font(theme::body(11.0))
                    .color(theme.faint),
                );
                ui.add_space(6.0);
                    });
            });
        self.show_help = open;
    }
}

// ------------------------------------------------------------- grid cells --

fn build_tsv(doc: &Document, res: &Results) -> String {
    let mut out = String::from(
        "Week\tPeriod\tDemand\tManual receipts\tOrder receipts\tClosing stock\tSafety stock\tLower trigger\tUpper line\tCover (weeks)\tReference receipts\tWeekly need\n",
    );
    for i in 0..PERIODS {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            i + 1,
            doc.period(i),
            doc.demand[i],
            doc.manual[i],
            res.receipts[i],
            res.end_stock[i],
            res.safety_stock[i],
            res.lower_threshold[i],
            res.upper_threshold[i],
            res.coverage[i],
            res.planned_receipts[i],
            res.weekly_requirement[i]
        ));
    }
    out
}

fn row_meta(theme: &Theme, planned: bool) -> Vec<(&'static str, &'static str, Color32)> {
    let mut rows = vec![
        (
            "Demand",
            "Weekly demand, in units. Editable — drag a number or click it and type.",
            theme.text,
        ),
        (
            "Manual receipts",
            "Stock you know is arriving (row 10). Editable; a manual receipt blocks the automatic order for that week.",
            theme.warn,
        ),
        (
            "Order receipts",
            "Calculated orders after they are gathered onto your order rhythm (row 9).",
            theme.good,
        ),
        (
            "Closing stock",
            "Inventory left at the end of the week, after that week's demand. This is the solid line on the graph, and the figure that compares directly with the upper line.",
            theme.text,
        ),
        (
            "Safety stock",
            "The level the plan protects: max(static buffer, next weeks of demand) (row 11).",
            theme.warn,
        ),
        (
            "Lower trigger",
            "Safety stock × (100 − lower %) (row 14).",
            theme.bad,
        ),
        (
            "Upper line",
            "Reference closing balance plus tolerance (row 12).",
            theme.accent2,
        ),
        (
            "Cover (weeks)",
            "Weeks of demand the stock on hand at the start of the week covers (row 15).",
            theme.text,
        ),
        (
            "Classification",
            "One word per week, as in row 19 of the sheet: frozen inside the frozen period, below when the closing stock is under the lower trigger, over when it is above the upper line, in range otherwise.",
            theme.dim,
        ),
    ];
    if planned {
        rows.push((
            "Reference receipts",
            "Receipts of the reference plan without manual receipts or a frozen period (row 16).",
            theme.good.gamma_multiply(0.8),
        ));
        rows.push((
            "Weekly need",
            "Unconsolidated weekly requirement before orders are gathered (row 20).",
            theme.dim,
        ));
    }
    rows
}

/// Row 19 of the sheet: a word per column saying how that week stands against
/// its own trigger lines. The sheet shows "in range" and "frozen"; the two
/// outside-the-band words are the natural counterparts of the upper and lower
/// rows, and they use the same colours as those rows.
fn classification_row(
    ui: &mut Ui,
    theme: &Theme,
    res: &Results,
    frozen: usize,
    row: usize,
) -> Rect {
    let resp = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for i in 0..PERIODS {
            let end = res.end_stock.get(i).copied().unwrap_or(0.0);
            let lower = res.lower_threshold.get(i).copied().unwrap_or(0.0);
            let upper = res.upper_threshold.get(i).copied().unwrap_or(0.0);
            let (text, color) = if i < frozen {
                ("frozen", theme.warn.gamma_multiply(0.9))
            } else if end < lower {
                ("below", theme.bad)
            } else if end > upper {
                ("over", theme.accent2)
            } else {
                ("in range", theme.good.gamma_multiply(0.85))
            };
            let (rect, _) = ui.allocate_exact_size(Vec2::new(CELL_W, ROW_H), Sense::hover());
            if let Some(bg) = stripe(theme, row) {
                ui.painter().rect_filled(rect, 0.0, bg);
            }
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                text,
                theme::mono(9.5),
                color,
            );
        }
    });
    resp.response.rect
}

fn week_header_cell(
    ui: &mut Ui,
    theme: &Theme,
    i: usize,
    frozen: usize,
    first_week: u32,
    first_year: u32,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(CELL_W, ROW_H), Sense::hover());
    if i < frozen {
        ui.painter()
            .rect_filled(rect, 0.0, theme.warn.gamma_multiply(0.07));
    }
    if i > 0 && i % 13 == 0 {
        ui.painter().line_segment(
            [
                Pos2::new(rect.left(), rect.top()),
                Pos2::new(rect.left(), rect.bottom()),
            ],
            Stroke::new(1.0, theme.stroke),
        );
    }
    // The sheet labels its columns by week-of-year, so these do too; every
    // thirteenth column is the start of a quarter and reads brighter.
    let is_quarter = i == 0 || (i + 1) % 13 == 1 || i + 1 == PERIODS;
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        fmt::period(first_week, first_year, i),
        theme::mono(9.5),
        if is_quarter { theme.text } else { theme.faint },
    );
}

fn stripe(theme: &Theme, row: usize) -> Option<Color32> {
    if row % 2 == 1 {
        Some(theme.card_soft.gamma_multiply(0.85))
    } else {
        None
    }
}

fn text_cell(ui: &mut Ui, text: &str, color: Color32, bg: Option<Color32>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(CELL_W, ROW_H), Sense::hover());
    if let Some(bg) = bg {
        ui.painter().rect_filled(rect, 2.0, bg);
    }
    if !text.is_empty() {
        ui.painter().text(
            Pos2::new(rect.right() - 6.0, rect.center().y),
            Align2::RIGHT_CENTER,
            text,
            theme::mono(11.5),
            color,
        );
    }
}

fn edit_cell(ui: &mut Ui, value: &mut f64, bg: Option<Color32>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(CELL_W, ROW_H), Sense::hover());
    if let Some(bg) = bg {
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    let inner = rect.shrink2(Vec2::new(3.0, 2.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::right_to_left(Align::Center)),
    );
    child.style_mut().override_font_id = Some(theme::mono(11.5));
    child.spacing_mut().interact_size.x = inner.width();
    child.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    child.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;
    child.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
    child.add(
        egui::DragValue::new(value)
            .speed(2.0)
            .range(0.0..=1.0e9)
            .max_decimals(0)
            .min_decimals(0)
            // Same grouped text as the calculated rows ("1,500", not "1500");
            // typing keeps working because commas are stripped again.
            .custom_formatter(|n, _| fmt::qty(n))
            .custom_parser(fmt::parse_qty),
    );
}

fn editable_row(ui: &mut Ui, theme: &Theme, values: &mut [f64], row: usize) -> Rect {
    let resp = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for i in 0..PERIODS {
            if i >= values.len() {
                break;
            }
            edit_cell(ui, &mut values[i], stripe(theme, row));
        }
    });
    resp.response.rect
}

fn value_row(
    ui: &mut Ui,
    theme: &Theme,
    values: &[f64],
    color: Color32,
    row: usize,
    tint_positive: bool,
) -> Rect {
    let resp = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for i in 0..PERIODS {
            let v = values.get(i).copied().unwrap_or(0.0);
            let bg = if tint_positive && v > 0.0 {
                Some(theme.good.gamma_multiply(0.16))
            } else {
                stripe(theme, row)
            };
            let text = if v == 0.0 {
                "·".to_owned()
            } else {
                fmt::qty(v)
            };
            text_cell(
                ui,
                &text,
                if v == 0.0 { theme.faint } else { color },
                bg,
            );
        }
    });
    resp.response.rect
}

fn balance_row(ui: &mut Ui, theme: &Theme, values: &[f64], row: usize) -> Rect {
    let resp = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for i in 0..PERIODS {
            let v = values.get(i).copied().unwrap_or(0.0);
            text_cell(
                ui,
                &fmt::qty(v),
                if v < 0.0 { theme.bad } else { theme.text },
                stripe(theme, row),
            );
        }
    });
    resp.response.rect
}

fn coverage_row(ui: &mut Ui, theme: &Theme, values: &[f64], row: usize) -> Rect {
    let resp = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for i in 0..PERIODS {
            let v = values.get(i).copied().unwrap_or(0.0);
            let color = if v <= 0.0 {
                theme.faint
            } else if v < 2.0 {
                theme.bad
            } else if v < 4.0 {
                theme.warn
            } else {
                theme.good
            };
            let text = if v <= 0.0 {
                "·".to_owned()
            } else {
                format!("{}", v as i64)
            };
            text_cell(ui, &text, color, stripe(theme, row));
        }
    });
    resp.response.rect
}

fn preset_fields(ui: &mut Ui, theme: &Theme, inputs: &mut Inputs) {
    let _ = &theme;
    ui.style_mut().override_font_id = Some(theme::mono(11.0));
    ui.spacing_mut().interact_size.x = 58.0;
    ui.add(egui::DragValue::new(&mut inputs.opening_stock).speed(10.0).range(0.0..=1e9).max_decimals(0));
    ui.add(egui::DragValue::new(&mut inputs.min_lot).speed(10.0).range(0.0..=1e9).max_decimals(0));
    ui.add(egui::DragValue::new(&mut inputs.rounding).speed(1.0).range(1.0..=1e6).max_decimals(0));
    ui.add(egui::DragValue::new(&mut inputs.frozen_period).speed(0.25).range(0..=PERIODS as u32));
    ui.add(egui::DragValue::new(&mut inputs.order_frequency).speed(0.25).range(1..=PERIODS as u32));
    ui.add(egui::DragValue::new(&mut inputs.static_safety).speed(10.0).range(0.0..=1e9).max_decimals(0));
    ui.add(egui::DragValue::new(&mut inputs.safety_weeks).speed(0.25).range(0..=52u32));
    ui.add(egui::DragValue::new(&mut inputs.tolerance_pct).speed(1.0).range(0.0..=500.0).max_decimals(0));
    ui.add(egui::DragValue::new(&mut inputs.lower_pct).speed(1.0).range(0.0..=100.0).max_decimals(0));
}

/// Text dump used while verifying the port against the original macros.
pub fn selftest() -> String {
    let dem: Vec<f64> = (1..=PERIODS)
        .map(|i| {
            let f = i as f64;
            let v = 300.0
                + 120.0 * (std::f64::consts::TAU * f / 52.0).sin()
                + 60.0 * (std::f64::consts::TAU * f / 13.0).sin()
                + ((i * 37) % 50) as f64;
            v.round()
        })
        .collect();
    let mut manual = vec![0.0; PERIODS];
    manual[9] = 1500.0;
    manual[40] = 900.0;
    let scenarios = [
        (
            "s1",
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
            manual.clone(),
        ),
        (
            "s2",
            Inputs {
                opening_stock: 500.0,
                min_lot: 0.0,
                rounding: 1.0,
                frozen_period: 0,
                order_frequency: 3,
                static_safety: 0.0,
                safety_weeks: 0,
                tolerance_pct: 0.0,
                lower_pct: 0.0,
            },
            vec![0.0; PERIODS],
        ),
        (
            "s3",
            Inputs {
                opening_stock: 100.0,
                min_lot: 500.0,
                rounding: 25.0,
                frozen_period: 8,
                order_frequency: 4,
                static_safety: 400.0,
                safety_weeks: 3,
                tolerance_pct: 25.0,
                lower_pct: 70.0,
            },
            manual.clone(),
        ),
    ];
    let mut parts: Vec<String> = Vec::new();
    for (name, inputs, manual) in scenarios {
        let (r, _) = engine::run(&inputs, &dem, &manual);
        let arr = |v: &Vec<f64>| {
            v.iter()
                .map(|x| format!("{:.6}", x))
                .collect::<Vec<_>>()
                .join(",")
        };
        parts.push(format!(
            "\"{name}\":{{\"safety\":[{}],\"lower\":[{}],\"receipts\":[{}],\"intermediate\":[{}],\"balance\":[{}],\"closing\":[{}],\"upper\":[{}],\"weekly\":[{}],\"sim\":[{}],\"coverage\":[{}]}}",
            arr(&r.safety_stock),
            arr(&r.lower_threshold),
            arr(&r.receipts),
            arr(&r.unconsolidated),
            arr(&r.stock_balance),
            arr(&r.end_stock),
            arr(&r.upper_threshold),
            arr(&r.weekly_requirement),
            arr(&r.planned_receipts),
            arr(&r.coverage),
        ));
    }
    format!("{{{}}}", parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(orders: usize, units: f64, cover: f64) -> Summary {
        Summary {
            order_count: orders,
            total_ordered: units,
            lowest_coverage: cover,
            ..Default::default()
        }
    }

    /// The button must never answer with a bare "done": either it names the
    /// numbers that moved, or it says the plan came out the same.
    #[test]
    fn recalc_report_explains_itself() {
        let same_plan = recalc_report(&summary(22, 26_400.0, 1.0), &summary(22, 26_400.0, 1.0));
        assert!(same_plan.contains("exactly as before"), "{same_plan}");
        assert!(same_plan.contains("22 orders"), "{same_plan}");
        assert!(same_plan.contains("1 week"), "{same_plan}");
        assert!(!same_plan.contains("on its own"), "{same_plan}");

        let moved = recalc_report(&summary(41, 48_200.0, 1.0), &summary(22, 26_400.0, 2.0));
        assert!(moved.contains("48,200 → 26,400"), "{moved}");
        assert!(moved.contains("orders 41 → 22"), "{moved}");
        assert!(moved.contains("lowest cover 1 week → 2 weeks"), "{moved}");
    }

    /// A rebuild is only triggered by a real change: the fingerprint has to see
    /// every one of the nine values and every week of both rows.
    #[test]
    fn signature_notices_every_input() {
        let base = Document::default();
        let sig = plan_signature(&base);

        let mut i = base.clone();
        i.inputs.opening_stock += 10.0;
        assert_ne!(plan_signature(&i), sig);

        let mut i = base.clone();
        i.inputs.safety_weeks += 1;
        assert_ne!(plan_signature(&i), sig);

        let mut i = base.clone();
        i.inputs.lower_pct += 5.0;
        assert_ne!(plan_signature(&i), sig);

        let mut i = base.clone();
        i.demand[103] += 1.0;
        assert_ne!(plan_signature(&i), sig);

        let mut i = base.clone();
        i.manual[0] += 1.0;
        assert_ne!(plan_signature(&i), sig);

        assert_eq!(plan_signature(&base.clone()), sig);
    }

    /// The start-of-week balance is measured at a different moment from the
    /// upper line, so it is not a table row any more — and the copy the person
    /// takes must match the table they are looking at.
    #[test]
    fn the_table_hides_the_opening_balance() {
        let th = &theme::THEMES[0];
        let names: Vec<&str> = row_meta(th, true).iter().map(|r| r.0).collect();
        assert!(!names.contains(&"Stock balance"), "{names:?}");
        assert!(names.contains(&"Closing stock"), "{names:?}");
        assert!(names.contains(&"Cover (weeks)"), "{names:?}");

        let doc = Document::default();
        let (res, _) = engine::run(&doc.inputs, &doc.demand, &doc.manual);
        let tsv = build_tsv(&doc, &res);
        assert!(!tsv.contains("Stock balance"), "{}", &tsv[..200.min(tsv.len())]);
        assert!(tsv.starts_with(
            "Week\tPeriod\tDemand\tManual receipts\tOrder receipts\tClosing stock\tSafety stock"
        ));
        // The period column matches the labels on the table's header row.
        assert!(tsv.contains("45/26\t"), "{}", &tsv[..300.min(tsv.len())]);
        // Cover is still worked out from that hidden balance, so it must not
        // vanish with the row.
        assert!(res.coverage.iter().any(|c| *c > 0.0));
    }
}
