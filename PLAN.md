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

**v1 is complete**, and the portability work after it. Working today: a
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
  post-v1 item that had stood since the review.

Next is v2, which needs ordering before it can start.

## Ground rules

- **The real-time boundary is the one thing that must not drift:** no
  locks and no allocations on the audio thread. UI <-> audio talks only
  through `SharedControl`'s atomics or a ring buffer, never a mutex.
- **Stays small:** "run and play", not a DAW. No upfront abstraction and
  no UI framework layer - small functions and structs, factored further
  only when duplication actually shows up.
- **Minimal dependencies,** with v1 step 6 as the one deliberate
  exception.
- **`Renderer::Glow` is mandatory** - the default wgpu renderer crashes
  (STATUS_ACCESS_VIOLATION) on this machine's Intel UHD graphics.
- **`f32` above the device, full scale at +/-1.0.** Whatever format the
  hardware speaks stops at the backend; `sample.rs` owns the conversion
  at the two edges that still deal in integers (the device, and the
  saved WAV). Clipping happens once, on the way out.
- **One small commit per step,** so history stays reviewable step by
  step rather than as one large diff.

## Portability split - done

Between v1 and v2, and not a feature: the aim was to make the app
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
- **The by-ear pass is owed twice** - see the [post-v1 review](v1-archive.md#post-v1-review---done).

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

## v2: extended build

Ordered roadmap. Each step is small, testable, and doesn't touch the
audio device layer until the final integration step.

### 0. BPM config field - prerequisite

Add `metronome_bpm: u32` to `AppConfig` (range 40-300, default 120).
This is needed before any timing feature.

### 1. Per-layer mute - finishes track abstraction

A `mute: bool` on `Layer` and a per-layer UI list. The layer stack
already supports independent layer management; mute is the one piece
of "record/mute/delete-able" not built. Finishes v1 step 2's debt and
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
mute and level, once the track abstraction (v1 step 2) is in place. Nothing
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
