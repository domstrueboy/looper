# Guitar Looper - Plan

A practice companion for guitar: a looper, a metronome, and soon drums
and a mic channel, in one light Windows window. ASIO first, WASAPI as
the fallback for machines without it - no DAW, no plugin host.

This file is forward-looking: what's left to build, in what order, and
the decisions that shouldn't drift. What already exists is documented
where it belongs instead of being restated here - `README.md` for
architecture, module layout and build steps, `docs/user-guide.md` for
how the app is actually used, `v1-archive.md` and `v2-archive.md` for
how the finished steps were planned and why.

## Product

Decisions that steer what gets built and in what order.

- **Niche: a practice companion, not "a looper".** Metronome, drums,
  looper and mic in one light window. The competition isn't a DAW but a
  habit: YouTube backing tracks, Moises, a separate metronome app.
- **The principle: open it and play.** No friction between launching and
  playing. Sell the absence of friction - and show it, don't describe it.
- **Who for.** The core: intermediate improvisers. Then teachers (a
  cheap Windows laptop is the typical setup), and singers and
  beatboxers - a mic channel is rare in loopers, so it's a plus of its
  own. Bassists and ukulele players too. Windows first; macOS later.
- **What that implies:**
  - A teacher's laptop often has no ASIO driver, so the WASAPI fallback
    has to be good, not just present.
  - The tuner has to reach a five-string bass's low B (31 Hz) and read
    a ukulele's re-entrant tuning.
  - Hands-free matters: cheap USB foot pedals (~$10) send key presses,
    so they work out of the box - say so wherever the app is described.
    MIDI pedals are the next step.
  - Bluetooth headphones add enough latency to ruin a first impression,
    so the app and the docs warn about them.
  - Effects are a few built-in ones, not a plugin host - no licensing
    pain, nothing to install.
  - A session you come back to tomorrow matters more than exporting a
    file.

## Status

**v1** shipped as `0.1.x`: device picking, live monitoring, a record /
loop / stop cycle, overdub layers, the loop surviving restarts. See
`v1-archive.md`.

**v2** steps 0-8 are done (`v2-archive.md`): the mini/extended switch,
layer mute, redo, the metronome with live tempo, tap tempo and a time
signature, loops snapped to bars with a tempo for every loop, the pedal
feel - two pedals (Space never stops, S always does), three record
modes, hold to undo, record start and stop modes - and loop and click
levels on screen. Step 9, the code review, is under way.

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
- **The looper screen is testable**, on a mock backend.

## Ground rules

- **The real-time boundary is the one thing that must not drift:** no
  locks and no allocations on the audio thread. UI <-> audio talks only
  through `SharedControl`'s atomics or a ring buffer, never a mutex.
