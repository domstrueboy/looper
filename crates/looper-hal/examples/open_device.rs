//! Opens a device for real, runs it, and reports whether the two ends
//! keep time with each other.
//!
//! Enumeration can be checked from `list_devices`; claiming a driver and
//! having both callbacks fire cannot, and that is the half that breaks.
//! The number that matters is the drift: the looper's whole alignment
//! scheme assumes a fixed backlog between capture and render, which only
//! holds while both are driven by one clock. Output is silence on
//! purpose - this reports, it isn't something to listen to.
//!
//!     cargo run -p looper-hal --features asio --example open_device
//!     cargo run -p looper-hal --features asio --example open_device -- wasapi
//!     cargo run -p looper-hal --features asio --example open_device -- wasapi "In Name" "Out Name"

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use looper_hal::{
    Backends, DeviceInfo, Direction, InputProcessor, OutputProcessor, StreamRequest,
    cpal_backend::default_backends,
};

const SECONDS: u64 = 10;

#[derive(Default)]
struct Meter {
    frames: AtomicUsize,
    calls: AtomicUsize,
    largest: AtomicUsize,
}

impl Meter {
    fn note(&self, frames: usize) {
        self.frames.fetch_add(frames, Ordering::Relaxed);
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.largest.fetch_max(frames, Ordering::Relaxed);
    }

    fn frames(&self) -> usize {
        self.frames.load(Ordering::Relaxed)
    }

    fn report(&self, label: &str) {
        println!(
            "  {label}: {} callbacks, {} frames, largest {} frames",
            self.calls.load(Ordering::Relaxed),
            self.frames(),
            self.largest.load(Ordering::Relaxed),
        );
    }
}

struct Capture(Arc<Meter>, usize);

impl InputProcessor for Capture {
    fn process(&mut self, input: &[f32]) {
        self.0.note(input.len() / self.1.max(1));
    }
}

struct Silence(Arc<Meter>, usize);

impl OutputProcessor for Silence {
    fn process(&mut self, output: &mut [f32]) {
        self.0.note(output.len() / self.1.max(1));
        output.fill(0.0);
    }
}

fn pick(devices: &[DeviceInfo], name: Option<&String>, wanted: Direction) -> Option<DeviceInfo> {
    devices
        .iter()
        .find(|device| {
            let suits = match wanted {
                Direction::Input => device.direction.can_capture(),
                _ => device.direction.can_play(),
            };
            suits && name.is_none_or(|name| &device.id.name == name)
        })
        .cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let backends = Backends::new(default_backends());

    let devices: Vec<DeviceInfo> = match args.first() {
        Some(id) => match backends.by_name(id) {
            Some(backend) => backend.devices().unwrap_or_default(),
            None => {
                println!("no backend called '{id}'");
                return;
            }
        },
        None => backends.devices(),
    };

    let (Some(input), Some(output)) = (
        pick(&devices, args.get(1), Direction::Input),
        pick(&devices, args.get(2), Direction::Output),
    ) else {
        println!("could not find both an input and an output device");
        return;
    };

    // Whatever both ends will actually take. A shared-mode capture
    // endpoint usually offers only its own mix rate.
    let backend = backends
        .get(input.id.backend)
        .expect("the backend that listed it");
    let in_rates = backend
        .caps(&input)
        .map(|c| c.sample_rates)
        .unwrap_or_default();
    let out_rates = backend
        .caps(&output)
        .map(|c| c.sample_rates)
        .unwrap_or_default();
    let Some(sample_rate) = in_rates.iter().copied().find(|r| out_rates.contains(r)) else {
        println!("no rate both ends agree on: in {in_rates:?}, out {out_rates:?}");
        return;
    };

    println!("Backend: {}", backend.label());
    println!("  in  '{}'", input.id.name);
    println!("  out '{}'", output.id.name);
    let duplex = input.direction == Direction::Duplex;
    println!(
        "  {} at {sample_rate} Hz",
        if duplex {
            "one duplex handle"
        } else {
            "two endpoints"
        }
    );

    let open = match backends.open(&StreamRequest {
        input: input.id.clone(),
        output: output.id.clone(),
        sample_rate,
    }) {
        Ok(open) => open,
        Err(err) => {
            println!("could not open: {err}");
            return;
        }
    };

    let format = open.format();
    println!(
        "Granted: {} Hz, {} in, {} out",
        format.sample_rate, format.input_channels, format.output_channels
    );

    let captured = Arc::new(Meter::default());
    let rendered = Arc::new(Meter::default());
    let stream = match open.start(
        Box::new(Capture(
            Arc::clone(&captured),
            format.input_channels as usize,
        )),
        Box::new(Silence(
            Arc::clone(&rendered),
            format.output_channels as usize,
        )),
        Arc::new(|err| eprintln!("  stream error: {err}")),
    ) {
        Ok(stream) => stream,
        Err(err) => {
            println!("could not start: {err}");
            return;
        }
    };

    // The difference between frames captured and frames rendered is the
    // backlog the looper's rings carry. Bounded means one clock; growing
    // steadily means two, and a loop that slides out of time.
    println!("\n  t     captured    rendered       gap");
    let started = std::time::Instant::now();
    let mut first: Option<(f64, i64)> = None;
    let mut last = (0.0, 0i64);
    // Sampled finely as well as once a second: a gap that is the same at
    // every second boundary can still swing by a whole callback in
    // between, and that swing is the alignment error a take inherits.
    let (mut low, mut high) = (i64::MAX, i64::MIN);
    for tick in 0..SECONDS * 1000 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        let elapsed = started.elapsed().as_secs_f64();
        let gap = captured.frames() as i64 - rendered.frames() as i64;

        // Only once both streams are properly running, so the ragged
        // first moments aren't read as wander.
        if elapsed > 1.0 {
            low = low.min(gap);
            high = high.max(gap);
            first.get_or_insert((elapsed, gap));
            last = (elapsed, gap);
        }
        if tick % 1000 == 999 {
            println!(
                "{elapsed:5.1}s {:11} {:11} {gap:9}",
                captured.frames(),
                rendered.frames()
            );
        }
    }

    if let Some((t0, g0)) = first {
        let seconds = last.0 - t0;
        if seconds > 0.0 {
            let per_second = (last.1 - g0) as f64 / seconds;
            println!(
                "\n  drift  {per_second:+.1} frames/s = {:+.0} ppm",
                per_second / format.sample_rate as f64 * 1e6
            );
        }
        println!(
            "  wander {} frames ({} to {}) = {:.1} ms",
            high - low,
            low,
            high,
            (high - low) as f64 / format.sample_rate as f64 * 1000.0
        );
    }
    captured.report("capture");
    rendered.report("render");

    drop(stream);
    println!("Stopped cleanly.");
}
