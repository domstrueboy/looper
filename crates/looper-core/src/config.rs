use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Defaults, and the range each setting is allowed to take. The settings
/// screen builds its sliders from these, and `clamped` holds a
/// hand-edited file to them, so the two can't drift apart.
pub const VOLUME_PCT_RANGE: RangeInclusive<u32> = 0..=200;
pub const DEFAULT_VOLUME_PCT: u32 = 100;

/// The click's level, the same way: 100 is the click as generated, half
/// full scale, so 200 is as loud as a sample gets.
pub const CLICK_VOLUME_PCT_RANGE: RangeInclusive<u32> = 0..=200;
pub const DEFAULT_CLICK_VOLUME_PCT: u32 = 100;

/// Wait before recording starts, with `RecordStart::Delay`. Long enough
/// by default to put a guitar back on and be ready; zero turns it off.
pub const PREROLL_MS_RANGE: RangeInclusive<u32> = 0..=5000;
pub const DEFAULT_PREROLL_MS: u32 = 4000;

/// How loud the input has to get to start a recording, with
/// `RecordStart::Detect`, in dB below full scale. Quiet enough for a
/// clean guitar's first note, loud enough to sit above most noise
/// floors; a high-gain rig may want it higher.
pub const DETECT_DB_RANGE: RangeInclusive<i32> = -60..=-10;
pub const DEFAULT_DETECT_DB: i32 = -40;

/// How long `StopMode::FadeOut` takes to bring the loop to silence.
pub const FADE_MS_RANGE: RangeInclusive<u32> = 500..=10_000;
pub const DEFAULT_FADE_MS: u32 = 3_000;

/// Headroom between the input and output callbacks, to absorb timing
/// jitter. The setting to raise when the log complains about underruns -
/// at the cost of monitoring being that much less immediate.
pub const LATENCY_MS_RANGE: RangeInclusive<u32> = 2..=50;
pub const DEFAULT_LATENCY_MS: u32 = 8;

/// Longest loop that can be recorded. Every layer is pre-allocated at
/// this length, so together with `max_layers` it decides how much memory
/// the app takes: seconds x layers x sample rate x 4 bytes.
pub const MAX_LOOP_SECS_RANGE: RangeInclusive<u32> = 10..=120;
pub const DEFAULT_MAX_LOOP_SECS: u32 = 60;

/// How many layers can be stacked, the first recording included. Eight
/// by default, since a layer per pass uses them up quickly: each is a
/// `max_loop_secs` buffer, ~10 MB at 60 s, held twice (stack and copy).
pub const MAX_LAYERS_RANGE: RangeInclusive<u32> = 1..=8;
pub const DEFAULT_MAX_LAYERS: u32 = 8;

/// How long a control has to be held: the main one to undo a layer, stop
/// to clear the loop.
pub const LONG_PRESS_MS_RANGE: RangeInclusive<u32> = 500..=4000;
pub const DEFAULT_LONG_PRESS_MS: u32 = 2000;

/// Which controls the looper screen shows. Mini is the pedal on its own;
/// extended adds the layer list, the drums and the mic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Mini,
    Extended,
}

/// Metronome tempo. Changed live from the looper screen as well as in
/// Settings.
pub const METRONOME_BPM_RANGE: RangeInclusive<u32> = 40..=300;
pub const DEFAULT_METRONOME_BPM: u32 = 120;

/// Where the main control goes when it ends the first take, and what it
/// does while dubbing. Read from the older `after_take = "play" / "dub"`
/// too, so a config saved before keeps its choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordMode {
    /// The take plays; a press while dubbing keeps the layer and plays.
    #[default]
    #[serde(alias = "play")]
    RecPlay,
    /// The take goes straight on into a dub, and a press while dubbing
    /// keeps the layer and dubs the next - layer after layer without
    /// playing in between.
    #[serde(alias = "dub")]
    RecDub,
    /// As REC→DUB after the take, but the layers split themselves: each
    /// pass round the loop is one, with no press. A press plays.
    RecDubPerPass,
}

