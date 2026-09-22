# Looper Pedal — AI Coding Assistant Guide

Captures project-specific conventions that can't be inferred from code alone.

## Architecture

Four crates, strict dependency direction:

```
looper-app      — binary, eframe glue, window shell
looper-ui-egui  — egui screens (leaf, reads model, returns Action)
looper-core     — pedal logic (no cpal, no egui)
looper-hal      — audio devices, formats, streams (no looping)
```

Dependency: app → ui → core → hal. Neither ui nor core depends on the other end.

**Hard boundaries** — never cross:
- `egui::` types only in `looper-app` and `looper-ui-egui`
- `cpal` types only in `looper-hal`
- UI crate is a leaf: `render(ui, model) -> Option<Action>`. Never writes actions back.
- HAL knows about devices, never about loops/layers/takes.

## Conventions

| Item | Pattern | Example |
|---|---|---|
| Crate | `looper-*` (kebab) | `looper-core` |
| Module file | `snake_case.rs` | `state_machine.rs` |
| Type | PascalCase | `LoopState`, `LoopStateMachine` |
| Function / method | snake_case | `press()`, `toggle_overdub()` |
| Variable / field | snake_case | `loop_len`, `play_pos` |
| Constant | UPPER_SNAKE_CASE | `MAX_BLOCK_FRAMES`, `DEFAULT_VOLUME_PCT` |
| Test file | `{name}_tests.rs` | `state_machine_tests.rs` |
| Test function | snake_case, scenario | `starts_idle`, `press_cycles_full_sequence` |

**Tests** — sibling files only, never inline:
```rust
#[cfg(test)]
#[path = "state_machine_tests.rs"]
mod tests;
```
Test files use `use super::*;`.

**Doc comments** — prose style, explain *what* and *why*, not *how*. `//!` for modules, `///` for items. Keep module docs to 2-4 lines. Inline comments only when the "why" isn't obvious from code.

## Style

- **Be short.** If a sentence can be cut without losing meaning, cut it.
- **No filler.** Skip "Here's the plan", "Let me explain", "As you can see".
- **Prefer tables over prose** for structured info (naming, settings, state transitions).
- **Code speaks for itself.** Don't describe what code does — explain *why* it's there.
- **One idea per sentence.** If a sentence does two things, split it.
- **No apologies, no hedging.** Don't say "I think" or "maybe". State what's correct.
- **Diff over summary.** When changing code, show the change — don't narrate it.

## Rules (Non-Negotiable)

1. **Audio thread**: no allocation, no locks, no blocking, no I/O. Bounded work only.
2. **Thread boundary**: UI → audio via atomics (`SharedControl`). Audio → UI via atomics + ring buffers. No mutex in the audio path.
3. **Samples are `f32` everywhere above the device layer**, full scale at +/-1.0.
4. **Never cross crate boundaries**: `egui` in core/hal, `cpal` in core.
5. **Tests in sibling files**, never inline.
6. **Adding dependencies to HAL** only if truly device-level.

## Working Process

This is how work flows — the agent follows it, the user controls it.

1. **Plan first.** The user defines the feature. The agent splits it into steps before writing code.
2. **One step at a time.** Implement one step, then pause.
3. **Describe what changed.** Short summary of what was done — not what the agent *plans* to do, what it *did*.
4. **Commit message.** A concise message (1-2 lines) focused on *why*, not *what*.
5. **Pause for review.** Never commit without explicit user approval. The agent must ask: "Ready to commit?"
6. **Fixes first.** If the user requests changes, apply them in the same step — don't start a new one.

The agent must **never commit on its own**. Every commit requires an explicit user go-ahead.

## Plan

`PLAN.md` at the repo root is the single source of truth for what's left to build. It's forward-looking: ordered items, open decisions, what's blocked on what. What's done is in README.

- Agents may create session-specific plans in their own dirs (`.gigacode_vsc/plans/`, `.claude/plans/`, etc.) — those are scratch, ignored by git.
- When an agent completes work, it updates `PLAN.md` — mark items done, remove open decisions, note new ones if they surface.
- Keep it ordered. Don't dump a list — sequence matters when the next agent picks it up.
- Don't duplicate what's in README. `PLAN.md` is about what's *ahead*, not what's behind.
