# Design system

The GUI is a Native Shell (nsh) Rust app. `nsh-base` supplies behavior without a
look; everything visual is the app's own, and it lives in two places:

- `src/ui/palette.rs` — the only file with raw colour literals. Semantic roles,
  each with a light and a dark value: `BACKGROUND`, `SIDEBAR`, `CARD`, `PANE`,
  `ELEVATED`, `INSET`, `SHELL`, `BORDER`, `HAIRLINE`, `TEXT`, `MUTED`, `ACCENT`,
  `ON_ACCENT`, `ACCENT_WASH`, `ACCENT_WASH_STRONG`, `KEEP`, `KEEP_WASH`,
  `KEEP_WASH_STRONG`, `SELECT`, `SELECT_WASH`, `SELECT_WASH_STRONG`, `DANGER`,
  `SCRIM`. Depth comes from the surface ramp, the bezel and one-pixel
  hairlines, never from shadows.
- `src/ui/widgets.rs` — the styled controls every pane composes: `card`,
  `bezel`/`bezel_core`, `heading`/`page_heading`/`panel_title`/`caption`/
  `field_label`, `primary_button`, `secondary_button`, `ghost_button`,
  `danger_button`, `icon_button`, `icon_pill_button`, `icon_secondary_button`,
  `icon_ghost_button`, `icon_chip`, `toggle_switch`, `chip`, `divider`, `field`,
  `hint`, `help_dot`, `icon`.

Scales are finite tokens, not per-call values: `radius::SHELL` (22), `radius::CARD`
(18) and `radius::CONTROL` (12); `space::{XXS..XXXL}` = 2, 4, 8, 12, 16, 24, 32,
44; `text::{MICRO, CAPTION, BODY, HEADING, PAGE}` = 11, 12, 14, 22, 30 pt. Text
is set in points and the engine applies the user's text scale to all of it, so
the user's text-size setting is honoured without a second scale in the app.
Labeled pills share one height, `widgets::PILL_HEIGHT` (35 pt): an icon control
that lands in a row of pills takes `icon_pill_button` so the row lines up.

## Surfaces, palette and hairlines

The neutral surfaces are warm in both appearances: a paper with a yellow-brown
cast in the light one and a deep warm charcoal in the dark one, with the app's
blue as the accent and a light step dark enough to carry white label text.
Nothing is a neutral grey — the tint is what keeps the app calm rather than
clinical.

Depth is a ramp plus a hairline, in that order. The window itself steps up to
the rail and then to a card, and in both appearances that direction is lighter:
`BACKGROUND` paper → `SIDEBAR` → `CARD` white, and `#0a0b0b` → `#101112` →
`#17191a` in the dark one. Anything that sits *inside* a card moves the other way
from the reader: `PANE` is a deeper step than `CARD` (darker in light, lighter
in dark, because in a dark ramp depth reads as lift), and `ELEVATED` is one more
step again for the hover. `INSET` is the exception that proves the rule: wells,
fields and the discs of circular icon buttons are punched into whatever holds
them, so they are the darkest step in light and the darkest step in dark.

| Role | Light | Dark | Used for |
| --- | --- | --- | --- |
| `BACKGROUND` | `#f5f3ef` | `#0a0b0b` | the window behind everything |
| `SIDEBAR` | `#fbf9f5` | `#101112` | the rail and folder pane |
| `CARD` | `#ffffff` | `#17191a` | cards, bars, dialogs, a bezel's core |
| `PANE` | `#f2efe9` | `#1f2123` | rows on a card, secondary buttons |
| `ELEVATED` | `#e9e5dd` | `#292b2d` | the hover step above a pane |
| `INSET` | `#eeebe5` | `#0d0e0f` | wells, fields, circular icon buttons |
| `SHELL` | black 8% | white 10% | a bezel's outer tray |
| `BORDER` | `#e4dfd6` | `#2b2e30` | rules, media frames, raised-control outlines |
| `HAIRLINE` | black 12% | white 14% | container and control seams |
| `ACCENT` | `#0b63d8` | `#74b4ff` | primary actions, focus, links |
| `ACCENT_INK` | `#0a4696` | `#cfe6ff` | text and glyphs on an accent tint |
| `KEEP` | `#1d9a4a` | `#32d74b` | the copy a group keeps |
| `SELECT` | `#c27500` | `#ff9f0a` | a copy ticked for deletion |
| `DANGER` | `#ff3b30` | `#ff453a` | Delete, and errors |

An accent tint is a backing, not a label colour: `ACCENT` on
`ACCENT_WASH_STRONG` measures 3.2:1 in light and 3.5:1 in dark, so anything
written on that tint takes `ACCENT_INK` instead (5.2:1 on the light tint,
5.9:1 on the dark one). `ACCENT` itself is only used on a plain surface,
or as a fill under `ON_ACCENT`.

Hue is meaning, not decoration. Blue is reserved for actions and focus; the
keeper of a group is green so "this one is kept" can never be mistaken for
"this one is ticked", and the tick itself is a separate warm amber. Washes are
the same hue at low alpha (`*_WASH` for a resting tint, `*_WASH_STRONG` for a
selected chip or a filled state) so a surface can be tinted without inventing a
new colour.

### Two hairlines, two jobs

- `HAIRLINE` is a translucent veil — black at 12% in light, white at 14% in
dark — so it reads against whatever is behind it. It draws seams: the edge of a
`card`, both edges of a `bezel` tray, the seam between a tray and its core, and
the edge of a raised control such as `icon_button`. Use it where two surfaces
meet and the join should be legible but quiet.
- `BORDER` is an opaque line, one step stronger. It draws things that are lines
  in their own right: `divider` rules (a single device pixel as a `bg`, so they
  stay thin on a 2x display), media and preview frames, and the outline of a
  raised control such as `secondary_button`. A text field is not a line: it is
  an `INSET` well whose seam is a `HAIRLINE`.

The rule of thumb: if the line exists to separate two surfaces, it is a
hairline; if it is the outline of an object, it is `BORDER`.

### The bezel

A bezelled surface is two elements, never one with a thick border:

1. `bezel()` — a tray with a 3 pt inset, filled with `SHELL` (a veil of the
opposite appearance over the background, so it darkens the light theme and
lights the dark one), a `HAIRLINE` seam, and `radius::SHELL` (22).
2. `bezel_core()` — the `CARD` surface nested inside, with its own `HAIRLINE`
   and a **concentric** radius: `radius::SHELL - 3` = 19, so the two curves stay
   parallel instead of drifting apart at the corners.

Both take `BorderCurve::Continuous`, the squircle curve, which is what makes the
nesting look machined rather than drawn. A container that does not need the
tray uses `card()` alone: the same `CARD` fill and `HAIRLINE`, at
`radius::CARD`.

## Interaction states

`nsh-base` draws no pressed, focused or disabled appearance, so every control
carries all three:

| State | Look | Where |
| --- | --- | --- |
| hover | one surface step up (`ELEVATED`, `PANE`) or a 8% opacity wash | `.hover(..)` on each control |
| pressed | 90% opacity (buttons, icon chips) or `ELEVATED` (rail, folder rows) | `.active(..)` |
| keyboard focus | 2pt accent outline, only after the keyboard moved focus | `.focus_visible(focus_ring)` |
| disabled | 45% opacity | `.disabled_style(..)` |

Small icon targets (30 pt and below, and the pill-height variants) also take
`.hit_slop(EdgeInsets::all(px(4.0)))`.
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
