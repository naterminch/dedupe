# Design system

The GUI is a Native Shell (nsh) Rust app. `nsh-base` supplies behavior without a
look; everything visual is the app's own, and it lives in two places:

- `src/ui/palette.rs` — the only file with raw colour literals. Semantic roles,
  each with a light and a dark value: `BACKGROUND`, `SIDEBAR`, `CARD`, `PANE`,
  `ELEVATED`, `INSET`, `BORDER`, `TEXT`, `MUTED`, `ACCENT`, `ON_ACCENT`,
  `ACCENT_WASH`, `KEEP`, `KEEP_WASH`, `SELECT`, `SELECT_WASH`, `DANGER`, `SCRIM`.
  Depth comes from the surface ramp plus one-pixel hairlines, never from shadows.
- `src/ui/widgets.rs` — the styled controls every pane composes: `card`,
  `heading`/`page_heading`/`panel_title`/`caption`/`field_label`,
  `primary_button`, `secondary_button`, `ghost_button`, `danger_button`,
  `icon_button`, `icon_ghost_button`, `icon_chip`, `toggle_switch`, `chip`,
  `divider`, `field`, `hint`, `help_dot`, `icon`.

Scales are finite tokens, not per-call values: `radius::CARD` (18) and
`radius::CONTROL` (12); `space::{XXS..XXL}` = 2, 4, 8, 12, 16, 24, 32;
`text::{CAPTION, BODY, HEADING, PAGE}` = 12, 14, 20, 28 pt. Text is set in
points and the engine applies the user's text scale to all of it, so the user's
text-size setting is honoured without a second scale in the app.

## Interaction states

`nsh-base` draws no pressed, focused or disabled appearance, so every control
carries all three:

| State | Look | Where |
| --- | --- | --- |
| hover | one surface step up (`ELEVATED`, `PANE`) or a 8% opacity wash | `.hover(..)` on each control |
| pressed | 90% opacity (buttons, icon chips) or `ELEVATED` (rail, folder rows) | `.active(..)` |
| keyboard focus | 2pt accent outline, only after the keyboard moved focus | `.focus_visible(focus_ring)` |
| disabled | 45% opacity | `.disabled_style(..)` |

Small icon targets (30pt and below) also take `.hit_slop(EdgeInsets::all(px(4.0)))`.
Selection is not a state: a selected chip or rail slot is a static style computed
from the owner's value, so it survives rebuilds.

## Motion inventory

Durations are tokens in `widgets::timing`: `FAST` (150 ms) for a control
answering the pointer, `BASE` (180 ms) for a look change, `SLOW` (240 ms) for a
surface arriving. Every one goes through `widgets::motion(ms)`, which reads the
platform's reduce-motion preference reactively and returns 0 when it is on: the
same end state arrives without the wait.

Arrivals use nsh's motion presets through two helpers: `widgets::entering()`
(fade while rising 24 pt, 450 ms) for a whole page, and `widgets::fading_in()`
(fade only, 300 ms) for anything that must not move under the reader. Both are
created where the pane is built, so a pane that mounts again plays again, and
nsh lands them at their end state when reduce-motion is on.

| Change | Tool | Tokens |
| --- | --- | --- |
| Page switch (workspace ⇄ Settings) | `entering()` bound to the page | `FadeInUp`, 450 ms |
| First-run hero appearing | `entering()` | `FadeInUp`, 450 ms |
| Results list rebuilt (scan landing, filter, delete, Show more) | `fading_in()` on the list | `FadeIn`, 300 ms |
| Button hover wash | colour transition, ease-out | `BASE` |
| Button press dim and sink | state style + scale 0.97, transitioned | `FAST`, opacity 0.9 |
| Rail slot moving between pages | background transition | `BASE`, `PANE` |
| Group disclosure chevron opening | transform rotation 0° → 90° | `FAST`, one glyph turned rather than two swapped |
| Toggle switch thumb | `transition-transform` class on the thumb | `FAST` |
| Text-field focus edge | colour transition | `BASE`, `BORDER` → `ACCENT` |
| Keeper pick and duplicate tick | colour transition on the member surface | `FAST`, `KEEP_WASH` / `SELECT_WASH` |
| Scan progress bar filling | `transition-all` on the indicator | `SLOW` |
| Delete dialog scrim | `fading_in()` on the scrim | `FadeIn`, 300 ms |
| Delete dialog panel | nsh's modal slide, panel-only (the scrim is a page-layer sibling) | platform `Slide` |
| Toast entering and leaving | all-property transition with starting/ending classes | `SLOW`, `translate-y-3` + opacity |

Two things deliberately do not move: the scrim (a full-window layer must not
travel) and the results list on a rebuild (it keeps the reader's scroll place,
so only its opacity changes). Everything else that swaps content fades or rises
once, instead of blinking.

## Known gaps

- The duplicate-group list is built from the current page of groups (up to
  `GROUP_PAGE`) rather than `virtual_list`. Each group is a variable-height card
  with its own media previews, so virtualization needs per-cell measurement of
  nested content first. Feeds and any tabular view must use `virtual_list` from
  the start.
- Desktop only: no mobile or web target is shipped, so touch targets, sheets and
  the keyboard inset are unexercised.