impl RecordMode {
    /// Whether ending the take goes straight on into a dub.
    pub fn dubs_after_take(self) -> bool {
        self != Self::RecPlay
    }
}

const CONFIG_DIR: &str = "looper-pedal";
const CONFIG_FILE: &str = "config.toml";
const LOOP_DIR: &str = "loop";

/// Persisted device/rate/input-channel/volume/pre-roll choice, which
/// doubles as the bundle of settings the looper is started with.
///
/// Settings added after the first release carry defaults, so a config
/// written by an older build still loads rather than throwing the user
/// back to the settings screen. The two that identify the device don't:
/// without them there's nothing to open, so such a file counts as no
/// config at all.
#[derive(Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// The input device, and on a duplex backend the output one too.
    /// Named as it always was, so files written before there were two
    /// still load.
    pub device_name: String,
    pub sample_rate: u32,
    /// Which backend `device_name` belongs to. A name alone doesn't
    /// identify a device once there is more than one backend - WASAPI
    /// and ASIO both present the same interface under names of their
    /// own.
    #[serde(default = "default_backend")]
    pub backend: String,
    /// The output device, when it isn't the input one. `None` covers
    /// every duplex backend, and every config written before backends
    /// that split them existed.
    #[serde(default)]
    pub output_device_name: Option<String>,
    /// 0-indexed input channel to capture/record/loop.
    #[serde(default)]
    pub input_channel: u16,
    /// Loop playback gain as a percentage of unity (100 = unchanged).
    /// Applied to the loop only, never the live passthrough.
    #[serde(default = "default_volume_pct")]
    pub volume_pct: u32,
    /// See `CLICK_VOLUME_PCT_RANGE`.
    #[serde(default = "default_click_volume_pct")]
    pub click_volume_pct: u32,
    /// See `RecordStart`.
    #[serde(default)]
    pub record_start: RecordStart,
    /// How long to wait between pressing record and actually capturing,
    /// so there's time to get ready. 0 = start immediately.
    #[serde(default = "default_preroll_ms")]
    pub preroll_ms: u32,
    /// See `DETECT_DB_RANGE`.
    #[serde(default = "default_detect_db")]
    pub detect_db: i32,
    /// See `StopMode`.
    #[serde(default)]
    pub stop_mode: StopMode,
    /// See `FADE_MS_RANGE`.
    #[serde(default = "default_fade_ms")]
    pub fade_ms: u32,
    /// See `LATENCY_MS_RANGE`.
    #[serde(default = "default_latency_ms")]
    pub latency_ms: u32,
    /// See `MAX_LOOP_SECS_RANGE`.
    #[serde(default = "default_max_loop_secs")]
    pub max_loop_secs: u32,
    /// See `MAX_LAYERS_RANGE`.
    #[serde(default = "default_max_layers")]
    pub max_layers: u32,
    /// See `LONG_PRESS_MS_RANGE`.
    #[serde(default = "default_long_press_ms")]
    pub long_press_ms: u32,
    /// See `METRONOME_BPM_RANGE`.
    #[serde(default = "default_metronome_bpm")]
    pub metronome_bpm: u32,
    /// See `TimeSignature`.
    #[serde(default)]
    pub time_signature: TimeSignature,
    /// See `RecordMode`.
    #[serde(default, alias = "after_take")]
    pub record_mode: RecordMode,
    /// See `Mode`. Switched from the looper screen, not from Settings.
    #[serde(default)]
    pub mode: Mode,
}

/// How a first recording starts once Space is pressed. Each is for a
/// different situation: a count-in is at home and wrong on stage, and
/// waiting for the first note can't start a part on an off-beat - so
/// it's a choice, not a fixed way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStart {
    /// The moment Space goes down.
    Instant,
    /// After `preroll_ms`, counted down on screen.
    #[default]
    Delay,
    /// After a bar of clicks, on the bar line - the click plays for it
    /// even with the metronome off.
    CountIn,
    /// When the input first crosses `detect_db`, from just before it.
    Detect,
}

