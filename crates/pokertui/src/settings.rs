use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Bot action delays offered in the menu, in milliseconds.
pub const BOT_DELAY_PRESETS_MS: [u64; 6] = [0, 250, 500, 700, 1000, 2000];
/// Table sizes offered in the menu.
pub const SEAT_PRESETS: [usize; 5] = [2, 3, 4, 5, 6];
/// Blind levels offered in the menu, as `(small_blind, big_blind)` pairs.
pub const BLIND_PRESETS: [(u64, u64); 4] = [(25, 50), (50, 100), (100, 200), (200, 400)];

/// Which way a value cycles through its preset list. `Left` moves toward the
/// first preset, `Right` toward the last; both clamp at the ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cycle {
    Prev,
    Next,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// How long a bot waits before acting. Applies live.
    pub bot_delay: Duration,
    /// Whether the `EQUITY · POT ODDS` rail panel is shown. Applies live.
    pub show_win_rate: bool,
    /// Number of seats at the table. Applies on the next game.
    pub seats: usize,
    /// Small blind. Applies on the next game.
    pub small_blind: u64,
    /// Big blind. Applies on the next game.
    pub big_blind: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bot_delay: Duration::from_millis(500),
            show_win_rate: true,
            seats: 6,
            small_blind: 50,
            big_blind: 100,
        }
    }
}

impl Settings {
    /// Move the bot delay to the adjacent preset (clamped).
    pub fn cycle_bot_delay(&mut self, dir: Cycle) {
        let cur = self.bot_delay.as_millis() as u64;
        let i = step_index(&BOT_DELAY_PRESETS_MS, cur, dir);
        self.bot_delay = Duration::from_millis(BOT_DELAY_PRESETS_MS[i]);
    }

    /// Toggle the win-rate display (`Right` → off, `Left` → on, clamped).
    pub fn cycle_show_win_rate(&mut self, dir: Cycle) {
        self.show_win_rate = match dir {
            Cycle::Prev => true,
            Cycle::Next => false,
        };
    }

    /// Move the seat count to the adjacent preset (clamped).
    pub fn cycle_seats(&mut self, dir: Cycle) {
        let keys: Vec<u64> = SEAT_PRESETS.iter().map(|&n| n as u64).collect();
        let i = step_index(&keys, self.seats as u64, dir);
        self.seats = SEAT_PRESETS[i];
    }

    /// Move the blind level to the adjacent preset (clamped). Presets are
    /// ordered by big blind, so that is the key used for stepping/snapping.
    pub fn cycle_blinds(&mut self, dir: Cycle) {
        let keys: Vec<u64> = BLIND_PRESETS.iter().map(|&(_, bb)| bb).collect();
        let i = step_index(&keys, self.big_blind, dir);
        let (sb, bb) = BLIND_PRESETS[i];
        self.small_blind = sb;
        self.big_blind = bb;
    }

    /// Load settings from the config file, falling back to defaults when it is
    /// missing or unparseable. Never returns an error that could break the TUI.
    pub fn load() -> Settings {
        config_path()
            .map(|p| Self::load_from(&p))
            .unwrap_or_default()
    }

    /// Persist settings to the config file. Best-effort: a write failure is
    /// ignored, never propagated.
    pub fn save(&self) {
        if let Some(path) = config_path() {
            let _ = self.save_to(&path);
        }
    }

    /// Read settings from a specific path, falling back to defaults when the
    /// file is missing or unparseable.
    fn load_from(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str::<SettingsFile>(&s).ok())
            .map(Settings::from)
            .unwrap_or_default()
    }

    /// Write settings to a specific path, creating parent dirs as needed.
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string(&SettingsFile::from(self)).map_err(std::io::Error::other)?;
        std::fs::write(path, body)
    }
}

/// Nearest preset index to `current`; if `current` already sits on a preset,
/// step one slot in `dir` (clamped at the ends). An off-preset value snaps to
/// the nearest preset without also stepping, so a hand-edited file lands on a
/// valid value on its first cycle. `keys` is assumed non-empty.
fn step_index(keys: &[u64], current: u64, dir: Cycle) -> usize {
    let nearest = keys
        .iter()
        .enumerate()
        .min_by_key(|&(_, &v)| v.abs_diff(current))
        .map(|(i, _)| i)
        .unwrap_or(0);
    if keys[nearest] == current {
        match dir {
            Cycle::Prev => nearest.saturating_sub(1),
            Cycle::Next => (nearest + 1).min(keys.len() - 1),
        }
    } else {
        nearest
    }
}

/// Resolve the config file path: `$XDG_CONFIG_HOME/pokertui/config.toml`,
/// falling back to `$HOME/.config/pokertui/config.toml`.
fn config_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("pokertui").join("config.toml"))
}

/// On-disk form of `Settings`: `Duration` is flattened to integer milliseconds
/// for a clean, human-editable TOML file.
#[derive(Debug, Serialize, Deserialize)]
struct SettingsFile {
    bot_delay_ms: u64,
    show_win_rate: bool,
    seats: usize,
    small_blind: u64,
    big_blind: u64,
}

