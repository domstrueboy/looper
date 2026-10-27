# Looper Pedal - Manual Test Plan

A checklist for a person at the real app, with a real interface and a
guitar. What the screens show and ask for is tested automatically -
every label, which controls are greyed when, what a click or a key asks
the looper to do (`cargo test -p looper-ui-egui`) - and what the pedal
does from every state in every record mode is too (`cargo test -p
looper-core`). This keeps what only a person at the app can judge: how
it looks, what you hear, how it feels in real time, and what the real
device and a restart do.

## How to use it

- Work top to bottom. Later sections assume the earlier ones passed.
- Each row is one thing to do and what should happen. Tick **✓** when it
  does; when it doesn't, write the row's ID and what happened instead in
  *Findings* at the end.
- "Press" means a short press, under the hold time. "Hold" means past
  it - **2.0 s** by default (**Hold to undo / clear** in Settings).
- Space and the round button are interchangeable, as are S and **Stop**:
  try each row with the key, and spot-check a few with the button.

**You need:** the release build - `cargo build --release -p looper-app`
in the repo, which leaves `target\release\looper_pedal.exe` - an audio
interface with an ASIO driver, a guitar plugged into one of its inputs,
and headphones or monitors on its output. Sections 11-13 need more, and
say what.

**Start clean:** close the app, then delete
`%APPDATA%\looper-pedal\` (the config and any saved loop), so the app
opens on Settings as it would the first time.

| Build | Interface | Driver | Date | Tester |
|---|---|---|---|---|
| | | | | |

## 1. Settings screen

Open the app with no saved config: it opens on Settings.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| S1 | Look at the screen | The settings left-aligned and readable at the window's width, with **Start** centred below them. The list scrolls if the window is short; **Start** never scrolls away | |
| S2 | **Driver** | **ASIO** first, **WASAPI** after it | |
| S3 | **Device**, **Sample rate**, **Input channel** on ASIO | Your interface; rates it takes, 44100 Hz among them; one entry per input it has | |
| S4 | Hover a **Record start**, **Stop** or **Record mode** option | A one-line hint says what it does | |
| S5 | Drag **Max loop length** and **Max layers** | The memory line under them follows as you drag | |
| S6 | Set **Record start** to **Instant** and **Hold to undo / clear** to 1.0 s (it makes the rest quicker), leave the rest at their defaults, then **Start** | The looper screen opens, in **Mini** | |
| S7 | Close the app and open it again | It opens straight on the looper, not Settings | |
| S8 | **⚙** (top right) | Back on Settings, with every choice as you left it | |
| S9 | Unplug the interface, then **Start** | Settings stays, with a red line saying why; plug it back in and **Start** works | |

## 2. Looper screen - Mini

Record start is **Instant** from S6, so a press records at once.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| L1 | Look at the screen | Everything fits the window, nothing clipped; the hint lines read at a glance | |
| L2 | Play, without recording | You hear the guitar live, clean, in both ears | |
| L3 | Record, play, dub, play, stop, play - Space, Space, Space, Space, S, Space | You hear each: the take comes back on the loop, the dub joins it from the next pass, S cuts it, Space resumes from where it stopped. The dot pulses while recording and dubbing | |
| L4 | **Undo**, then **Redo** | The dub goes from what you hear, then comes back exactly as it was | |
| L5 | Hold Space while it loops | While held, the button shows **⟲**; on release the newest layer is gone | |
| L6 | Hold S | While held, the stop button reads **🗑 Cleared**; then **Empty** | |
| L7 | Press Space twice quickly while it loops | It stops, and no sliver of a layer is left - the double feels like one gesture, not two presses | |
| L8 | **M** | You hear the click, beat one higher | |
| L9 | Hold **.** | The tempo keeps climbing while the key is held | |
| L10 | Drag **Loop** to 0%, then back | The loop goes silent and returns, as you drag; your live guitar stays the same throughout | |
| L11 | Drag **Click** | The click's level follows, as you drag | |
| L12 | Close and reopen the app | **Loop** and **Click** are where you left them | |

## 3. Looper screen - Extended

Switch to **Extended** with a loop of three layers recorded.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| E1 | Play, then mute and unmute a layer by its button and by its number key | It goes silent and comes back; the others play on throughout | |

## 4. Record start

From **Empty** each time (hold S), changing **Record start** in Settings
between rows. Metronome off unless the row says.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| R1 | **Instant**: play a note exactly as you press Space | The note is at the very start of the loop - nothing before it lost | |
| R2 | **Delay** (4 s): press Space | Blue pulsing **Get ready**, the seconds counting down over a filling gray bar, button **✖**; recording starts as the bar fills | |
| R3 | **Delay**: press Space, then Space again before it runs out; again with S | Either one calls it off: **Empty**, nothing recorded | |
| R4 | **Count-in**, 4/4: press Space | One bar of clicks - four, beat one higher - even with the metronome off; recording starts on the next beat one. Play on that beat and it's the top of the loop | |
| R5 | **Count-in**, 3/4 | Three clicks, not four | |
| R6 | **When you play**: press Space, wait, then play | **Play to start**, no countdown, as long as you wait; recording starts with your first note, its attack intact at the top of the loop | |
| R7 | **When you play**: press Space and don't play | Hum and string noise don't start it at the default **Start level**; if they do, note the level that stops it | |

## 5. Ending the first take, with the metronome on

Record start **Instant**, metronome on, 120 BPM: a bar is 2 seconds.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| B1 | Record, and end the take just after a bar line | The loop is cut back to that bar line; the click lands on the top of the loop every pass | |
| B2 | Record, and end it more than half way into a bar | **Recording to the bar line** until the next one; whatever you stop playing in that time stays silent in the loop | |
| B3 | As B2, then press again while it waits | It stops right away; the rest of the bar is silent | |
| B4 | Metronome off: record a loop of any length | The loop is exactly as long as you played it; the button shows its tempo to a tenth. Turn the click on: it fits the loop, beat one at the top | |

## 6. Record modes

Record start **Instant**. **Max layers** at its default, 8, unless the
row says.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| M1 | **REC > PLAY**: press to end the take, press, press, then S while dubbing | Plays; dubs; plays again with the dub kept; S during a dub stops and keeps it | |
| M2 | **REC > DUB**: press to end the take | Straight into a dub, no silent pass between: the take you just played plays back as you dub over it | |
| M3 | **REC > DUB**, dubbing: press at the top of a pass | **Layers** goes up by one and the next dub starts at once - nothing missed at the join | |
| M4 | **REC > DUB**, dubbing: S, then S | Keeps the layer and plays; then stops | |
| M5 | **REC > DUB**, **Max layers** 3: dub until the last layer is recording | The round button turns from **⏺** to **▶**, and the next press plays rather than dubs | |
| M6 | **REC > DUB, a layer per pass**: end the take, then keep playing for three passes with no press | **Layers** goes up each pass, a loop's length after the dub began - not at the top. A phrase played across the top of the loop stays in one layer | |
| M7 | As M6, then hold Space | Only the last pass goes | |
| M8 | As M6, with **Max layers** 3 | Once the last layer is reached, dubbing carries on into it rather than stopping | |
| M9 | **A layer per pass**, dubbing: press | Keeps the layer and plays | |

## 7. Stop modes

A loop of two layers playing, **REC > PLAY**.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| T1 | **Immediately**: S | Silent at once | |
| T2 | **At the loop end**: S mid-loop | **Stopping at the loop end**, the button **Stop now (S)**; it plays to the end, then **Stopped**. The next Space starts from the top | |
| T3 | **At the loop end**: S, then S again before the end | Stops right there | |
| T4 | **At the loop end**: S, then Space before the end | It plays on, as if never stopped - and no dub opens | |
| T5 | **Fade out** (3 s): S | **Fading out**: an even fade to silence over three seconds, no click at the end; then **Stopped** | |
| T6 | **Fade out**: S, then Space part-way | It plays on, back at full level. Note whether the jump back is a click | |
| T7 | **At the loop end**, dubbing: S | The dub is kept at once, then the loop plays to its end | |
| T8 | Any stop mode, recording the first take: S | It stops at once - there's nothing to play out | |

## 8. What's kept

| ID | Do | Expect | ✓ |
|---|---|---|---|
| K1 | Record three layers, mute one (Extended), halve or double the tempo (whichever is lit); close the app; open it | The loop is back, **Stopped**, three layers, none muted, the tempo as you left it; Space plays it | |
| K2 | **⚙**, change nothing, **Start** | The same loop back, **Stopped** | |
| K3 | Record a dub, and end the app from Task Manager straight after | Reopened, the dub is there | |
| K4 | With a loop saved: change **Sample rate**, **Start** | No loop - dropped, not played at the wrong speed | |
| K5 | A loop of three layers saved: set **Max layers** to 2, **Start** | Two layers back; the third is dropped | |
| K6 | **Undo**, close the app, open it | **Redo** is greyed: what undo dropped doesn't survive a restart | |

## 9. By ear

The pass owed since v1: the move to `f32` and the swap onto the device
layer each changed everything the signal passes through, and only ears
on real hardware can say the result is right. ASIO, the latency at its
default, metronome on at 120 BPM unless the row says.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| H1 | Record four bars against the click, then let it loop for two minutes | Click and loop stay locked together - no drift, no flam creeping in | |
| H2 | Stack four layers, each a tight muted strum on every beat | The strums land together, layer on layer - no smear that grows with each one | |
| H3 | Record a loop ending on a chord that rings across the top | No click, gap or bump at the seam, pass after pass | |
| H4 | Play through the app for ten minutes - live, looping, dubbing | **Audio fell behind** never appears. If it does: raise **Latency** until it doesn't, and note the lowest that holds | |
| H5 | Play live, with nothing recorded | The monitoring delay is small enough to play through comfortably | |

## 10. An interface unplugged

| ID | Do | Expect | ✓ |
|---|---|---|---|
| U1 | With a loop playing, unplug the interface | A red **Audio stopped: ...** line and **Choose a device** under the title; the app itself keeps responding | |
| U2 | Plug it back in, **Choose a device**, **Start** | The loop is back, **Stopped**, every layer | |
| U3 | As U1, but in the middle of a dub | Afterwards, every layer before the dub is back; the dub itself is lost | |

## 11. A USB foot pedal

You need: a USB foot pedal or presentation clicker - the kind that
types keys.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| F1 | Open Notepad and press each switch | Note in *Carry forward* what each one types - often PageDown and PageUp, sometimes arrows, B or Space | |
| F2 | Hold a switch down in Notepad for three seconds | Note whether the key repeats while held, or arrives once - a pedal that sends down and up together can't hold, so hold-to-undo won't work from it | |
| F3 | If it sends PageDown and PageUp - or can be set to Space and S - play a loop from it | The first switch is the main control, the second Stop; press, double press and hold all work from your foot as they do from the keyboard | |

## 12. A laptop with no ASIO

You need: a Windows laptop with no ASIO driver installed, and the same
exe copied to it.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| W1 | Open the app | Settings, with no **Driver** row - WASAPI is the only one - an **Input** and an **Output** picker, and the amber note | |
| W2 | Look at what **Input** picked by itself | Note its name. If it's a "Loop-back", "Stereo Mix" or similar, it records what's playing, not you - pick the real input and note that it was needed | |
| W3 | Record and stack three layers against the click | Note how loose it feels against ASIO | |
| W4 | Play for five minutes at the default latency | Note whether **Audio fell behind** appears, and the lowest **Latency** that keeps it away | |

## 13. Bluetooth headphones

You need: Bluetooth headphones, paired with the machine.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| BT1 | WASAPI, the headphones as **Output**: play live | Note how far behind your playing it sounds: playable, distracting, or unusable | |
| BT2 | Look at the **Output** list | Note exactly how the headphones are named - every entry they appear as. Step 11's warning has to recognise them | |
| BT3 | If the headphones' microphone shows as an **Input**, or the sound drops to telephone quality | Note it: that's the headset profile, and the warning should mention it | |

## Carry forward

Step 11 builds on these, so fill them in even if every row passed.

| What | Found |
|---|---|
| Keys the foot pedal sends (F1), and whether it repeats when held (F2) | |
| Lowest latency with no underruns: ASIO (H4), WASAPI (W4) | |
| What WASAPI picked as **Input** by itself (W2) | |
| How the Bluetooth headphones are named (BT2), and how bad they sound (BT1) | |
| Whether cancelling a fade clicks (T6) | |

## Findings

One line per row that didn't do what it says: the ID, what happened,
and whether it happens every time.

| ID | What happened | Every time? |
|---|---|---|
| | | |
