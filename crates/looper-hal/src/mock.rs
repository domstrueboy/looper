//! A backend with no device behind it, driven by whoever is testing.
//!
//! Always compiled, and costing no dependencies, because it is the only
//! way to drive the whole path - processors, start, stop - without an
//! audio interface plugged in. Opening a real device in a test needs
//! hardware, a driver, and an exclusive claim on it that a second test
//! would fight over.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::{
    AudioBackend, AudioStream, BackendId, CANDIDATE_SAMPLE_RATES, DeviceCaps, DeviceId, DeviceInfo,
    Direction, ErrorSink, HalError, HalResult, InputProcessor, MAX_BLOCK_FRAMES, OpenDevice,
    OutputProcessor, StreamFormat, StreamRequest,
};

/// The one device this backend reports.
pub const DEVICE: &str = "Mock Device";

struct Running {
    input: Box<dyn InputProcessor>,
    output: Box<dyn OutputProcessor>,
    /// Kept so a test can make the stream fail the way a device being
    /// unplugged does.
    on_error: ErrorSink,
}

#[derive(Default)]
struct Shared {
    running: Option<Running>,
    /// Counts every `start` so a test can tell "never started" from
    /// "started and then stopped".
    starts: usize,
}

/// How a backend presents an interface.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// One device, both directions - ASIO.
    Duplex,
    /// Two devices under the same name, one each way, with rate lists
    /// that don't match - WASAPI.
    Split,
}

/// A mock backend, and the handle that pretends to be its driver. Both
/// refer to the same stream, so starting one is visible from the other.
pub fn mock() -> (MockBackend, MockDriver) {
    mock_with(1, 1)
}

/// A backend shaped the way WASAPI is: the interface's capture and
/// render halves arrive as separate devices sharing a name, and only the
/// capture end is honest about its rates - the render end claims a range
/// it would have to resample to reach.
pub fn split() -> (MockBackend, MockDriver) {
    let (mut backend, driver) = mock_with(2, 2);
    backend.shape = Shape::Split;
    (backend, driver)
}

/// The only rate a split backend's capture end will take: its own mix
/// rate, whatever its render half advertises.
pub const SPLIT_CAPTURE_RATE: u32 = 44_100;

/// The same, with the channel counts a test wants. The device reports
/// them as one duplex device, the way ASIO does.
pub fn mock_with(input_channels: u16, output_channels: u16) -> (MockBackend, MockDriver) {
    let shared = Rc::new(RefCell::new(Shared::default()));
    let backend = MockBackend {
        shared: Rc::clone(&shared),
        input_channels,
        output_channels,
        granted_rate: None,
        shape: Shape::Duplex,
    };
    let driver = MockDriver {
        shared,
        input_channels,
        output_channels,
    };
    (backend, driver)
}

pub struct MockBackend {
    shared: Rc<RefCell<Shared>>,
    input_channels: u16,
    output_channels: u16,
    granted_rate: Option<u32>,
    shape: Shape,
}

impl MockBackend {
    /// Grant this rate whatever was asked for, the way a shared-mode
    /// endpoint hands back its own mix rate. For proving that a caller
    /// reads the granted format instead of assuming its request stood.
    pub fn always_grants(mut self, sample_rate: u32) -> Self {
        self.granted_rate = Some(sample_rate);
        self
    }

    fn device_id() -> DeviceId {
        DeviceId::new(BackendId::MOCK, DEVICE)
    }
}

impl AudioBackend for MockBackend {
    fn id(&self) -> BackendId {
        BackendId::MOCK
    }

    fn label(&self) -> &str {
        "Mock"
    }

    fn devices(&self) -> HalResult<Vec<DeviceInfo>> {
        let directions: &[Direction] = match self.shape {
            Shape::Duplex => &[Direction::Duplex],
            // Same name both times, which is the trap: a lookup by name
            // alone finds whichever comes first.
            Shape::Split => &[Direction::Output, Direction::Input],
        };
        Ok(directions
            .iter()
            .map(|&direction| DeviceInfo {
                id: Self::device_id(),
                direction,
            })
            .collect())
    }

    fn caps(&self, device: &DeviceInfo) -> HalResult<DeviceCaps> {
        if device.id.name != DEVICE {
            return Err(HalError::DeviceNotFound {
                backend: BackendId::MOCK,
                device: device.id.name.clone(),
            });
        }
        Ok(match (self.shape, device.direction) {
            (Shape::Split, Direction::Input) => DeviceCaps {
                sample_rates: vec![SPLIT_CAPTURE_RATE],
                input_channels: self.input_channels,
                output_channels: 0,
            },
            (Shape::Split, _) => DeviceCaps {
                sample_rates: CANDIDATE_SAMPLE_RATES.to_vec(),
                input_channels: 0,
                output_channels: self.output_channels,
            },
            _ => DeviceCaps {
                sample_rates: CANDIDATE_SAMPLE_RATES.to_vec(),
                input_channels: self.input_channels,
                output_channels: self.output_channels,
            },
        })
    }

    fn open(&self, request: &StreamRequest) -> HalResult<Box<dyn OpenDevice>> {
        for device in [&request.input, &request.output] {
            if device.name != DEVICE {
                return Err(HalError::DeviceNotFound {
                    backend: BackendId::MOCK,
                    device: device.name.clone(),
                });
            }
        }
        let sample_rate = match self.granted_rate {
            Some(rate) => rate,
            None if CANDIDATE_SAMPLE_RATES.contains(&request.sample_rate) => request.sample_rate,
            None => {
                return Err(HalError::UnsupportedConfig(format!(
                    "{} Hz is not supported by '{DEVICE}'",
                    request.sample_rate
                )));
            }
        };

        Ok(Box::new(MockOpenDevice {
            shared: Rc::clone(&self.shared),
            format: StreamFormat {
                sample_rate,
                input_channels: self.input_channels,
                output_channels: self.output_channels,
                max_block_frames: MAX_BLOCK_FRAMES,
            },
        }))
    }
}

