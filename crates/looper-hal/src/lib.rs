//! The audio devices an app can open, and nothing about what it plays
//! through them.
//!
//! This crate knows about devices, formats and callbacks; it has never
//! heard of loops, layers or takes. That is the whole point of it being
//! separate: porting to another operating system means writing a
//! `AudioBackend` here, not touching anything that decides what the app
//! does.
//!
//! Samples cross this boundary as `f32` with full scale at +/-1.0,
//! whatever the device's own format is. Converting is the backend's job,
//! so no format assumption reaches the code above.

use std::fmt;
use std::sync::Arc;

#[cfg(feature = "cpal")]
pub mod cpal_backend;
pub mod mock;

/// The most frames a processor is handed in one call, so every buffer it
/// needs can be taken up front rather than inside a real-time callback.
///
/// A statement about what a driver might hand us, not about how any app
/// wants to behave - which is why it lives down here.
pub const MAX_BLOCK_FRAMES: usize = 32_768;

/// The rates worth offering, filtered per device by `caps`. A device
/// concern, so it lives with the devices.
pub const CANDIDATE_SAMPLE_RATES: [u32; 5] = [44_100, 48_000, 88_200, 96_000, 192_000];

/// Which backend a device came from.
///
/// A newtype over a fixed string rather than an enum: it is what a
/// config file stores, and resolving a name back to a backend is the
/// registry's job, so nothing here needs to know the whole list.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BackendId(pub &'static str);

impl BackendId {
    pub const ASIO: Self = Self("asio");
    pub const WASAPI: Self = Self("wasapi");
    pub const COREAUDIO: Self = Self("coreaudio");
    pub const ALSA: Self = Self("alsa");
    pub const JACK: Self = Self("jack");
    pub const MOCK: Self = Self("mock");

    pub fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for BackendId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Which way audio flows through a device.
///
/// ASIO hands out one device that does both. WASAPI does not: there, a
/// device is a single endpoint, and asking a render endpoint to capture
/// silently records whatever the speakers are playing rather than
/// failing. Naming the direction is what keeps that unrepresentable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Input,
    Output,
    Duplex,
}

impl Direction {
    pub fn can_capture(self) -> bool {
        matches!(self, Self::Input | Self::Duplex)
    }

    pub fn can_play(self) -> bool {
        matches!(self, Self::Output | Self::Duplex)
    }
}

/// A device, as something that can be written to a config file and
/// looked up again later.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceId {
    pub backend: BackendId,
    pub name: String,
}

impl DeviceId {
    pub fn new(backend: BackendId, name: impl Into<String>) -> Self {
        Self {
            backend,
            name: name.into(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub direction: Direction,
}

/// What a device will agree to. Channel counts are zero for a direction
/// it doesn't do at all.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DeviceCaps {
    pub sample_rates: Vec<u32>,
    pub input_channels: u16,
    pub output_channels: u16,
}

/// Names the same device twice on a duplex backend, and two endpoints on
/// one that splits them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StreamRequest {
    pub input: DeviceId,
    pub output: DeviceId,
    pub sample_rate: u32,
}

/// What the device actually agreed to, which is not always what was
/// asked for - a shared-mode endpoint hands back its own mix rate
/// whatever the request said. Read this rather than assuming the
/// request was honoured.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub input_channels: u16,
    pub output_channels: u16,
    /// Never more than this many frames per `process` call.
    pub max_block_frames: usize,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HalError {
    /// The backend itself isn't there - no driver installed, or no
    /// subsystem to talk to.
    BackendUnavailable {
        backend: BackendId,
        detail: String,
    },
    DeviceNotFound {
        backend: BackendId,
        device: String,
    },
    /// An input from one backend and an output from another. They would
    /// have separate clocks and no way to agree on a buffer size.
    CrossBackend {
        input: BackendId,
        output: BackendId,
    },
    /// The device is there but wouldn't say what it supports.
    Enumeration(String),
    /// It answered, and the answer rules out what was asked for.
    UnsupportedConfig(String),
    OpenFailed(String),
    StartFailed(String),
    /// Reported by a running stream, after it started.
    Runtime(String),
}

impl fmt::Display for HalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BackendUnavailable { backend, detail } => {
                write!(f, "{backend} is unavailable: {detail}")
            }
            Self::DeviceNotFound { backend, device } => {
                write!(f, "{backend} device '{device}' not found")
            }
            Self::CrossBackend { input, output } => write!(
                f,
                "input is {input} and output is {output}; they must be the same"
            ),
            Self::Enumeration(detail) => {
                write!(f, "could not list what the device supports: {detail}")
            }
            Self::UnsupportedConfig(detail) => write!(f, "{detail}"),
            Self::OpenFailed(detail) => write!(f, "could not open the device: {detail}"),
            Self::StartFailed(detail) => write!(f, "could not start the stream: {detail}"),
            Self::Runtime(detail) => write!(f, "stream error: {detail}"),
        }
    }
}