/// The metronome's bar: how many clicks, beat one accented. 6/8 clicks
/// the eighths, so its BPM counts them. It sets a bar's length wherever
/// one is used - the count-in, rounding a take to the bar, a loop's
/// tempo - so it's a setting rather than changed under a loop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeSignature {
    #[default]
    #[serde(rename = "4/4")]
    FourFour,
    #[serde(rename = "3/4")]
    ThreeFour,
    #[serde(rename = "6/8")]
    SixEight,
}

impl TimeSignature {
    pub fn beats_per_bar(self) -> u32 {
        match self {
            Self::FourFour => 4,
            Self::ThreeFour => 3,
            Self::SixEight => 6,
        }
    }
}

/// How a playing loop stops - by Stop, or a double press. Not the end of
/// a first take, which stops at once whatever this says: the loop hasn't
/// played yet, so there's nothing to play out or fade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopMode {
    #[default]
    Immediate,
    /// Plays on to the end of the loop - the next time round starts at
    /// the top, as the song would.
    LoopEnd,
    /// Brings the loop down to silence over `fade_ms`.
    FadeOut,
}

/// What a config written before this field existed meant: those files
/// all came from a build that could only open ASIO.
fn default_backend() -> String {
    "asio".to_string()
}

fn default_volume_pct() -> u32 {
    DEFAULT_VOLUME_PCT
}

fn default_preroll_ms() -> u32 {
    DEFAULT_PREROLL_MS
}

fn default_latency_ms() -> u32 {
    DEFAULT_LATENCY_MS
}

fn default_max_loop_secs() -> u32 {
    DEFAULT_MAX_LOOP_SECS
}

fn default_max_layers() -> u32 {
    DEFAULT_MAX_LAYERS
}

fn default_long_press_ms() -> u32 {
    DEFAULT_LONG_PRESS_MS
}

fn default_metronome_bpm() -> u32 {
    DEFAULT_METRONOME_BPM
}

fn default_detect_db() -> i32 {
    DEFAULT_DETECT_DB
}

fn default_click_volume_pct() -> u32 {
    DEFAULT_CLICK_VOLUME_PCT
}

fn default_fade_ms() -> u32 {
    DEFAULT_FADE_MS
}

impl AppConfig {
    /// Reads the config. `None` means there's nothing usable and the
    /// caller should show the settings picker.
    pub fn load() -> Option<Self> {
        Self::load_from(&config_path())
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&config_path())
    }

    /// The output device, which is the input one unless a backend that
    /// splits them says otherwise.
    pub fn output_device(&self) -> &str {
        self.output_device_name
            .as_deref()
            .unwrap_or(&self.device_name)
    }

    /// Where `load` actually reads from. Split out so the file half can
    /// be tested against a temp directory: `config_path()` is the real
    /// per-user one, and a test that wrote there would clobber the
    /// config of whoever ran it.
    pub fn load_from(path: &Path) -> Option<Self> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| Self::from_toml(&text))
    }

    /// Creates the directory as well: on a first run nothing has made
    /// the per-user config directory yet.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)
                .map_err(|e| format!("creating {}: {e}", directory.display()))?;
        }
        std::fs::write(path, self.to_toml()?)
            .map_err(|e| format!("writing {}: {e}", path.display()))
    }

    fn from_toml(text: &str) -> Option<Self> {
        toml::from_str::<Self>(text).ok().map(Self::clamped)
    }

    /// Holds every setting to its supported range, so a hand-edited file
    /// can't ask for something absurd - a thousand layers of a
    /// two-minute loop would try to allocate tens of gigabytes before the
    /// window even opened.
    fn clamped(mut self) -> Self {
        self.volume_pct = clamp_to(self.volume_pct, VOLUME_PCT_RANGE);
        self.click_volume_pct = clamp_to(self.click_volume_pct, CLICK_VOLUME_PCT_RANGE);
        self.preroll_ms = clamp_to(self.preroll_ms, PREROLL_MS_RANGE);
        self.detect_db = clamp_to(self.detect_db, DETECT_DB_RANGE);
        self.fade_ms = clamp_to(self.fade_ms, FADE_MS_RANGE);
        self.latency_ms = clamp_to(self.latency_ms, LATENCY_MS_RANGE);
        self.max_loop_secs = clamp_to(self.max_loop_secs, MAX_LOOP_SECS_RANGE);
        self.max_layers = clamp_to(self.max_layers, MAX_LAYERS_RANGE);
        self.long_press_ms = clamp_to(self.long_press_ms, LONG_PRESS_MS_RANGE);
        self.metronome_bpm = clamp_to(self.metronome_bpm, METRONOME_BPM_RANGE);
        self
    }

    /// Memory the loop layers will take at `sample_rate`, in bytes: every
    /// layer is pre-allocated at the full loop length, so this is the
    /// price of the two settings that decide it.
    pub fn loop_memory_bytes(&self) -> u64 {
        u64::from(self.max_loop_secs)
            * u64::from(self.max_layers)
            * u64::from(self.sample_rate)
            * size_of::<f32>() as u64
    }

    fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| format!("serializing config: {e}"))
    }
}

