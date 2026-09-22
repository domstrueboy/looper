# v1 Archive — what's done

This file is historical. The released feature set (v1, shipped as `0.1.x`)
is documented here. Forward-looking items are in `PLAN.md` as "v2".

## v1 roadmap (ordered)

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
purpose: v2's metronome count-in is this same window with clicks in it,
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
executable was dropped in the post-v1 review, no such file being likely
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

## Post-v1 review - done

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
  `Idle`, which would work today. v2's count-in needs the callback to
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