struct MockOpenDevice {
    shared: Rc<RefCell<Shared>>,
    format: StreamFormat,
}

impl OpenDevice for MockOpenDevice {
    fn format(&self) -> StreamFormat {
        self.format
    }

    fn start(
        self: Box<Self>,
        input: Box<dyn InputProcessor>,
        output: Box<dyn OutputProcessor>,
        on_error: ErrorSink,
    ) -> HalResult<Box<dyn AudioStream>> {
        let mut shared = self.shared.borrow_mut();
        shared.running = Some(Running {
            input,
            output,
            on_error,
        });
        shared.starts += 1;
        drop(shared);

        Ok(Box::new(MockStream {
            shared: self.shared,
            format: self.format,
        }))
    }
}

struct MockStream {
    shared: Rc<RefCell<Shared>>,
    format: StreamFormat,
}

impl AudioStream for MockStream {
    fn format(&self) -> StreamFormat {
        self.format
    }
}

/// Dropping the handle stops the audio, which is the contract every
/// backend keeps - so the mock has to keep it too, or a test would pass
/// where a real device leaves a stream running.
impl Drop for MockStream {
    fn drop(&mut self) {
        self.shared.borrow_mut().running = None;
    }
}

/// The driver side: what a test calls to make callbacks happen.
pub struct MockDriver {
    shared: Rc<RefCell<Shared>>,
    input_channels: u16,
    output_channels: u16,
}

impl MockDriver {
    /// One buffer switch: the capture callback, then the render one, in
    /// the order a driver runs them. `input` is interleaved; so is what
    /// comes back.
    ///
    /// Panics if nothing is running, because a test that meant to drive
    /// a stream and silently drove nothing would pass for the wrong
    /// reason.
    pub fn cycle(&self, input: &[f32]) -> Vec<f32> {
        let frames = input.len() / self.input_channels as usize;
        let mut output = vec![0.0; frames * self.output_channels as usize];

        let mut shared = self.shared.borrow_mut();
        let running = shared
            .running
            .as_mut()
            .expect("cycle() with no stream running");

        // Chunked the way a real backend chunks, so a processor that
        // relies on never seeing more than `max_block_frames` is held to
        // the same bound here as on a device.
        let in_block = MAX_BLOCK_FRAMES * self.input_channels as usize;
        let out_block = MAX_BLOCK_FRAMES * self.output_channels as usize;
        for (captured, rendered) in input.chunks(in_block).zip(output.chunks_mut(out_block)) {
            running.input.process(captured);
            running.output.process(rendered);
        }
        output
    }

    /// The capture callback on its own, with nothing draining what it
    /// produces - which is what one side falling behind the other looks
    /// like from in here, and the only way to provoke an underrun
    /// without a device that is genuinely struggling.
    pub fn capture(&self, input: &[f32]) {
        let mut shared = self.shared.borrow_mut();
        let running = shared
            .running
            .as_mut()
            .expect("capture() with no stream running");
        for chunk in input.chunks(MAX_BLOCK_FRAMES * self.input_channels as usize) {
            running.input.process(chunk);
        }
    }

    /// Makes the running stream report trouble, the way a device being
    /// unplugged mid-loop does. Does nothing if nothing is running.
    pub fn fail(&self, error: HalError) {
        // Cloned out before calling: the sink may well reach back into
        // whatever owns this, and holding the borrow across it would
        // panic rather than deliver.
        let sink = self
            .shared
            .borrow()
            .running
            .as_ref()
            .map(|running| Arc::clone(&running.on_error));
        if let Some(sink) = sink {
            sink(error);
        }
    }

    /// Whether a stream is open right now.
    pub fn is_running(&self) -> bool {
        self.shared.borrow().running.is_some()
    }

    /// How many times a stream has been started, so "never opened" can
    /// be told from "opened and closed again".
    pub fn starts(&self) -> usize {
        self.shared.borrow().starts
    }

    /// A sink that files errors here for a test to read back.
    pub fn error_sink(&self) -> (ErrorSink, ErrorLog) {
        let log = ErrorLog::default();
        let sink = log.clone();
        (
            Arc::new(move |error| sink.0.lock().expect("not poisoned").push(error)),
            log,
        )
    }
}

/// Errors a stream reported, for a test to assert on. Behind a mutex
/// rather than a `RefCell` because `ErrorSink` has to be `Send + Sync`.
#[derive(Clone, Default)]
pub struct ErrorLog(std::sync::Arc<std::sync::Mutex<Vec<HalError>>>);

impl ErrorLog {
    pub fn taken(&self) -> Vec<HalError> {
        std::mem::take(&mut *self.0.lock().expect("not poisoned"))
    }
}

/// A request naming the mock device both ways, which is what a duplex
/// backend gets handed.
pub fn request(sample_rate: u32) -> StreamRequest {
    StreamRequest {
        input: DeviceId::new(BackendId::MOCK, DEVICE),
        output: DeviceId::new(BackendId::MOCK, DEVICE),
        sample_rate,
    }
}

#[cfg(test)]
#[path = "mock_tests.rs"]
mod tests;