impl std::error::Error for HalError {}

pub type HalResult<T> = Result<T, HalError>;

/// Runs on the driver's capture thread. Interleaved frames, already in
/// `f32`, at most `max_block_frames` frames at a time.
///
/// Real-time: it must not allocate, lock, block or do I/O.
pub trait InputProcessor: Send + 'static {
    fn process(&mut self, input: &[f32]);
}

/// Runs on the driver's render thread, under the same rules. Fill the
/// buffer; the backend converts it to whatever the device wants.
pub trait OutputProcessor: Send + 'static {
    fn process(&mut self, output: &mut [f32]);
}

/// Where a stream reports trouble once it is running.
///
/// `Arc` rather than `Box` because one sink has to reach both the
/// capture and the render stream, which are separate objects. It may be
/// called from a real-time thread, so it must not block - setting a flag
/// for the UI to notice is the shape that works.
pub type ErrorSink = Arc<dyn Fn(HalError) + Send + Sync + 'static>;

pub trait AudioBackend {
    fn id(&self) -> BackendId;

    /// What to call it on screen: "ASIO", "WASAPI".
    fn label(&self) -> &str;

    fn devices(&self) -> HalResult<Vec<DeviceInfo>>;

    /// Takes the whole `DeviceInfo` rather than its id: a name alone
    /// does not identify an endpoint. WASAPI presents an interface's
    /// capture and render halves under the *same* name, so asking by
    /// name answers about whichever came first in the list.
    fn caps(&self, device: &DeviceInfo) -> HalResult<DeviceCaps>;

    /// Negotiate, and stop there. Nothing is running yet and no callback
    /// has been handed over.
    fn open(&self, request: &StreamRequest) -> HalResult<Box<dyn OpenDevice>>;
}

/// A negotiated device pair that hasn't started.
///
/// This half-open step exists because the caller cannot build its
/// processors until it knows the format: buffer sizes, delay lines and
/// anything measured in samples all depend on the rate and channel
/// counts that were actually granted. Handing the processors to `open`
/// would mean sizing them against a guess.
pub trait OpenDevice {
    fn format(&self) -> StreamFormat;

    /// Take the processors and start. Everything the callbacks need is
    /// allocated here, on this thread, before any of them runs.
    fn start(
        self: Box<Self>,
        input: Box<dyn InputProcessor>,
        output: Box<dyn OutputProcessor>,
        on_error: ErrorSink,
    ) -> HalResult<Box<dyn AudioStream>>;
}

/// A running pair of streams. Dropping it stops the audio.
///
/// Deliberately not `Send`: some backends' stream handles aren't, and
/// the handle is only ever held and dropped by whoever opened it.
pub trait AudioStream {
    fn format(&self) -> StreamFormat;
}

/// The backends this build can offer, best first.
///
/// Holding them together is what lets the app show one device list
/// across all of them and resolve a name out of a config file without
/// knowing which backends exist.
pub struct Backends {
    backends: Vec<Box<dyn AudioBackend>>,
}

impl Backends {
    pub fn new(backends: Vec<Box<dyn AudioBackend>>) -> Self {
        Self { backends }
    }

    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn AudioBackend> {
        self.backends.iter().map(AsRef::as_ref)
    }

    pub fn get(&self, id: BackendId) -> Option<&dyn AudioBackend> {
        self.iter().find(|backend| backend.id() == id)
    }

    /// Resolves what a config file stored. `None` means that backend
    /// isn't in this build or on this machine.
    pub fn by_name(&self, name: &str) -> Option<&dyn AudioBackend> {
        self.iter().find(|backend| backend.id().as_str() == name)
    }

    /// Every device across every backend.
    ///
    /// A backend that fails to enumerate is skipped rather than
    /// surfaced: with no ASIO driver installed, asking ASIO is expected
    /// to fail, and letting that failure through would hide every WASAPI
    /// device behind an error about a driver the user hasn't got.
    pub fn devices(&self) -> Vec<DeviceInfo> {
        self.iter()
            .filter_map(|backend| backend.devices().ok())
            .flatten()
            .collect()
    }

    pub fn open(&self, request: &StreamRequest) -> HalResult<Box<dyn OpenDevice>> {
        if request.input.backend != request.output.backend {
            return Err(HalError::CrossBackend {
                input: request.input.backend,
                output: request.output.backend,
            });
        }
        let backend = request.input.backend;
        self.get(backend)
            .ok_or(HalError::BackendUnavailable {
                backend,
                detail: "not built into this app".to_string(),
            })?
            .open(request)
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
