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

**v2 is under way** - steps 0-7 done: the mini/extended switch, layer
mute, redo, the metronome with live tempo, loops snapped to bars, a
tempo for every loop, saved with it, and the pedal feel - two pedals
(Space never stops, S always does), three record modes, hold to undo,
and record start and stop modes. Next is step 8.

Since v1 the app has been split into four crates - `looper-hal`
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
- Export / save loops to a chosen file - reuses v1 step 5's WAV writer.
  Could grow into numbered memory slots, several saved loops to pick
  from (the NUX has 99), with save and delete.
- **A/B song parts** (extended) - two loops, switched between for verse
  and chorus, as on the NUX Dual Loop Stereo. Sharing one tempo, and a
  switch *waits for the end of the current loop* rather than cutting it
  off. Each part would want its own layer stack, so memory doubles, and
  its own drum groove; worth doing after the mic channel has shown what
  a second track costs. The NUX's other mode, parallel tracks muted in
  and out for quiet sections, is mostly what layer mute already does.
- **Footswitch control** - the NUX ships a two-button pedal, because
  hands belong on the guitar. For a desktop app that means MIDI foot
  controllers (a dependency, so a deliberate choice). Pedals that send
  key presses work today, mapped to Space, O, R and the rest.
- **Split output** - loop and guitar on the left channel, drums or click
  on the right, so the click can go to headphones only, or each to its
  own amp. The output is mono duplicated to every channel today.
- **Effects** - an insert-effect stage between input and the mix, which
  the signal path leaves room for: NAM model loading, and the NUX's
  looper-shaped set - an **octave down** to play a bass part on the
  guitar, a **boost** for soloing over the loop, **reverb** on the drums,
  and a cabinet simulation for going straight to a mixer.
- **Import a backing track** - a WAV as a loop to play over, beside
  export and memory slots.
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

The app gets two modes on one screen: **mini** is the v1 looper plus
the metronome, **extended** adds the layer list, drums and mic. One
binary, switched at runtime - a separate mini build was dropped, since
a hidden feature costs nothing once its audio path is a skipped flag
check.

| Mode | Has |
|---|---|
| mini | record/loop/overdub/redo, metronome, bar-grid sync, pedal feel, tempo and levels |
| extended | mini + layer list with mute, drum machine, mic channel |

Steps 7 and 8 come from the NUX Dual Loop Stereo's manual and a review
of it, whose first complaint about loopers in general - "why does the
loop start when you *release* the pedal?" - turned out to be true of
this one.

### 0. Mini/extended switch - done

A `mode` field in `AppConfig` (default mini) and a button on the looper
screen to flip it, with an empty extended area for later steps to fill.
The button is enabled in `Idle` and `Stopped`, never mid-play. Going to
mini unmutes every layer, since mini can't show a mute.

### 1. Per-layer mute - extended - done

A `mute: bool` on `Layer` and a per-layer UI list. The layer stack
already supports independent layer management; mute is the one piece
of "record/mute/delete-able" not built. Finishes v1 step 2's debt and
unblocks the mic channel (per-source monitoring requires per-source
mute).

### 2. Redo - mini - done

Brings back the layer "remove last" dropped. Close to free: removing
only lowers the layer count, so the audio stays in the buffer until the
next take overwrites it. Available from a remove until a new layer is
recorded or the loop is cleared, then gone - which needs the mirror to
keep the removed layer's copy for the same window, so a redone layer is
saved again. Not across restarts.

### 3. Metronome click synthesis - done

A short sine/triangle burst, generated once when the stream opens, at
the granted sample rate. No synthesis in the callback, no allocation,
and no asset file to embed. Independent of everything else in v2; it
only has to exist before step 5 plays it.

### 4. Scheduler core - pure, testable - done

(sample position, BPM, pattern) -> events -> voices. One scheduler
that the metronome is a preset of, and the drum machine is a richer
version of. No audio dependency - driven by sample position, tested
against a mock clock. Simple first: 4/4 time, all beats equal, no
accents.

### 5. Metronome audio integration - mini - done

Add `metronome_bpm: u32` to `AppConfig` (range 40-300, default 120),
landing here rather than earlier since this is the first step that
reads it. Add `Metronome` struct to `OutputPath`, process clicks into
the output buffer. Driven by `play_pos` from `SharedControl` - clicks on
beat boundaries. Toggle key (`M`) + UI indicator in looper screen.
Clicks during `Arming` state (count-in).

### 6. Loop/bar-grid sync - mini - done