impl Default for AppConfig {
    /// Every setting at its default, with no device chosen yet: what the
    /// settings screen starts from when there's nothing saved.
    ///
    /// Deliberately not what `serde` falls back to - `device_name` and
    /// `sample_rate` stay required in the file, because a config without
    /// them names nothing to open. The values come from the same consts
    /// the per-field defaults do, so there's still one declaration each.
    fn default() -> Self {
        Self {
            device_name: String::new(),
            sample_rate: 0,
            // Empty rather than `default_backend`'s "asio", for the same
            // reason `device_name` is: nothing is chosen yet, and the
            // settings screen picks from what this machine actually has.
            // The serde default is about reading an older file, which is
            // a different question.
            backend: String::new(),
            output_device_name: None,
            input_channel: 0,
            volume_pct: DEFAULT_VOLUME_PCT,
            click_volume_pct: DEFAULT_CLICK_VOLUME_PCT,
            record_start: RecordStart::Delay,
            preroll_ms: DEFAULT_PREROLL_MS,
            detect_db: DEFAULT_DETECT_DB,
            stop_mode: StopMode::Immediate,
            fade_ms: DEFAULT_FADE_MS,
            latency_ms: DEFAULT_LATENCY_MS,
            max_loop_secs: DEFAULT_MAX_LOOP_SECS,
            max_layers: DEFAULT_MAX_LAYERS,
            long_press_ms: DEFAULT_LONG_PRESS_MS,
            metronome_bpm: DEFAULT_METRONOME_BPM,
            time_signature: TimeSignature::FourFour,
            record_mode: RecordMode::RecPlay,
            mode: Mode::Mini,
        }
    }
}

fn config_path() -> PathBuf {
    config_path_in(&app_dir())
}

/// The config file inside `dir`. Taking the directory lets a test point
/// it somewhere harmless; `app_dir()` is the real one.
pub fn config_path_in(dir: &Path) -> PathBuf {
    dir.join(CONFIG_FILE)
}

/// The recorded loop, one WAV per layer, kept beside the config.
pub fn loop_dir_in(dir: &Path) -> PathBuf {
    dir.join(LOOP_DIR)
}

/// The per-user config directory - `%APPDATA%\looper-pedal` on Windows,
/// `~/.config/looper-pedal` on Linux, `~/Library/Application
/// Support/looper-pedal` on macOS. Next to the executable stopped being
/// the right place because it isn't writable once the app is installed
/// somewhere like Program Files; that's only the fallback for having no
/// home directory at all.
pub fn app_dir() -> PathBuf {
    match directories::BaseDirs::new() {
        Some(dirs) => dirs.config_dir().join(CONFIG_DIR),
        None => exe_dir(),
    }
}

fn clamp_to<T: Ord + Copy>(value: T, range: RangeInclusive<T>) -> T {
    value.clamp(*range.start(), *range.end())
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
