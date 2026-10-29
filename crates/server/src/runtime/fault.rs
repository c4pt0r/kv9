//! Deterministic fault points for the operation drivers (docs/RANGE-SPLIT.md §14).
//!
//! Off unless `KV9_FAULT_DIR` names a directory. Each driver calls
//! [`point`] at a step boundary (`plan:install:before`, `split:seal:after`,
//! `drain:remove_voter:before`, …). The directory holds `armed.json`, a list of
//! `{"point": "...", "action": "crash" | "pause:<ms>"}` the harness writes, and
//! `fired.log`, one line per fired point that this module appends BEFORE acting.
//! A point fires once cluster-wide: a restarted node that reaches the same point
//! finds it in `fired.log` and goes on. `crash` is `process::exit(137)` with no
//! flushing — what a kill looks like; `pause` sleeps so the harness can move
//! leadership or kill another node under the step. Production never sets the
//! variable; the cost of an unarmed build is one environment lookup per point.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

static FIRED_HERE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn dir() -> Option<PathBuf> {
    std::env::var("KV9_FAULT_DIR").ok().filter(|d| !d.is_empty()).map(PathBuf::from)
}

/// Reach a fault point. Returns immediately when nothing is armed for it.
pub(super) fn point(name: &str) {
    let Some(dir) = dir() else { return };
    let Ok(armed) = std::fs::read_to_string(dir.join("armed.json")) else { return };
    let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(&armed) else { return };
    let Some(action) = entries
        .iter()
        .find(|e| e.get("point").and_then(|p| p.as_str()) == Some(name))
        .and_then(|e| e.get("action").and_then(|a| a.as_str()))
        .map(str::to_owned)
    else {
        return;
    };
    let fired = dir.join("fired.log");
    let already = std::fs::read_to_string(&fired)
        .map(|log| log.lines().any(|l| l.split_whitespace().nth(1) == Some(name)))
        .unwrap_or(false);
    if already {
        return;
    }
    let node = std::env::var("KV9_FAULT_NODE").unwrap_or_default();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&fired) {
        let _ = writeln!(f, "{node} {name} {action}");
        let _ = f.sync_all();
    }
    FIRED_HERE.lock().expect("fault log").push(format!("{name}:{action}"));
    if action == "crash" {
        std::process::exit(137);
    }
    if let Some(ms) = action.strip_prefix("pause:").and_then(|v| v.parse::<u64>().ok()) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// The `fault=` status line: whether points are armed here and which fired in
/// this process (a crash is reported by the harness from `fired.log`).
pub(super) fn status() -> String {
    serde_json::to_string(&serde_json::json!({
        "armed_dir": dir().map(|d| d.display().to_string()),
        "fired_here": *FIRED_HERE.lock().expect("fault log"),
    }))
    .expect("serialize fault status")
}
