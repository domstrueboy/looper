use super::*;

/// Records what it was handed, so a test can see the driver reached it.
struct Capture(std::sync::Arc<std::sync::Mutex<Vec<f32>>>);

impl InputProcessor for Capture {
    fn process(&mut self, input: &[f32]) {
        self.0
            .lock()
            .expect("not poisoned")
            .extend_from_slice(input);
    }
}

/// Fills the buffer with a fixed value, so a test can see the render
/// callback ran and that what it wrote came back out.
struct Tone(f32);

impl OutputProcessor for Tone {
    fn process(&mut self, output: &mut [f32]) {
        output.fill(self.0);
    }
}

type Heard = std::sync::Arc<std::sync::Mutex<Vec<f32>>>;

fn start(
    driver: &MockDriver,
    backend: &MockBackend,
    sample_rate: u32,
) -> (Box<dyn AudioStream>, Heard) {
    let heard: Heard = Heard::default();
    let open = backend.open(&request(sample_rate)).expect("opens");
    let (sink, _log) = driver.error_sink();
    let stream = open
        .start(
            Box::new(Capture(std::sync::Arc::clone(&heard))),
            Box::new(Tone(0.5)),
            sink,
        )
        .expect("starts");
    (stream, heard)
}

#[test]
fn it_reports_one_duplex_device() {
    let (backend, _driver) = mock();
    let devices = backend.devices().expect("enumerates");

    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].direction, Direction::Duplex);
    assert!(devices[0].direction.can_capture() && devices[0].direction.can_play());
    assert_eq!(devices[0].id.backend, BackendId::MOCK);
}

#[test]
fn a_device_it_has_never_heard_of_is_not_found() {
    let (backend, _driver) = mock();
    let missing = DeviceInfo {
        id: DeviceId::new(BackendId::MOCK, "Not A Device"),
        direction: Direction::Duplex,
    };

    assert!(matches!(
        backend.caps(&missing),
        Err(HalError::DeviceNotFound { .. })
    ));
}

#[test]
fn caps_report_the_channels_it_was_built_with() {
    let (backend, _driver) = mock_with(2, 4);
    let device = backend.devices().expect("enumerates").remove(0);
    let caps = backend.caps(&device).expect("caps");

    assert_eq!(caps.input_channels, 2);
    assert_eq!(caps.output_channels, 4);
    assert!(caps.sample_rates.contains(&44_100));
}

#[test]
fn a_rate_the_device_does_not_offer_is_refused() {
    let (backend, _driver) = mock();

    assert!(matches!(
        backend.open(&request(12_345)),
        Err(HalError::UnsupportedConfig(_))
    ));
}

#[test]
fn opening_does_not_start_anything() {
    // The whole reason opening is its own step: the caller has to be
    // able to read the format and size its buffers before a single
    // callback runs.
    let (backend, driver) = mock();
    let open = backend.open(&request(44_100)).expect("opens");

    assert_eq!(open.format().sample_rate, 44_100);
    assert!(!driver.is_running(), "opening is not starting");
    assert_eq!(driver.starts(), 0);
}

#[test]
fn the_granted_format_can_differ_from_the_one_asked_for() {
    // What a shared-mode endpoint does: it hands back its own mix rate
    // whatever the request said. A caller that assumed its request stood
    // would size every buffer against the wrong rate.
    let (backend, _driver) = mock_with(1, 1);
    let backend = backend.always_grants(48_000);

    let open = backend.open(&request(96_000)).expect("opens");
    assert_eq!(open.format().sample_rate, 48_000);
}

#[test]
fn starting_runs_both_callbacks_in_driver_order() {
    let (backend, driver) = mock();
    let (_stream, heard) = start(&driver, &backend, 44_100);

    assert!(driver.is_running());
    let out = driver.cycle(&[0.25, 0.5, 0.75, 1.0]);

    assert_eq!(
        heard.lock().expect("not poisoned").as_slice(),
        &[0.25, 0.5, 0.75, 1.0],
        "the capture callback saw what the driver handed over"
    );
    assert_eq!(
        out,
        vec![0.5; 4],
        "and the render callback's buffer came back"
    );
}

#[test]
fn a_cycle_interleaves_by_the_granted_channel_counts() {
    // One input channel, two output: a frame in is one sample, a frame
    // out is two, so the buffer sizes are not the same number.
    let (backend, driver) = mock_with(1, 2);
    let (_stream, _heard) = start(&driver, &backend, 44_100);

    let out = driver.cycle(&[0.1, 0.2, 0.3]);
    assert_eq!(out.len(), 6, "three frames of two channels");
}

#[test]
fn dropping_the_handle_stops_the_stream() {
    // Every backend stops on drop, so the mock has to as well - a test
    // would otherwise pass while a real device kept playing.
    let (backend, driver) = mock();
    let (stream, _heard) = start(&driver, &backend, 44_100);
    assert!(driver.is_running());

    drop(stream);

    assert!(!driver.is_running(), "the handle is what holds it open");
    assert_eq!(driver.starts(), 1, "it did start, and then stopped");
}

#[test]
fn a_stream_can_be_opened_again_after_being_dropped() {
    // Which is what changing device in Settings does.
    let (backend, driver) = mock();
    let (stream, _heard) = start(&driver, &backend, 44_100);
    drop(stream);

    let (_stream, heard) = start(&driver, &backend, 48_000);
    driver.cycle(&[0.5, 0.5]);

    assert_eq!(driver.starts(), 2);
    assert_eq!(heard.lock().expect("not poisoned").len(), 2);
}

#[test]
fn errors_reach_the_sink() {
    let (sink, log) = mock().1.error_sink();
    sink(HalError::Runtime("the device went away".to_string()));

    let reported = log.taken();
    assert_eq!(reported.len(), 1);
    assert!(log.taken().is_empty(), "taking them clears them");
}