Two ways to get a loop that fits a bar grid, picked by whether the
metronome is on:

- **Metronome on: round the loop.** Quantize loop length to whole bars
  when recording stops. Needs the clock running *while* recording,
  pairs naturally with the arming window as a count-in.
- **Metronome off: derive the tempo.** The loop stays whatever length
  was played, and the BPM is worked out from it - the number of 4/4
  bars that puts the tempo nearest a playable range (roughly 70-140),
  with halve/double to correct a wrong guess. Taken from the NUX Dual
  Loop Stereo, whose drummer follows the loop rather than the reverse.

The rejected alternative was leaving the drums free-running and keeping
time yourself: drums against such a loop drift audibly. Either way the
saved loop will want its BPM and bar count stored alongside it.

This gates the drum machine: drums against a loop with no tempo will
clash.

Split four ways:

- **6a. Snap to bars - done.** With the metronome on, stopping a take
  lands it on the *nearest* bar line, at least one bar: late, the extra
  is cut; early, recording carries on to the line. The audio thread
  rounds; the UI thread only holds off stopping while the line is ahead.
- **6b. Count-in - done.** With the metronome on, the pre-roll is one
  bar of clicks instead of the "Record delay" time, and the take opens
  on the audio thread at the bar line rather than on a UI frame.
- **6c. Tempo from the loop - done.** With the metronome off, the BPM is
  worked out from the loop's length, with ½ / 2× to correct it. While a
  loop exists it sets the tempo either way.
- **6d. Save the tempo - done.** The beat count beside the saved loop
  (`tempo.toml`), so a restored loop keeps its tempo; the BPM follows
  from it and the length.

### 7. Pedal feel - mini

How the main control behaves, as settings rather than one fixed way.

- **7a. Act on press-down - done.** A press fired on *release*, so
  every record start and stop landed as late as the key or foot stayed
  down - 100-200 ms. It fires on the way down now; a long hold still
  clears, wiping whatever the press had just started. It also showed
  the UI thread's copy could save a take at its played length rather
  than its rounded one, now fixed.
- **7b. Dub mode - done.** "After recording: Play / Overdub" in
  Settings. With Overdub the main control is the NUX's footswitch:
  record, dub, play, dub, play - every dub a layer - and a double press
  stops, taking back a dub its first half had just opened. A **Stop**
  control (S, and a button) stops in one press in either mode, keeping
  what was in progress - the NUX's second footswitch.
