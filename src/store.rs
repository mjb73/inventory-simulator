//! Keeping the plan between runs.
//!
//! Everything the person types lives in one small JSON file in their own
//! settings folder. The window writes it a moment after the last change and
//! reads it again on the next start, so closing the app never loses work.
//! Named copies ("Save a copy") are a separate, optional thing.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::app::{Document, PatternSettings};
use crate::chart::ChartOptions;
use crate::engine::Inputs;

pub const FORMAT: u32 = 1;

/// The bits of the window's own setup worth remembering: which theme, which
/// chart lines are on, how far the weeks are scrolled, and the numbers last
/// used by the demand generator.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme_idx: usize,
    pub chart: ChartOptions,
    pub pattern: PatternSettings,
    pub h_offset: f32,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme_idx: 0,
            chart: ChartOptions::default(),
            pattern: PatternSettings::default(),
            h_offset: 0.0,
        }
    }
}

/// One saved session: the plan itself plus the window preferences.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub format: u32,
    pub doc: Document,
    pub prefs: Prefs,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            format: FORMAT,
            doc: Document::default(),
            prefs: Prefs::default(),
        }
    }
}

/// Where the automatic copy lives. `INVENTORY_SIMULATOR_STATE` (a full file
/// path) overrides it, which is handy for tests and for keeping a second set
/// of numbers side by side.
pub fn state_path() -> PathBuf {
    if let Some(p) = std::env::var_os("INVENTORY_SIMULATOR_STATE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    // Windows keeps per-user data in %APPDATA%; Linux (and anything else) in
    // the XDG settings folder. A missing variable falls back to the working
    // directory rather than failing to start.
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("inventory-simulator").join("plan.json")
}

/// Read the automatic copy.
///
/// `Ok(None)` means there is nothing saved yet — a first run. `Err` means the
/// file was there but could not be used; it is moved aside so the next save
/// starts from something valid, and the message explains what happened.
pub fn load(path: &Path) -> Result<Option<Session>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{} could not be read ({e})", path.display())),
    };
    match serde_json::from_str::<Session>(&text) {
        Ok(s) => Ok(Some(s)),
        Err(e) => {
            let aside = path.with_file_name("plan.unreadable.json");
            let _ = std::fs::rename(path, &aside);
            Err(format!(
                "the saved plan was damaged ({e}), so it was moved to {}",
                aside.display()
            ))
        }
    }
}

/// Write the automatic copy. The text goes to a temporary file first and is
/// then renamed over the real one, so a crash mid-write cannot leave half a
/// plan behind.
pub fn save_text(path: &Path, json: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("{} could not be created ({e})", dir.display()))?;
        }
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "plan.json".to_owned());
    let tmp = path.with_file_name(format!("{name}.tmp"));
    std::fs::write(&tmp, json).map_err(|e| format!("{} could not be written ({e})", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{} could not be replaced ({e})", path.display())
    })
}

// ---------------------------------------------------------- earlier versions
//
// The automatic copy always holds the newest numbers. These extra copies hold
// older ones, written while the person works and before anything replaces the
// figures on screen. They are what makes "I lost my numbers" recoverable from
// inside the app, without hunting for files.

/// How many earlier versions are kept. Each one is a few kilobytes, so this is
/// a couple of hours of ordinary work and costs nothing worth worrying about.
pub const HISTORY_KEEP: usize = 80;

/// An earlier version of the plan, as offered back to the person.
#[derive(Clone)]
pub struct Snapshot {
    pub path: PathBuf,
    pub when: SystemTime,
    pub preset: String,
    pub inputs: Inputs,
    pub filled_weeks: usize,
    pub doc: Document,
}

fn history_dir(path: &Path) -> PathBuf {
    path.parent()
        .map(|d| d.join("history"))
        .unwrap_or_else(|| PathBuf::from("history"))
}

/// Write one earlier version. Returns the file it wrote, or `None` when the
/// folder could not be made (the app keeps working either way — the automatic
/// copy is still there).
pub fn write_snapshot(state_path: &Path, json: &str) -> Option<PathBuf> {
    let dir = history_dir(state_path);
    std::fs::create_dir_all(&dir).ok()?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    let file = dir.join(format!("{millis}.json"));
    std::fs::write(&file, json).ok()?;
    prune(&dir, HISTORY_KEEP);
    Some(file)
}

