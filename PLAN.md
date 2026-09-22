# Guitar Looper - Plan

A minimal, single-track looper pedal replacement for practicing guitar
through an ASIO audio interface. Standalone Windows app - no DAW, no
plugin host.

This file is forward-looking: what's left to build, in what order, and
the decisions that shouldn't drift. What already exists is documented
where it belongs instead of being restated here - `README.md` for
architecture, module layout and build steps, `docs/user-guide.md` for
how the app is actually used.

## Status

**v2 is complete**, and the portability work after it. Working today: a
driver / device / sample-rate / input-channel picker with settings
persisted as TOML in the per-user config directory, always-on live
monitoring, the record -> loop -> stop -> resume cycle on spacebar or
button, overdub layers on a control of their own with remove-last, a
configurable pre-roll before recording, long-press clear, the loop
surviving restarts as WAV layers, an app icon, no console window in
release builds, technical + user docs, and a Windows CI build/release
pipeline.

Since then the app has been split into four crates - `looper-hal`
(devices), `looper-core` (the pedal), `looper-ui-egui` (the screens) and
`looper-app` (the shell) - the core moved to `f32`, and a second backend
(WASAPI) landed beside ASIO. See README's architecture section. What that
bought, concretely:

- **A port is a backend, not a rewrite.** `looper-hal` is one trait set
  and has no dependencies unless its `cpal` feature is asked for.
- **No format assumption survives.** i16/i24/i32/f32 all convert at the
  backend; the i32 the app was built around is gone from everything
  above it.
- **The suite runs anywhere.** `cargo test -p looper-core` needs no
  audio SDK, on any platform.
- **The looper screen is testable**, on a mock backend, which closed a
  post-v2 item that had stood since the review.

Next is v3, which needs ordering before it can start.

## Ground rules

- **The real-time boundary is the one thing that must not drift:** no
  locks and no allocations on the audio thread. UI <-> audio talks only
  through `SharedControl`'s atomics or a ring buffer, never a mutex.
- **Stays small:** "run and play", not a DAW. No upfront abstraction and
  no UI framework layer - small functions and structs, factored further
  only when duplication actually shows up.
- **Minimal dependencies,** with v2 step 6 as the one deliberate
  exception.
- **`Renderer::Glow` is mandatory** - the default wgpu renderer crashes
  (STATUS_ACCESS_VIOLATION) on this machine's Intel UHD graphics.
- **`f32` above the device, full scale at +/-1.0.** Whatever format the
  hardware speaks stops at the backend; `sample.rs` owns the conversion
  at the two edges that still deal in integers (the device, and the
  saved WAV). Clipping happens once, on the way out.
- **One small commit per step,** so history stays reviewable step by
  step rather than as one large diff.

## v2 roadmap (ordered)

### 1. UI/view-model split - done

`main.rs` is eframe glue only now. `app.rs` (which screen is up, acting
on actions), `looper.rs` and `settings.rs` hold the state as plain data
with no `egui::` types, and each pairs with a renderer of the same name
under `ui/` following `render(ui, model) -> Option<Action>`. See
README's module layout.

Step numbers below stay fixed as items land - they're cross-referenced
throughout this file.

### 2. Overdub as removable layers - done

`LoopStack` replaced the flat `LoopBuffer`: the first recording fixes the
loop length and each overdub adds an aligned layer on top, up to
`MAX_LAYERS` (4), played back as their sum. The newest layer can be
dropped again. Overdub is on its own control (`O` / a button) rather than
in the press cycle, so nothing about the existing cycle changed - see
README's state machine and layers sections.

Per-layer **mute** is the one part of "record/mute/delete-able" not built:
it needs the per-layer UI list, which the count-plus-remove-last UI
deliberately skipped. Worth doing together with the waveform work, or
whenever a layer list appears.

### 3. Hide the console window outside dev builds - done

`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` in
`main.rs`: the release binary is a GUI-subsystem exe with no console,
`cargo run` still builds a console one. Verified by reading the
subsystem field out of both binaries' PE headers (2 vs 3).

`asio_host()` went with it: it used to `expect()`, and a panic before
the window opens is invisible without a console, so it now reports and
the settings screen shows it.

### 4. Pre-roll delay before recording - done