- **Stays small:** "run and play", not a DAW - zero friction is the
  product (see *Product*). No upfront abstraction and
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
- **The by-ear pass is owed twice** - see the [post-v1 review](v1-archive.md#post-v1-review---done). Folded into step 10.

## Open decisions

Worth settling before the work they block starts.

- **Open source - decided; which license is still open.** It settles
  the other two questions: CI builds against the ASIO SDK's GPLv3
  option, which an open-source release can meet - the source is public
  anyway, and the released exe goes out under GPLv3's terms - and an
  open-source project can sign for free (SignPath Foundation). The
  source's own license is the choice left: GPL-3.0 keeps every fork open
  too; MIT keeps the most options for a paid tier later, since nothing
  but the author's own code would need relicensing. Either way a
  `LICENSE` file, the `license` field in `Cargo.toml`, and a line in the
  README, in step 11. Moving to closed source later would mean
  Steinberg's proprietary ASIO SDK licence (free, but signed for) in
  place of the GPLv3 one.
- **The order after the release** (steps 12-16) is a proposal: tuner
  first because it's small and keeps people from giving up, sessions
  before drums because a backing track you can come back to is the
  practice-companion promise. Reorder freely.

## Roadmap

Ordered. One step at a time, each a commit (see *Ground rules*).

### 9. Code review - under way

Read with fresh eyes before the drums build on the scheduler, the tempo
and the press logic. Checked and clean: nothing the callback reaches
allocates, locks or does I/O, and every loop in it is bounded - the
per-pass split loop included, since a pass only ends with a slot free
for the next; `cpal` only in hal, `egui` only in app/ui, the HAL with no
looping; the suite runs in about a second with no sleeps. The findings,
in the order they're fixed:

- **9a. Hygiene - done.** `cargo fmt` repo-wide, the clippy `as_chunks`
  warning, CRLF working copies back to LF, and `rustfmt.toml` with
  `newline_style = "Unix"` so `cargo fmt` stops preserving CRLF.
- **9b. Saving off the UI thread - done.** A save wrote every layer
  inside `tick`, ~90 MB at 8 x 60 s, delaying the presses read in that
  frame. Now only what changed is written, on `loop_writer.rs`'s thread,
  which also takes the tempo and the clear's delete so they stay in
  order.
- **9c. The copy bounded - done.** `LoopMirror` kept a dub as played,
  so a long one held every pass. Now a dub folds onto one loop length
  as it arrives, a first take holds only what the stack kept, and
  Settings says the copy can take as much memory again.
- **9d. Real-time edges.** Looping with an empty stack (a take that got
  no input, e.g. a dead capture end) makes `add_on_loop` retrigger the
  click at the top of every block - a buzz. And `set_bpm` builds a
  `Scheduler`, whose `assert!(bpm > 0.0)` is a panic reachable from the
  callback; ignore a tempo that isn't finite and positive instead.
- **9e. Dead code.** `LoopStack`'s "remove the only layer" stash
  (`removed_len`, the `count == 0` branches of remove and redo) - undo
  never takes the first layer; `can_stop` (tests only); the comment on
  `active_loop_tempo` about a removed loop redo could bring back;
  `Pattern::METRONOME` and `Voice::is_playing` (tests only).
  `Pattern::new` stays - the drums need it.
- **9f. Thread contract.** One table in `shared_control.rs`: every
  request and level, who sets it, when the callback reads it and in
  what order (next layer, then state change with abandon, then clear,
  remove, redo, mutes, then pending stop inside the playing branch).
  Plus why `request_stop(Immediate)` stores the same 0 as "none".
- **9g. Press logic.** First a table-driven test over record mode x
  state x gesture (press, double, hold, stop, stop-hold) to the
  resulting state and requests. Then press, double, hold, stop,
  `end_take` and `clear` move to `looper/pedal.rs` - a child module
  still sees `LooperState`'s fields, so it's a move, not a redesign.
- **9h. The UI is not a leaf.** `looper_ui_egui::looper::render` takes
  `&mut LooperState` and calls undo, redo, mutes, tempo, levels and the
  mode switch directly; `Action` only carries `Start` and
  `OpenSettings`. Decided: the screen takes `&LooperState` and returns
  its intents - the button latches included - and the app applies them,
  as AGENTS.md says.
- **9i. Tests.** `temp_dir` is written four times; one shared test
  helper.
- **9j. Docs.** README's module diagram puts `app.rs` in looper-core
  and lacks `tempo.rs`, `tap_tempo.rs`, `metronome.rs`, `scheduler.rs`
  and `click.rs`. Stale doc comments: `state_machine.rs` (long press
  clears; "the overdub control"; Arming "treated like Idle"),
  `looper-ui-egui`'s "`egui` nowhere else", `main.rs`'s `ui/`. The user
  guide read against the app. The screenshot needs retaking by hand.

### 10. Manual test plan

A checklist in `docs/test-plan.md` for a person at the real app, once
step 9's fixes are in, so it tests what ships. Every control on both
screens, every setting, each record mode, record start and stop mode
through its full cycle, and what should survive a restart. Plus what only
hardware shows, owed since the post-v1 review: the by-ear pass (click
against loop, overdubs in time, no click at the loop seam, a smooth
fade), an unplugged interface mid-loop, a USB foot pedal (which keys it
sends - step 11 makes those work), WASAPI
on a laptop with no ASIO
driver, and Bluetooth headphones - to see how bad it is and word the
warning from that.

### 11. Release 0.2

- **License:** the `LICENSE` file and fields - see *Open decisions*.
- **Signed exe,** through SignPath Foundation's free signing for
  open-source projects (check its current terms when applying).
  SmartScreen's "unknown publisher" scares people off more than it
  seems to.
- **Pedal keys:** **PageDown** works as Space and **PageUp** as S, the
  keys fixed-key foot pedals and presentation clickers send, so a cheap
  pedal works with no setup. Held and double-pressed like the keys
  they stand for. Widened to whatever else step 10's pedal turned out
  to send.
- **Bluetooth warning** in Settings when an output looks like Bluetooth,
  and in the docs - wired headphones or the interface's own output.
- **CI actually verified:** the existing release workflow's zips have
  never been checked (see *CI / releases*); a minor bump to 0.2.0.
- **README as the landing page:** a GIF demo rather than screenshots -
  launch, riff, loop, play over it - the USB pedal note, and the
  Bluetooth note. A Telegram channel for feedback, linked from it.

### 12. Tuner

Pitch detection on a decimated copy of the input, analysed on a
background thread or per UI frame - never in the callback, whose budget
FFT or autocorrelation cost would break. Guitar, bass down to a
five-string's low B (31 Hz), ukulele. A small readout, mini mode.

### 13. Sessions

"Built a backing track - came back to it tomorrow." The loop already
survives a restart as one auto-saved session; this makes several, named,
each carrying what makes it playable again: layers, tempo and time
signature, levels, and later the drum groove and the mic track. Save,
open, delete. Absorbs what used to be separate *Later* items: numbered
memory slots (the NUX has 99) and importing a WAV as a backing track.

### 14. Drum machine - extended

Preset grooves, not a step editor: each drum style is a 4x4 grid of
ready-made grooves, **variation** across and **strength** up, and you
pick a cell - as on the NUX. Far more "run and play" than editing steps,
and far less to build. Grooves are step patterns underneath, so the
scheduler plays them as it plays the metronome, with `Voice`'s gain for
accents.

One-shot samples, not pre-recorded loops (whose tempo is baked in, so
changing BPM would mean resampling). A small fixed-size polyphonic voice
pool, pre-allocated to stay RT-safe. Kits load on the UI thread and are
handed to the audio thread the way a restored loop is, never loaded in
the callback; embed one small CC0 kit via `include_bytes!` to keep the
single-exe property, with an optional folder next to the config for
more. The drums follow the loop's tempo and time signature. A drum level
beside the loop and click levels.

### 15. Mic channel - extended

The second input as a track of its own, with independent record-enable,
mute and level. Nothing stands in the way physically: both iD4 inputs
are on the same stream and the input callback already receives every
channel interleaved, it just discards all but the chosen one - so no
second device to open and no cross-device drift. Not in place yet: a
track abstraction above the layer stack - the stack is one track's
layers today - so this step starts by adding one. Two things the
single-channel path never had to face: **per-source monitoring** (guitar
monitoring is always wanted, live mic monitoring often isn't - feedback,
and hearing yourself dry is unpleasant), and **levels that differ
enormously** between an instrument input and a mic preamp, so one shared
gain isn't enough - eventually an input meter, so it can be set by eye.

### 16. Effects

A few built-in effects, not a plugin host: **drive**, **delay** and
**reverb**, on the guitar and on the voice - an insert stage between
input and the mix, which the signal path leaves room for. Then the NUX's
looper-shaped extras: an **octave down** for a bass part on the guitar,
a **boost** for soloing over the loop.

All of the above stays "run and play", not a DAW.

## Later (unordered)

- Trim loop start/end, and the waveform rendering it needs anyway
- **Export** the mix to a WAV file - reuses the saved loop's WAV writer.
  After sessions, which matter more.
- **A/B song parts** (extended) - two loops, switched between for verse
  and chorus, as on the NUX Dual Loop Stereo. Sharing one tempo, and a
  switch *waits for the end of the current loop* rather than cutting it
  off. Each part would want its own layer stack, so memory doubles, and
  its own drum groove; worth doing after the mic channel has shown what
  a second track costs. The NUX's other mode, parallel tracks muted in
  and out for quiet sections, is mostly what layer mute already does.
- **MIDI footswitches** - a dependency, so a deliberate choice. Pedals
  that send key presses already work, on Space and S.
- **Split output** - loop and guitar on the left channel, drums or click
  on the right, so the click can go to headphones only, or each to its
  own amp. The output is mono duplicated to every channel today.
- **NAM model loading and a cabinet simulation** - beyond step 16's
  built-in effects, for going straight to a mixer.
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

## Launch - after step 11

Not code, but it decides whether anyone uses the code.

- **Show, don't tell:** a 30-second demo - launch, riff, loop, play over
  it - as the README's GIF and as Shorts / TikTok.
- **Where:** RU - Habr (the "tried everything, wrote my own" story),
  Pikabu, VK and Telegram guitar communities. EN - r/guitar and
  r/guitarlessons (read their self-promotion rules first), Show HN, KVR
  Audio, guitar schools' Discord servers.
- **The multiplier:** free for teachers and YouTubers - one video is
  hundreds of installs, more than any advert.
- **Money - not now.** Free, collecting feedback, which is worth more.
  Later, in order of friction: donations (Boosty / Patreon), then a
  one-off $10-15, then paid effects or export. It won't pay the bills;
  it's a portfolio and a community.
- **Budget for support.** Drivers and questions take more time than
  development - plan for it rather than be surprised.

## CI / releases

`.github/workflows/release.yml`: every push to `main` bumps the patch
version, commits and tags it, builds on `windows-latest` against the
GPLv3 ASIO SDK, and attaches a zip to a GitHub Release - one job, one
run. It deliberately doesn't rely on the tag push triggering a second
run, since pushes made with the default `GITHUB_TOKEN` don't trigger
workflows.

Step 11 verifies the rest. Confirmed working: the bump/commit/tag/push half - the bot produced
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
