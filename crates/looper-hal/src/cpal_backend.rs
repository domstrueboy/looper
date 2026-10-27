//! A backend over one of `cpal`'s hosts.
//!
//! Generic over the host, because ASIO and WASAPI differ in what they
//! offer rather than in how they are driven. What they do *not* share is
//! how devices are shaped: an ASIO device does both directions, a WASAPI
//! one is a single endpoint. `Direction` carries that difference, and
//! `open` looks the two ends up separately, so neither case is special
//! here.
//!
//! This is also where every sample format stops. `cpal`'s own conversion
//! traits do the arithmetic - they carry each format's full-scale
//! convention, including the packed 24-bit ones a hand-written version
//! would get wrong.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::{
    AudioBackend, AudioStream, BackendId, CANDIDATE_SAMPLE_RATES, DeviceCaps, DeviceId, DeviceInfo,
    Direction, ErrorSink, HalError, HalResult, InputProcessor, MAX_BLOCK_FRAMES, OpenDevice,
    OutputProcessor, StreamFormat, StreamRequest,
};

/// The formats worth handling, best first. `F32` leads because it is
/// what the path above already works in, so it costs no conversion at
/// all.
const SUPPORTED_FORMATS: [SampleFormat; 4] = [
    SampleFormat::F32,
    SampleFormat::I32,
    SampleFormat::I24,
    SampleFormat::I16,
];

pub struct CpalBackend {
    id: BackendId,
    host_id: cpal::HostId,
    label: &'static str,
}

impl CpalBackend {
    pub fn new(id: BackendId, host_id: cpal::HostId, label: &'static str) -> Self {
        Self { id, host_id, label }
    }

    #[cfg(all(windows, feature = "asio"))]
    pub fn asio() -> Self {
        Self::new(BackendId::ASIO, cpal::HostId::Asio, "ASIO")
    }

    #[cfg(windows)]
    pub fn wasapi() -> Self {
        Self::new(BackendId::WASAPI, cpal::HostId::Wasapi, "WASAPI")
    }

    /// Resolved per call rather than held: a driver can be installed,
    /// removed or claimed by something else between one call and the
    /// next, and a stale handle would hide that.
    fn host(&self) -> HalResult<cpal::Host> {
        cpal::host_from_id(self.host_id).map_err(|e| HalError::BackendUnavailable {
            backend: self.id,
            detail: e.to_string(),
        })
    }

    fn enumerate(&self) -> HalResult<Vec<cpal::Device>> {
        Ok(self
            .host()?
            .devices()
            .map_err(|e| HalError::Enumeration(e.to_string()))?
            .collect())
    }

    /// The named device, if it does what `wanted` needs. Direction is
    /// part of the lookup because a name can belong to two endpoints -
    /// the capture and the render half of one interface - and picking
    /// the wrong one is silent rather than an error.
    fn find(&self, id: &DeviceId, wanted: Direction) -> HalResult<cpal::Device> {
        self.enumerate()?
            .into_iter()
            .find(|device| {
                device.to_string() == id.name
                    && match wanted {
                        Direction::Input => device.supports_input(),
                        Direction::Output => device.supports_output(),
                        Direction::Duplex => device.supports_input() && device.supports_output(),
                    }
            })
            .ok_or_else(|| HalError::DeviceNotFound {
                backend: self.id,
                device: id.name.clone(),
            })
    }
}

fn direction_of(device: &cpal::Device) -> Option<Direction> {
    match (device.supports_input(), device.supports_output()) {
        (true, true) => Some(Direction::Duplex),
        (true, false) => Some(Direction::Input),
        (false, true) => Some(Direction::Output),
        (false, false) => None,
    }
}

/// The rates from our candidate list this device will actually take.
///
/// Deliberately not filtered by sample format: the backend converts
/// whatever the device reports, so ruling a rate out because it is
/// offered in the "wrong" format would hide devices that work perfectly
/// well - which is what filtering on i32 used to do to every device that
/// wasn't the one this app was written against.
fn rates(device: &cpal::Device, direction: Direction) -> Vec<u32> {
    let mut supported: Vec<cpal::SupportedStreamConfigRange> = Vec::new();
    if direction.can_capture() {
        supported.extend(device.supported_input_configs().into_iter().flatten());
    }
    if direction.can_play() {
        supported.extend(device.supported_output_configs().into_iter().flatten());
    }
    CANDIDATE_SAMPLE_RATES
        .into_iter()
        .filter(|&rate| supported.iter().any(|c| c.contains_rate(rate)))
        .collect()
}