An `Arming` state advanced by the UI frame loop, with a 0-5s "Record
delay" slider in Settings, defaulting to 5s and reading "off" at zero.
The wait shows as the seconds counting down where the recording time
normally appears, over a light gray bar filling beneath it - laid out
like the looping and recording readouts rather than as its own thing. A
lone number on the button read as ambiguous, and a bar's own `text` sits
on the left in egui, where a shrinking number behind a growing fill
reads as a contradiction. First recording only; a press part-way through
calls it off, and the button shows a cancel cross to say so. The audio
callback treats `Arming` exactly like `Idle`, so no timing code went
near it - see README's pre-roll section.

`Arming` is published rather than hidden from the audio thread on
purpose: v3's metronome count-in is this same window with clicks in it,
and will need the callback to know.

`Action::Start` now carries an `AppConfig` rather than a growing list of
fields, and `AppConfig::load` defaults a missing `preroll_ms` instead of
rejecting the file, so configs written before this still load.

### 5. Preserve the loop across restarts - done

One WAV per layer beside the config, and a copy of the loop kept on the
UI thread to have something to write - see README's saved-loops section
for how and why.

Two decisions changed while building it:

- **Saved when the loop changes, not on exit.** An exit hook would have
  needed the whole loop pulled out of the audio thread in one moment,
  which the streaming handoff can't do; saving on change is also
  proof against a crash.
- **Per-layer, not mixed down.** Once the UI thread holds the takes,
  writing them separately costs nothing extra and keeps "remove last"
  meaningful after a restart.

Take layout is implemented twice - incrementally in the callback, in
one go in the mirror - with a test asserting they agree. Worth folding
into one implementation if a third caller ever appears.

### 6. Settings in a real config file - done

TOML via `serde` + `toml`, in the per-user config directory found with
`directories` - see README's settings section for the paths and the
compatibility rules. `device_name` and `sample_rate` are required,
everything else defaults, so adding a setting can't invalidate an
existing file. The reader for the old `looper-pedal.cfg` next to the
executable was dropped in the post-v2 review, no such file being likely
to be left anywhere.

Deps added as planned, the one deliberate exception to minimal-deps:
`serde`, `toml`, `directories`.

Step 5 can now put the recorded loop next to the config rather than next
to the exe.

Settings that used to be constants moved into the config and onto the
screen at the same time: latency, max loop length, max layers and the
hold-to-clear time. Each one's default and range live together in
`config.rs`, the sliders are built from those ranges, and a hand-edited
file is clamped to them - see README's settings table. The screen scrolls
now, with Start pinned below it.

## Portability split - done

Between v2 and v3, and not a feature: the aim was to make the app
reliable across machines and interfaces, and to leave other platforms as
work rather than as a rewrite. What it produced, in the order it landed:
config paths became testable; the core moved to `f32`; the tree became a
workspace; `looper-hal` appeared with its traits and a mock backend; the
cpal-free half of `engine.rs` and then the screen states moved to
`looper-core`; a cpal backend arrived and the app swapped onto it; the
screens became `looper-ui-egui`; the looper screen got tests; WASAPI was
measured and then offered; failures and underruns reached the screen.

Three decisions worth not re-litigating:

- **`f32` in the core, converted at the backend.** The alternative was
  keeping `i32` and converting for every non-ASIO device. `f32` is what
  the eventual NAM and drum work wants anyway, and it made clipping a
  single event at the edge instead of something the loop bus did to
  itself.
- **Backends are runtime trait objects, not `cfg`-picked modules.** One
  binary offers ASIO *and* WASAPI, which is what a machine with no ASIO
  driver needs; it is also what Linux's several hosts will need. The
  mock backend that falls out of it is what made the looper screen
  testable at all.
- **Two-phase open.** The caller cannot size a ring, a delay line or a
  layer stack until it knows the rate that was *granted*, and shared
  mode grants its own.

### What the hardware said

Neither guessable nor documented; both found by running
`looper-hal`'s examples against an iD4. Measured at 44.1 kHz:

| | ASIO | WASAPI (same interface) |
|---|---|---|
| shape | one duplex handle | two endpoints, same name |
| callback period | 64 frames (1.5 ms) | 441 frames (10 ms) |
| drift | 0 ppm | 0 ppm |
| wander | 64 frames (1.5 ms) | 882 frames (20 ms) |

Drift was the thing to fear and isn't: WASAPI's shared mode resamples
both ends onto the Windows audio engine's clock, so even two *different*
interfaces stay in step. Wander is the real cost, and `MonitorDelay`
holds back by a fixed amount, so nothing takes it out. WASAPI is a
fallback, labelled as one on the settings screen - not a peer.