- **7f. Two pedals - done.** 7b grew two ways to stop and two to dub;
  this replaces them with the two-pedal layout most loopers share.
  Space never stops; **S** always does, keeping what's in progress. A
  **double press** of Space always does exactly what S does - no
  setting - after taking back what its first half started: a dub it
  opened is abandoned, a take or a count-in it began on an empty looper
  leaves it empty. The O key and the Overdub button go. R becomes Redo (Y
  goes); the row is Stop (S) | Undo | Redo (R). Undo (= today's Remove)
  never takes the first take - clearing is a hold. "After recording"
  becomes **Record mode**, three choices:

  | Mode | After the take | Space while dubbing | Space while playing | S / double Space |
  |---|---|---|---|---|
  | REC→PLAY | plays | keeps the layer, plays | dubs a new layer | stops |
  | REC→DUB | dubs | keeps the layer, dubs the next | dubs a new layer | dubbing → plays; playing → stops |
  | REC→DUB, a layer per pass | dubs | keeps the layer, plays | dubs a new layer | stops |

  Space while stopped plays, in every mode. A mode only changes these
  cells; the rest is shared.
- **7g. Hold to undo, hold to clear - done.** Hold Space undoes the last
  layer, one per hold; hold S clears. A press acts as it goes down, so
  a hold first takes back what its press did - a dub it opened is
  abandoned, as the double press did - then undoes. Holding while
  dubbing counts the dub in progress as the layer undone, and plays.
- **7h. A layer per pass - done.** The third record mode: while dubbing, each
  pass closes the layer one loop length after it opened - not at the
  top, so a phrase crossing it stays whole - and opens the next, on
  the audio thread, reported as a take end. With every layer used, the
  last one keeps taking passes rather than dubbing stopping. Default
  `max_layers` goes to 8 (the mute keys already reach 8; ~10 MB per
  layer at 60 s). Silent passes are kept for now - Hold Space removes
  them; auto-dropping needs a threshold a noisy high-gain signal won't
  fool.
- **7c. Record start mode - done.** One setting: *Instant*, *Delay* (today's
  Record delay), *Count-in* (today's, one bar) or *Detect* - recording
  opens when the input crosses a level, so you don't have to press and
  play at once. Replaces the implicit "metronome on means count-in": a
  count-in is right at home and wrong on stage. Detect can't start a
  part on an off-beat, which is why it's a choice, not the default.
- **7d. Stop mode - done.** *Immediate* (today's), *At the loop end*, or *Fade
  out* over a few seconds. Fades scale the loop bus on the audio thread;
  the loop end is known to the frame. Applies to every stop: the single
  press, the double, and the Stop control.
- **7e. Exact take edges in the saved copy - done.** The audio thread
  reports each take's end on a ring of its own - how many samples were
  its, and whether it was a first take, an overdub, dropped or a clear -
  and the copy splits its stream by those, not by its own state changes.

### 8. Tempo and levels - mini

- **8a. Tap tempo.** A key tapped two or more times sets the tempo from
  the gaps between taps. Not while a loop sets the tempo.
- **8b. Levels on screen.** Click volume (fixed at half scale today)
  and loop volume (only in Settings today) as live controls on the
  looper screen, published through `SharedControl` like the tempo.
- **8c. Time signature.** 4/4, 3/4 or 6/8 for the metronome - the
  scheduler's `Pattern` already has a bar length; the bar rounding,
  the count-in and tempo-from-the-loop all have to follow it.

### 9. Review - plan and implementation

A full stop before the drum machine, which is the biggest addition yet:
step 7 reworked the controls three times, and the drums build on the
scheduler, the tempo and the press logic as they stand. Two passes, then
the fixes they turn up as sub-steps (9a, 9b, ...) before step 10.

**Plan.** Re-read this file top to bottom against what's built: drop or
merge stale items, re-order *Later*, settle the open decisions, and check
steps 10-11 still fit "run and play".

**Implementation.**

| Area | What to check |
|---|---|
| Real time | Everything the callback reaches: no allocation, lock, I/O or unbounded loop - the per-pass split loop included |
| Boundaries | `egui` only in app/ui, `cpal` only in hal, the UI a leaf, HAL with no looping |
| Press logic | `looper.rs` grew press, double, hold, stop, `pressed_from`, `undo_pending` - likely its own module, with one table-driven test over every mode x state x gesture |
| Thread contract | Every `SharedControl` request (clear, remove, redo, abandon, next layer) and the order the callback reads them in, written down in one place |
| Dead code | Paths nothing reaches any more - e.g. the stack's "remove the only layer" stash, `can_stop` beside `can_stop_or_clear` |
| Tests | Gaps, duplicated helpers, anything slow or timing-sensitive |
| Docs | README and the user guide against the app as it behaves; the screenshot |
| Hygiene | `cargo fmt` clean repo-wide (it isn't), the clippy `as_chunks` warning, line endings (`.gitattributes`) |
| Memory | 8 layers x 120 s, held twice - what Settings says against what's allocated |

### 10. Drum machine - extended

Preset grooves, not a step editor: each drum style is a 4x4 grid of
ready-made grooves, **variation** across and **strength** up, and you
pick a cell - as on the NUX. Far more "run and play" than editing steps,
and far less to build. Grooves are step patterns underneath, so the
scheduler plays them as it plays the metronome.

One-shot samples, not pre-recorded loops (whose tempo is baked in, so
changing BPM would mean resampling). A small fixed-size polyphonic voice
pool, pre-allocated to stay RT-safe. Kits load on the UI thread and are
handed to the audio thread the way v1 step 5 hands over a restored loop,
never loaded in the callback; embed one small CC0 kit via
`include_bytes!` to keep the single-exe property, with an optional
folder next to the config for more. The drums follow the loop's tempo,
which is what step 6 was for. A drum level control beside 8b's.

### 11. Mic channel - extended

The second input as a track of its own, with independent record-enable,
mute and level, once the track abstraction (v1 step 2) is in place. Nothing
stands in the way physically: both iD4 inputs are on the same stream and
the input callback already receives every channel interleaved, it just
discards all but the chosen one - so no second device to open and no
cross-device drift. Two things the single-channel path never had to
face: **per-source monitoring** (guitar monitoring is always wanted,
live mic monitoring often isn't - feedback, and hearing yourself dry is
unpleasant), and **levels that differ enormously** between an instrument
input and a mic preamp, so one shared gain isn't enough - eventually an
input meter, so it can be set by eye.

All of the above stays "run and play", not a DAW.

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