/// Picks a config at `sample_rate` carrying every channel the device has.
///
/// The channel count has to be matched explicitly: the supported list
/// holds a separate entry per channel count at each rate, so taking the
/// first match would quietly open a mono entry on a stereo interface.
fn negotiate(
    device: &cpal::Device,
    sample_rate: u32,
    capture: bool,
) -> HalResult<(cpal::StreamConfig, SampleFormat)> {
    let default = if capture {
        device.default_input_config()
    } else {
        device.default_output_config()
    }
    .map_err(|e| HalError::Enumeration(e.to_string()))?;
    let channels = default.channels();

    let ranges: Vec<_> = if capture {
        device.supported_input_configs().map(Iterator::collect)
    } else {
        device.supported_output_configs().map(Iterator::collect)
    }
    .map_err(|e| HalError::Enumeration(e.to_string()))?;

    // The device's own default format first, then whatever else we can
    // convert - a device is likeliest to be happiest in the format it
    // volunteered.
    let preferred = std::iter::once(default.sample_format())
        .chain(SUPPORTED_FORMATS)
        .collect::<Vec<_>>();

    let chosen = preferred.iter().find_map(|&format| {
        if !SUPPORTED_FORMATS.contains(&format) {
            return None;
        }
        ranges
            .iter()
            .find(|range| {
                range.sample_format() == format
                    && range.channels() == channels
                    && range.contains_rate(sample_rate)
            })
            .map(|range| (*range, format))
    });

    let (range, format) = chosen.ok_or_else(|| {
        HalError::UnsupportedConfig(format!(
            "'{device_name}' does not offer {sample_rate} Hz on {channels} channel(s) \
             in a format this app can read",
            device_name = device,
        ))
    })?;

    Ok((range.with_sample_rate(sample_rate).into(), format))
}

impl AudioBackend for CpalBackend {
    fn id(&self) -> BackendId {
        self.id
    }

    fn label(&self) -> &str {
        self.label
    }

    fn devices(&self) -> HalResult<Vec<DeviceInfo>> {
        Ok(self
            .enumerate()?
            .into_iter()
            .filter_map(|device| {
                let direction = direction_of(&device)?;
                Some(DeviceInfo {
                    id: DeviceId::new(self.id, device.to_string()),
                    direction,
                })
            })
            .collect())
    }

    fn caps(&self, info: &DeviceInfo) -> HalResult<DeviceCaps> {
        // Looked up with the direction, not by name: see the trait.
        let device = self.find(&info.id, info.direction)?;
        let direction = info.direction;

        Ok(DeviceCaps {
            sample_rates: rates(&device, direction),
            input_channels: direction
                .can_capture()
                .then(|| device.default_input_config().map(|c| c.channels()).ok())
                .flatten()
                .unwrap_or(0),
            output_channels: direction
                .can_play()
                .then(|| device.default_output_config().map(|c| c.channels()).ok())
                .flatten()
                .unwrap_or(0),
        })
    }

    fn open(&self, request: &StreamRequest) -> HalResult<Box<dyn OpenDevice>> {
        // `Direction::Input` here means "can capture", so a duplex
        // device qualifies as well as a capture-only endpoint.
        let input_device = self.find(&request.input, Direction::Input)?;

        // ASIO's duplex streams must be built from ONE handle, or they
        // silently drop audio. But naming the same device twice does not
        // by itself mean duplex: WASAPI presents an interface's capture
        // and render halves as separate devices under the *same name*,
        // so what settles it is whether the device we found does both.
        let output_device = if request.input == request.output && input_device.supports_output() {
            None
        } else {
            Some(self.find(&request.output, Direction::Output)?)
        };

        let (input_config, input_format) = negotiate(&input_device, request.sample_rate, true)?;
        let (output_config, output_format) = negotiate(
            output_device.as_ref().unwrap_or(&input_device),
            request.sample_rate,
            false,
        )?;

        Ok(Box::new(CpalOpenDevice {
            format: StreamFormat {
                sample_rate: input_config.sample_rate,
                input_channels: input_config.channels,
                output_channels: output_config.channels,
                max_block_frames: MAX_BLOCK_FRAMES,
            },
            input_device,
            output_device,
            input_config,
            input_format,
            output_config,
            output_format,
        }))
    }
}

struct CpalOpenDevice {
    format: StreamFormat,
    input_device: cpal::Device,
    /// `None` when one handle serves both directions.
    output_device: Option<cpal::Device>,
    input_config: cpal::StreamConfig,
    input_format: SampleFormat,
    output_config: cpal::StreamConfig,
    output_format: SampleFormat,
}

impl OpenDevice for CpalOpenDevice {
    fn format(&self) -> StreamFormat {
        self.format
    }