Two bugs only hardware could show, both now guarded by the type system
rather than by care: a device is identified by name *and* direction
(WASAPI gives an interface's two halves the same name, and a render
endpoint asked to capture records the speakers instead of failing), and
two equal device ids do not mean one duplex device.

### Still open from it

- **WASAPI's default input is whatever Windows lists first**, which on
  this machine is the iD4's *loopback* endpoint - it records what is
  playing, not the guitar. The only way to tell a loopback from a real
  input through `cpal` is sniffing the name, and "Loop-back",
  "Loopback", "Stereo Mix" and "What U Hear" are all different vendors'
  spellings, some localised. Left alone deliberately: the picker is
  visible and the choice is saved once. Worth revisiting if anyone
  actually trips on it.
- **The by-ear pass is owed twice** - see the post-v2 review below.

## Open decisions

Worth settling before the work they block starts.

- **Loop / bar-grid sync** - blocks both the metronome and the drum
  machine. A loop recorded today is whatever length happened to be
  played, so it won't line up with a bar grid and drums against it will
  clash. Either quantize the loop length to a whole bar when recording
  stops (needs the clock running *while* recording, and pairs naturally
  with step 4's arming window as a count-in), or leave the drums
  free-running and keep time yourself. Recommend quantizing - decided
  before either feature is built, since both depend on it.
- ~~**Release-build diagnostics**~~ - **done.** A failed stream shows on
  the looper screen with a button back to Settings, and underruns show
  as a running count naming the setting that fixes them. The console
  still carries more detail in a debug build.
- **GPL ASIO SDK before sharing binaries** - CI builds against the
  GPLv3 fallback SDK, and depending on how `asio-sys` links its
  compiled shim that can carry GPL obligations onto a distributed exe.
  Irrelevant for building and running it yourself; worth a look before
  treating a GitHub Release as "anyone can download this".

## Later (unordered)

- Trim loop start/end, and the waveform rendering it needs anyway
- Undo, distinct from full clear
- Export / save loops to a chosen file - reuses step 5's WAV writer
- NAM model loading, in the insert-effect stage the signal path leaves
  room for between input and the mix
- Tuner - pitch detection on a decimated copy of the input, analysed on
  a background thread or per UI frame. Never inside the callback: FFT /
  autocorrelation cost is unbounded relative to the audio budget.
- **macOS port** - both things this used to need are done: host
  selection is a backend registry, and the format assumption is gone.
  What is left is `CpalBackend::new(BackendId::COREAUDIO, HostId::CoreAudio,
  "Core Audio")`, a `cfg` to register it, and running the two
  `looper-hal` examples against real hardware to find what only hardware
  tells you - the WASAPI work turned up two such things in an afternoon.
  Plus whatever eframe wants on macOS, which is its own question.
- **Linux port** - the same, times the number of hosts worth offering.
  `cpal` has ALSA and JACK; the registry already shows several backends
  side by side and skips the ones whose driver isn't installed, which is
  the shape that fragmentation needs. Low-latency behaviour is the real
  unknown, and `open_device` is how to find out rather than guess.
- **Mobile** - not "one more platform". `cpal` and `eframe` are both
  rougher there, and mobile OSes make the low, predictable latency this
  app is built around much harder. Needs a throwaway spike answering
  whether a minimal cpal + eframe passthrough can even hit usable
  latency on a real phone, before it becomes a backlog item.
- **Web** - egui already compiles to WASM, but `cpal`'s ASIO backend has
  no browser equivalent, so a real web version means a parallel
  AudioWorklet engine - a separate project, not a port. Cheaper middle
  ground if ever wanted: keep this app as the audio engine and serve a
  browser page as a remote control/monitor over a local socket.

## Post-v2 review - done

A deliberate read of the whole codebase, cold, before any release -
everything in it had grown by accretion across seven steps and had
never been read as a whole.

What it produced, in the order it landed: the saved loop is now held to
the settings that load it and a lost-capture flag no longer disables
saving for the session; the frame is advanced before it's drawn rather
than part-way through; the state machine left `audio/`; the settings
screen holds an `AppConfig` instead of a copy of one; the pre-roll
became testable; the audio callbacks left their closures, which is what
made the recording alignment measurable at all; recording was landing a
whole monitoring delay ahead of the beat, and the rings were too small
to hold one driver buffer. See the history for the reasoning on each.

Still open:

- **The alignment fix wants confirming by ear.** `MonitorDelay` holds
  the recorded signal back to where the monitored one is, and a test
  measures the remaining offset as zero - but that test can't speak for
  the driver's own input/output offset. Worth stacking four layers
  against a click before calling it settled. The ring-sizing fix in the
  same area wants the same pass: no underruns reported at whatever
  latency you actually run. **Still open, and now owed twice**: the f32
  move and the swap onto the hardware layer each changed everything the
  signal passes through, and each deserves its own pass rather than one
  covering both.
- ~~**`looper.rs` still can't be tested**~~ - **done.** It holds one
  `Box<dyn AudioStream>` now and opens on a mock backend, so the rule
  that a cancelling press beats a pre-roll expiry on the same frame is a
  test. Reversing the two lines in `tick` makes it fail with the state on
  `Looping` rather than `Recording` - the expiry opens a take and the
  press immediately closes it, leaving an empty loop playing. The two
  readouts answer from the frame's own clock as well.
- **Take layout still exists twice** - incrementally in
  `read_mixed_with_overdub`, in one go in `LoopMirror::layer`, with a
  test asserting they agree. Worth folding into one if a third caller
  ever appears.

### Load-bearing, despite appearances

The cost of reading cold: these all look like dead weight and are not.
Each one is either a bug that has already happened once or a real-time
constraint. Check the reason before touching them - README and the
tests cover every one.

- **`MonitorDelay` in the input path** - looks like a buffer for its
  own sake. The dry path is delayed by `latency_frames` to absorb
  callback jitter, so a recorded signal that isn't delayed with it lands
  that far ahead of the beat it was played against, and every layer
  inherits the error again. Measured by a test; it read 8, 20 and 50
  samples at the matching latency settings before the fix.
- **Ring capacity of `latency_frames + MAX_BLOCK_FRAMES`** - looks
  over-generous. It was `latency_frames * 2` with half of it prefilled,
  which leaves room for exactly the delay: a driver buffer larger than
  that spilled on every single callback.
- **`loop_mirror::load` taking the whole `AppConfig`** - looks like
  needless coupling for a file read. It's the one place that decides
  whether a saved loop still applies, and both the layer stack and the
  UI thread's copy are seeded from its answer. Splitting the rule lets
  them disagree about what exists.
- **`button_held` in `looper.rs`** - a latch, rather than asking egui
  whether the pointer is on the button. egui reads a cursor drifting off
  the button as a release, which broke long-press-clear from the button
  once already.
- **Mixing before writing in `read_mixed_with_overdub`** - the take is
  mixed into the output *before* the incoming sample is recorded over
  it. Reverse the order and the player hears their own take echoed back
  on top of their live signal a buffer later.
- **Nothing clamps until the device** (`mix_at`, `mix_add`, then
  `sample::to_pcm32`) - clamping per layer made a hot four-layer stack
  permanently clipped, with the volume slider unable to rescue it, and
  took the live passthrough down with it since the dry signal is added
  after the loop bus. The 64-bit sum this replaced existed only because
  four `i32`s overflow; `f32` has the headroom.
- **The recorded window (`start` / `written`) in `Layer`** - not an
  optimisation for its own sake. It's what lets a layer be reused
  without memsetting megabytes inside the audio callback, and the
  alternative is unbounded work in the real-time path.
- **`recorded_len` separate from `loop_len` in `LoopStack`** - one is
  how much has been recorded, the other is where playback wraps.
  Conflating them made the elapsed time read zero for a whole take.
- **`Arming` published to the audio thread** rather than hidden behind
  `Idle`, which would work today. v3's count-in needs the callback to
  know it's counting down.
- **Chunking in both callbacks, as well as in the backend** - bounds one
  callback's work whatever the driver hands us. Overrunning the fixed
  scratch buffers would panic inside the real-time path. It looks
  redundant now that `looper-hal` chunks too, and is not: the tests
  drive these paths directly, with no backend in front of them, so
  removing it would leave the bound untested.
- **`caps` taking a whole `DeviceInfo`** rather than a device's name -
  WASAPI presents an interface's capture and render halves under the
  *same name*, so a lookup by name alone answers about whichever the
  driver listed first. A render endpoint handed to a capture stream
  doesn't fail either; `cpal` makes it a loopback and records the
  speakers.
- **Two-phase `open` then `start`** - looks like ceremony. Ring sizes,
  the monitoring delay and the layer stack are all measured in samples
  against the rate that was actually *granted*, and a shared-mode
  endpoint hands back its own mix rate whatever was asked for.

## v3: extended build

Ordered roadmap. Each step is small, testable, and doesn't touch the
audio device layer until the final integration step.

### 0. BPM config field - prerequisite

Add `metronome_bpm: u32` to `AppConfig` (range 40-300, default 120).
This is needed before any timing feature.

### 1. Per-layer mute - finishes track abstraction

A `mute: bool` on `Layer` and a per-layer UI list. The layer stack
already supports independent layer management; mute is the one piece
of "record/mute/delete-able" not built. Finishes step 2's debt and
unblocks the mic channel (per-source monitoring requires per-source
mute).

### 2. Scheduler core - pure, testable

(sample position, BPM, pattern) -> events -> voices. One scheduler
that the metronome is a preset of, and the drum machine is a richer
version of. No audio dependency — driven by sample position, tested
against a mock clock.

### 3. Metronome click synthesis

A pre-computed short sine/triangle burst, stored via `include_bytes!`.
No synthesis in the callback, no allocation. Simple: 4/4 time, all
beats equal, no accents.

### 4. Metronome audio integration

Add `Metronome` struct to `OutputPath`, process clicks into the output
buffer. Driven by `play_pos` from `SharedControl` — metronome clicks
on beat boundaries. Toggle key (`M`) + UI indicator in looper screen.
Clicks during `Arming` state (count-in).

### 5. Loop/bar-grid sync - decision + implementation

Quantize loop length to whole bars when recording stops. Needs the
clock running *while* recording, pairs naturally with the arming window
as a count-in. If loops become bar-quantized, the saved loop will want
its BPM and bar count stored alongside it.

This gates the drum machine: drums against a non-quantized loop will
clash.

### 6. Drum machine

One-shot samples plus an editable step pattern, not pre-recorded loops
(whose tempo is baked in, so changing BPM would mean resampling). A
small fixed-size polyphonic voice pool, pre-allocated to stay RT-safe.
Kits load on the UI thread and are handed to the audio thread the way
step 5 hands over a restored loop, never loaded in the callback; embed
one small CC0 kit via `include_bytes!` to keep the single-exe property,
with an optional folder next to the config for more.

### 7. Mic channel

The second input as a track of its own, with independent record-enable,
mute and level, once the track abstraction (step 2) is in place. Nothing
stands in the way physically: both iD4 inputs are on the same stream and
the input callback already receives every channel interleaved, it just
discards all but the chosen one — so no second device to open and no
cross-device drift. Two things the single-channel path never had to
face: **per-source monitoring** (guitar monitoring is always wanted,
live mic monitoring often isn't — feedback, and hearing yourself dry is
unpleasant), and **levels that differ enormously** between an instrument
input and a mic preamp, so one shared gain isn't enough — eventually an
input meter, so it can be set by eye.

### 8. Mini stays mini

The workspace exists now (`looper-hal`, `looper-core`, `looper-ui-egui`,
`looper-app`), so what is left of this is a second thin binary beside
`looper-app` assembling a different feature set, so the simple build
never carries multitrack code paths it doesn't use.

All of the above stays "run and play", not a DAW. The metronome is the
first step — small, self-contained, and building on `Arming`'s
existing publish-to-audio-thread pattern.

## CI / releases

`.github/workflows/release.yml`: every push to `main` bumps the patch
version, commits and tags it, builds on `windows-latest` against the
GPLv3 ASIO SDK, and attaches a zip to a GitHub Release - one job, one
run. It deliberately doesn't rely on the tag push triggering a second
run, since pushes made with the default `GITHUB_TOKEN` don't trigger
workflows.

Confirmed working: the bump/commit/tag/push half - the bot produced
both `v0.1.1` and `v0.1.2`, so branch protection isn't in the way.
Still unverified: the bump step runs *before* the build, so those tags
don't prove the headless ASIO SDK fetch, the tests, the release build
or the zip upload succeeded. Worth checking whether those two releases
actually have the zip attached.

Known rough edges: every commit to `main` becomes a tagged release
(gate it behind a commit-message marker if that gets noisy), and only
patch bumps are automatic - minor/major stay manual, since there's no
commit convention to infer them from.

## Docs

As each item lands, update `README.md` (architecture, module layout,
build) and `docs/user-guide.md` (usage) - docs track what's actually
built rather than staying frozen at the MVP.
