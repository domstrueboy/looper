# Looper Pedal - Manual Test Plan

A checklist for a person at the real app, with a real interface and a
guitar. The automated tests cover the logic; this covers what only the
app on a machine can show - what the screen says, what you hear, and
what the hardware does.

## How to use it

- Work top to bottom. Later sections assume the earlier ones passed.
- Each row is one thing to do and what should happen. Tick **✓** when it
  does; when it doesn't, write the row's ID and what happened instead in
  *Findings* at the end.
- "Press" means a short press, under the hold time. "Hold" means past
  it - **2.0 s** by default (**Hold to undo / clear** in Settings).
- Space and the round button are interchangeable, as are S and **Stop**:
  try each row with the key, and spot-check a few with the button.

**You need:** the release build (`looper_pedal.exe`), an audio
interface with an ASIO driver, a guitar plugged into one of its inputs,
and headphones or monitors on its output.

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
| S1 | Look at the screen | Title, then the list of settings, left-aligned, with **Start** centred below it. The list scrolls if the window is short; **Start** never scrolls away | |
| S2 | **Driver** | Shows only with more than one driver: **ASIO** first, **WASAPI** after it | |
| S3 | **Device** on ASIO | One picker named **Device**; your interface is in it | |
| S4 | **Driver** → **WASAPI** | **Device** becomes **Input**, an **Output** picker appears, and an amber note says takes can land up to ~20 ms off | |
| S5 | **Driver** back to **ASIO** | **Output** and the note go; **Input** reads **Device** again | |
| S6 | **Sample rate** | Lists only rates the device takes; 44100 Hz among them | |
| S7 | **Input channel** | One entry per input the interface has (e.g. Input 1, Input 2) | |
| S8 | **Record start** → each option in turn | **Delay** shows **Record delay** under it (0 reads **off**); **When you play** shows **Start level** in dB; **Instant** and **Count-in** show nothing extra | |
| S9 | **Stop** → **Fade out** | **Fade time** appears under it, in seconds; gone again for the other two | |
| S10 | Hover any **Record start**, **Stop** or **Record mode** option | A one-line hint says what it does | |
| S11 | **Hold to undo / clear**, **Metronome**, **Latency**, **Max loop length**, **Max layers** | Each slider moves within its range and shows its unit (s, BPM, ms) | |
| S12 | **Time signature**, **Record mode** | Radio buttons: 4/4, 3/4, 6/8; REC > PLAY, REC > DUB, REC > DUB, a layer per pass | |
| S13 | Drag **Max loop length** and **Max layers** | The memory line under them follows at once: "Reserves N MB ..., up to 2N MB with every layer recorded" | |
| S14 | Set **Record start** to **Instant** and **Hold to undo / clear** to 1.0 s (it makes the rest quicker), leave the rest at their defaults, then **Start** | The looper screen opens, in **Mini** | |
| S15 | Close the app and open it again | It opens straight on the looper, not Settings | |
| S16 | **⚙** (top right) | Back on Settings, with every choice as you left it | |
| S17 | Unplug the interface, then **Start** | Settings stays, with a red line saying why; plug it back in and **Start** works | |

## 2. Looper screen - Mini

Record start is **Instant** from S14, so a press records at once.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| L1 | Look at the screen | Gray dot, **Empty**; round button **⏺**; **Stop (S)**, **Undo**, **Redo (R)**; the metronome row; **Loop** and **Click** at 100%; **Layers: 0/8**; four hint lines | |
| L2 | Play, without recording | You hear the guitar live, clean, in both ears | |
| L3 | Press Space | Red pulsing dot, **Recording**, the seconds counting up; button **⏹** | |
| L4 | Press Space again | Green dot, **Looping**, "N.Ns loop", a progress bar running; button **⏺**; **Layers: 1/8** | |
| L5 | Press Space | Orange pulsing dot, **Overdubbing**; button **▶** | |
| L6 | Press Space | **Looping**, **Layers: 2/8** | |
| L7 | Press S | Amber dot, **Stopped**; the bar frozen where it was | |
| L8 | Press Space | **Looping** again, from where it stopped | |
| L9 | **Undo** | **Layers: 1/8**; the dub is gone from what you hear; **Redo** lights up | |
| L10 | **Redo** (or R) | **Layers: 2/8**; the dub is back | |
| L11 | Hold Space while it loops | While held, the button shows **⟲**; on release **Layers: 1/8**, still looping | |
| L12 | Hold Space again | Nothing more goes: the first recording is never undone | |
| L13 | Press Space twice quickly | **Stopped**, no new layer left behind | |
| L14 | Hold S | While held, the stop button reads **🗑 Cleared**; then **Empty**, **Layers: 0/8**, **Undo** and **Redo** greyed | |
| L15 | Record a loop, then try **Mini / Extended** | Greyed while it plays; hovering says "Stop the loop to switch". Works once it's **Stopped** or **Empty** | |
| L16 | **M** (or the **Metronome** button) | The button lights; you hear a click, beat one higher | |
| L17 | With no loop: **,** and **.**, then **−** and **+** | The tempo steps by one BPM; holding a key repeats | |
| L18 | **T** four times on a steady beat (or **Tap**) | The tempo follows your taps | |
| L19 | Record a loop with the metronome off | The tempo shows a tenth (e.g. **92.3 BPM**); **−** and **+** become **½** and **2×**; **Tap** is greyed | |
| L20 | **½**, then **2×** | The tempo halves and doubles; each greys out where it would leave 40-300 BPM | |
| L21 | Drag **Loop** to 0%, then back | The loop goes silent and returns; your live guitar stays the same throughout | |
| L22 | Drag **Click** | The click's level follows, as you drag | |
| L23 | Close and reopen the app | **Loop** and **Click** are where you left them | |

## 3. Looper screen - Extended

Switch to **Extended** with a loop of three layers recorded and stopped.

| ID | Do | Expect | ✓ |
|---|---|---|---|
| E1 | Look under the hint lines | A row of buttons **1**, **2**, **3**, all lit, and "1-3: mute / unmute a layer" | |
| E2 | Play, then click **2** (or press **2**) | Button 2 dims; that layer goes silent, the others play on | |
| E3 | Click **2** again | It lights and is heard again | |
| E4 | Mute **3**, then **Undo** | Layer 3 goes; record a new dub - it comes in unmuted | |
| E5 | Mute a layer, stop, switch to **Mini** and back | Every layer plays again: Mini unmutes all | |
| E6 | Hold S | The row is replaced by "Layers appear here once there's a loop" | |

## Findings

One line per row that didn't do what it says: the ID, what happened,
and whether it happens every time.

| ID | What happened | Every time? |
|---|---|---|
| | | |