    fn start(
        self: Box<Self>,
        input: Box<dyn InputProcessor>,
        output: Box<dyn OutputProcessor>,
        on_error: ErrorSink,
    ) -> HalResult<Box<dyn AudioStream>> {
        let channels = self.format.input_channels as usize;
        let input_stream = match self.input_format {
            SampleFormat::F32 => capture::<f32>(
                &self.input_device,
                &self.input_config,
                input,
                channels,
                &on_error,
            ),
            SampleFormat::I32 => capture::<i32>(
                &self.input_device,
                &self.input_config,
                input,
                channels,
                &on_error,
            ),
            SampleFormat::I24 => capture::<cpal::I24>(
                &self.input_device,
                &self.input_config,
                input,
                channels,
                &on_error,
            ),
            SampleFormat::I16 => capture::<i16>(
                &self.input_device,
                &self.input_config,
                input,
                channels,
                &on_error,
            ),
            other => Err(unsupported(other)),
        }?;

        let render_device = self.output_device.as_ref().unwrap_or(&self.input_device);
        let channels = self.format.output_channels as usize;
        let output_stream = match self.output_format {
            SampleFormat::F32 => render::<f32>(
                render_device,
                &self.output_config,
                output,
                channels,
                &on_error,
            ),
            SampleFormat::I32 => render::<i32>(
                render_device,
                &self.output_config,
                output,
                channels,
                &on_error,
            ),
            SampleFormat::I24 => render::<cpal::I24>(
                render_device,
                &self.output_config,
                output,
                channels,
                &on_error,
            ),
            SampleFormat::I16 => render::<i16>(
                render_device,
                &self.output_config,
                output,
                channels,
                &on_error,
            ),
            other => Err(unsupported(other)),
        }?;

        input_stream
            .play()
            .map_err(|e| HalError::StartFailed(e.to_string()))?;
        output_stream
            .play()
            .map_err(|e| HalError::StartFailed(e.to_string()))?;

        Ok(Box::new(CpalStream {
            _input: input_stream,
            _output: output_stream,
            format: self.format,
        }))
    }
}

fn unsupported(format: SampleFormat) -> HalError {
    HalError::UnsupportedConfig(format!(
        "sample format {format:?} is not one this app can read"
    ))
}

fn sink_for(on_error: &ErrorSink) -> impl FnMut(cpal::Error) + Send + 'static {
    let sink = on_error.clone();
    move |e: cpal::Error| sink(HalError::Runtime(e.to_string()))
}

/// The capture callback: convert the driver's buffer into `f32` and hand
/// it over, a bounded chunk at a time.
///
/// The scratch is taken here, before the closure exists, because the
/// callback itself may not allocate. Chunking is what makes that
/// possible - it is also the bound every processor is promised.
fn capture<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut processor: Box<dyn InputProcessor>,
    channels: usize,
    on_error: &ErrorSink,
) -> HalResult<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let mut scratch = vec![0.0f32; MAX_BLOCK_FRAMES * channels.max(1)];
    device
        .build_input_stream(
            *config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                for chunk in data.chunks(scratch.len()) {
                    let converted = &mut scratch[..chunk.len()];
                    for (slot, &raw) in converted.iter_mut().zip(chunk) {
                        *slot = raw.to_sample();
                    }
                    processor.process(converted);
                }
            },
            sink_for(on_error),
            None,
        )
        .map_err(|e| HalError::OpenFailed(e.to_string()))
}

/// The render callback, the other way about.
///
/// Clamping here is what makes everything upstream free to run past full
/// scale: a four-layer stack sums well over 1.0 and stays that way until
/// this point, so turning the loop down still rescues it. Converting an
/// out-of-range value without clamping is how a loud loop would come out
/// as noise.
fn render<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut processor: Box<dyn OutputProcessor>,
    channels: usize,
    on_error: &ErrorSink,
) -> HalResult<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let mut scratch = vec![0.0f32; MAX_BLOCK_FRAMES * channels.max(1)];
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for chunk in data.chunks_mut(scratch.len()) {
                    let rendered = &mut scratch[..chunk.len()];
                    processor.process(rendered);
                    for (slot, &value) in chunk.iter_mut().zip(rendered.iter()) {
                        *slot = T::from_sample(value.clamp(-1.0, 1.0));
                    }
                }
            },
            sink_for(on_error),
            None,
        )
        .map_err(|e| HalError::OpenFailed(e.to_string()))
}

struct CpalStream {
    _input: cpal::Stream,
    _output: cpal::Stream,
    format: StreamFormat,
}

impl AudioStream for CpalStream {
    fn format(&self) -> StreamFormat {
        self.format
    }
}

/// Every backend this build offers on this machine, best first.
// Built by pushing rather than as a list literal: which backends exist
// is decided by `cfg`, and attributes on list elements are not stable.
#[allow(clippy::vec_init_then_push)]
pub fn default_backends() -> Vec<Box<dyn AudioBackend>> {
    #[allow(unused_mut)]
    let mut backends: Vec<Box<dyn AudioBackend>> = Vec::new();

    // ASIO first where it exists: it is the only one of these that gives
    // a guitarist a latency they can play through.
    #[cfg(all(windows, feature = "asio"))]
    backends.push(Box::new(CpalBackend::asio()));

    // Always there on Windows and needing no driver of its own, but
    // shared mode only - an order more latency, and the two halves of an
    // interface arrive as separate endpoints. Running at all on a
    // machine with no ASIO driver is the point of it.
    #[cfg(windows)]
    backends.push(Box::new(CpalBackend::wasapi()));

    backends
}
