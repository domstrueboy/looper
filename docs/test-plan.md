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

## Findings

One line per row that didn't do what it says: the ID, what happened,
and whether it happens every time.

| ID | What happened | Every time? |
|---|---|---|
| | | |
