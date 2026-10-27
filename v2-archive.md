# v2 Archive - what's done

This file is historical: the v2 steps that have landed, as they were
planned and built, kept for the reasoning behind them. Later steps
sometimes replaced earlier behaviour - 7f the controls 7b built, 7c the
count-in 6b tied to the metronome, 8c the fixed 4/4 of steps 4-6 - so
read a step as what was true when it landed. What the app does now is in
`README.md` and `docs/user-guide.md`; what's ahead is in `PLAN.md`.

## v2 roadmap (steps 0-8)

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

- **8a. Tap tempo - done.** A key tapped two or more times sets the tempo from
  the gaps between taps. Not while a loop sets the tempo.
- **8b. Levels on screen - done.** Click volume (fixed at half scale today)
  and loop volume (only in Settings today) as live controls on the
  looper screen, published through `SharedControl` like the tempo.
- **8c. Time signature - done.** 4/4, 3/4 or 6/8 for the metronome - the
  scheduler's `Pattern` already has a bar length; the bar rounding,
  the count-in and tempo-from-the-loop all have to follow it.