/// Drop the oldest versions once there are more than `keep`.
fn prune(dir: &Path, keep: usize) {
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("json") {
                    p.file_name().map(|n| n.to_string_lossy().into_owned())
                } else {
                    None
                }
            })
            .collect(),
        Err(_) => return,
    };
    if names.len() <= keep {
        return;
    }
    names.sort_by(|a, b| {
        let na: u128 = a.trim_end_matches(".json").parse().unwrap_or(0);
        let nb: u128 = b.trim_end_matches(".json").parse().unwrap_or(0);
        na.cmp(&nb)
    });
    let drop_count = names.len() - keep;
    for name in names.into_iter().take(drop_count) {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// The earlier versions, newest first. Damaged files are skipped rather than
/// shown as broken rows.
pub fn snapshots(state_path: &Path, limit: usize) -> Vec<Snapshot> {
    let dir = history_dir(state_path);
    let mut files: Vec<(u128, PathBuf)> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                let stem = p.file_stem()?.to_str()?.parse::<u128>().ok()?;
                Some((stem, p))
            })
            .collect(),
        Err(_) => return Vec::new(),
    };
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
        .into_iter()
        .take(limit)
        .filter_map(|(millis, path)| {
            let text = std::fs::read_to_string(&path).ok()?;
            let session: Session = serde_json::from_str(&text).ok()?;
            let doc = session.doc;
            Some(Snapshot {
                path,
                when: UNIX_EPOCH + std::time::Duration::from_millis(millis as u64),
                preset: doc.preset_name.clone(),
                inputs: doc.inputs,
                filled_weeks: doc.demand.iter().filter(|v| **v != 0.0).count(),
                doc,
            })
        })
        .collect()
}

#[cfg(test)]
pub fn read_snapshot(path: &Path) -> Option<Document> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Session>(&text).ok().map(|s| s.doc)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_every_number() {
        let dir = std::env::temp_dir().join(format!("inv-sim-store-{}", std::process::id()));
        let path = dir.join("plan.json");
        let mut s = Session::default();
        s.doc.inputs.opening_stock = 1234.0;
        s.doc.demand[17] = 987.0;
        s.doc.manual[40] = 42.0;
        s.prefs.theme_idx = 2;
        s.prefs.h_offset = 620.0;
        serde_json::to_string(&s).unwrap();
        save_text(&path, &serde_json::to_string(&s).unwrap()).unwrap();
        let back = load(&path).unwrap().unwrap();
        assert_eq!(back.doc.inputs.opening_stock, 1234.0);
        assert_eq!(back.doc.demand[17], 987.0);
        assert_eq!(back.doc.manual[40], 42.0);
        assert_eq!(back.prefs.theme_idx, 2);
        assert_eq!(back.prefs.h_offset, 620.0);
        assert_eq!(back.doc.presets.len(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn earlier_versions_are_kept_newest_first_and_can_be_read_back() {
        let dir = std::env::temp_dir().join(format!("inv-sim-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("plan.json");
        let mut s = Session::default();
        s.doc.inputs.opening_stock = 111.0;
        s.doc.demand = vec![0.0; crate::engine::PERIODS];
        s.doc.demand[3] = 77.0;
        write_snapshot(&path, &serde_json::to_string(&s).unwrap()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(8));
        s.doc.inputs.opening_stock = 222.0;
        let newest = write_snapshot(&path, &serde_json::to_string(&s).unwrap()).unwrap();

        let list = snapshots(&path, 10);
        assert_eq!(list.len(), 2, "both versions are offered back");
        assert_eq!(list[0].inputs.opening_stock, 222.0, "newest first");
        assert_eq!(list[1].inputs.opening_stock, 111.0);
        assert_eq!(list[0].filled_weeks, 1);
        let back = read_snapshot(&list[1].path).unwrap();
        assert_eq!(back.inputs.opening_stock, 111.0);
        assert_eq!(back.demand[3], 77.0);
        assert!(newest.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_a_first_run_not_an_error() {
        let path = std::env::temp_dir().join("inv-sim-does-not-exist/plan.json");
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn old_files_without_prefs_still_open() {
        let s: Session = serde_json::from_str(r#"{"format":1,"doc":{"preset_name":"x","inputs":{"opening_stock":10,"min_lot":0,"rounding":1,"frozen_period":0,"order_frequency":1,"static_safety":0,"safety_weeks":0,"tolerance_pct":0,"lower_pct":0},"presets":[],"demand":[],"manual":[]}}"#).unwrap();
        assert_eq!(s.doc.inputs.opening_stock, 10.0);
        assert_eq!(s.prefs.theme_idx, 0);
        // Plans saved before the first period existed still open, on 45/26.
        assert_eq!(s.doc.first_week, 45);
        assert_eq!(s.doc.first_year, 2026);
    }

    /// The "planning rows" setting is gone — the two rows it showed are always
    /// hidden now. A plan written by the older version still has the key, and
    /// must still open without complaint.
    #[test]
    fn a_plan_saved_with_the_planning_rows_setting_still_opens() {
        let s: Session = serde_json::from_str(r#"{"format":1,"doc":{"preset_name":"x","inputs":{"opening_stock":10,"min_lot":0,"rounding":1,"frozen_period":0,"order_frequency":1,"static_safety":0,"safety_weeks":0,"tolerance_pct":0,"lower_pct":0},"presets":[],"demand":[],"manual":[]},"prefs":{"planning_rows":true,"theme_idx":2}}"#).unwrap();
        assert_eq!(s.doc.inputs.opening_stock, 10.0);
        assert_eq!(s.prefs.theme_idx, 2);
    }
}