impl From<&Settings> for SettingsFile {
    fn from(s: &Settings) -> Self {
        Self {
            bot_delay_ms: s.bot_delay.as_millis() as u64,
            show_win_rate: s.show_win_rate,
            seats: s.seats,
            small_blind: s.small_blind,
            big_blind: s.big_blind,
        }
    }
}

impl From<SettingsFile> for Settings {
    fn from(f: SettingsFile) -> Self {
        Self {
            bot_delay: Duration::from_millis(f.bot_delay_ms),
            show_win_rate: f.show_win_rate,
            seats: f.seats,
            small_blind: f.small_blind,
            big_blind: f.big_blind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_the_design_defaults() {
        let s = Settings::default();
        assert_eq!(s.bot_delay, Duration::from_millis(500));
        assert!(s.show_win_rate);
        assert_eq!(s.seats, 6);
        assert_eq!((s.small_blind, s.big_blind), (50, 100));
    }

    #[test]
    fn bot_delay_steps_through_presets() {
        let mut s = Settings::default(); // 500ms
        s.cycle_bot_delay(Cycle::Next);
        assert_eq!(s.bot_delay, Duration::from_millis(700));
        s.cycle_bot_delay(Cycle::Prev);
        assert_eq!(s.bot_delay, Duration::from_millis(500));
        s.cycle_bot_delay(Cycle::Prev);
        assert_eq!(s.bot_delay, Duration::from_millis(250));
    }

    #[test]
    fn bot_delay_clamps_at_both_ends() {
        let mut s = Settings::default();
        // Walk down to the first preset (0ms) and try to go further.
        for _ in 0..10 {
            s.cycle_bot_delay(Cycle::Prev);
        }
        assert_eq!(
            s.bot_delay,
            Duration::from_millis(0),
            "clamps at the first preset"
        );
        // Walk up to the last preset (2000ms) and try to go further.
        for _ in 0..10 {
            s.cycle_bot_delay(Cycle::Next);
        }
        assert_eq!(
            s.bot_delay,
            Duration::from_millis(2000),
            "clamps at the last preset"
        );
    }

    #[test]
    fn seats_step_and_clamp() {
        let mut s = Settings::default(); // 6 seats (last preset)
        s.cycle_seats(Cycle::Next);
        assert_eq!(s.seats, 6, "already at the max, no-op");
        s.cycle_seats(Cycle::Prev);
        assert_eq!(s.seats, 5);
        for _ in 0..10 {
            s.cycle_seats(Cycle::Prev);
        }
        assert_eq!(s.seats, 2, "clamps at the min seat preset");
    }

    #[test]
    fn blinds_step_and_clamp() {
        let mut s = Settings::default(); // 50/100
        s.cycle_blinds(Cycle::Next);
        assert_eq!((s.small_blind, s.big_blind), (100, 200));
        s.cycle_blinds(Cycle::Prev);
        assert_eq!((s.small_blind, s.big_blind), (50, 100));
        for _ in 0..10 {
            s.cycle_blinds(Cycle::Prev);
        }
        assert_eq!(
            (s.small_blind, s.big_blind),
            (25, 50),
            "clamps at the smallest blinds"
        );
        for _ in 0..10 {
            s.cycle_blinds(Cycle::Next);
        }
        assert_eq!(
            (s.small_blind, s.big_blind),
            (200, 400),
            "clamps at the largest blinds"
        );
    }

    #[test]
    fn win_rate_toggles_and_clamps() {
        let mut s = Settings::default(); // on
        s.cycle_show_win_rate(Cycle::Next);
        assert!(!s.show_win_rate, "Right turns the display off");
        s.cycle_show_win_rate(Cycle::Next);
        assert!(!s.show_win_rate, "stays off (clamped)");
        s.cycle_show_win_rate(Cycle::Prev);
        assert!(s.show_win_rate, "Left turns the display back on");
        s.cycle_show_win_rate(Cycle::Prev);
        assert!(s.show_win_rate, "stays on (clamped)");
    }

    #[test]
    fn off_preset_value_snaps_to_nearest_on_first_cycle() {
        let mut s = Settings::default();
        // A hand-edited file could leave 600ms, which is not a preset. The
        // nearest preset is 500ms, so the first cycle snaps there.
        s.bot_delay = Duration::from_millis(600);
        s.cycle_bot_delay(Cycle::Next);
        assert_eq!(s.bot_delay, Duration::from_millis(500));
    }

    #[test]
    fn load_returns_default_when_file_is_missing() {
        let path = unique_tmp_path("missing");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Settings::load_from(&path), Settings::default());
    }

    #[test]
    fn load_returns_default_when_file_is_malformed() {
        let path = unique_tmp_path("malformed");
        std::fs::write(&path, "this is not valid toml = = =").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_then_load_round_trips_every_field() {
        let path = unique_tmp_path("roundtrip");
        let original = Settings {
            bot_delay: Duration::from_millis(2000),
            show_win_rate: false,
            seats: 3,
            small_blind: 100,
            big_blind: 200,
        };
        original.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded, original);
        let _ = std::fs::remove_file(&path);
    }

    /// A unique temp path so parallel test runs don't collide.
    fn unique_tmp_path(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("pokertui-{tag}-{pid}-{n}.toml"))
    }
}
