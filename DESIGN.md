# Design system

The GUI is a Native Shell (nsh) Rust app. `nsh-base` supplies behavior without a
look; everything visual is the app's own, and it lives in two places:

- `src/ui/palette.rs` — the only file with raw colour literals, and the scales.
  Semantic colour roles, each with a light and a dark value; `text`, `radius`,
  `space` and `size` scales.
- `src/ui/widgets.rs` — the app's `ui` layer: the only module that imports
  `nsh_base` controls or writes a colour, radius or font size. Feature files
  (`root`, `shell`, `results`, `settings`) compose these and layout primitives,
  passing meanings (`BadgeTone`, `Decision`, `Ink`, enabled/active flags), never
  looks:
  - text roles: `heading`, `section_title`, `panel_title`, `name`, `body`,
    `amount`, `figure`, `caption`, `description`, `field_label`, `wordmark`;
  - buttons: `primary_button`, `secondary_button`, `icon_secondary_button`,
    `ghost_button`, `danger_button`, `quiet_button`, `icon_button`,
    `icon_ghost_button`, `rail_button`, `row_button`, `row_icon_button`;
  - choices and inputs: `segmented`, `icon_segmented`, `radio_segmented`,
    `option_select` + `dropdown_trigger`, `number_field`, `field`,
    `toggle_switch`, `tick`;
  - surfaces and regions: `ground`, `side_pane`, `status_strip`, `card`,
    `notice`, `error_notice`, `tile`, `list_row`, `disclosure`, `alert_dialog`,
    `action_bar` + `action_group`, `divider`;
  - media and status: `preview_frame`, `preview_image`, `preview_glyph`,
    `press_to_play`, `video_player`, `progress_bar`, `badge`, `glyph`, `logo`,
    `hint`, `help_dot`.

## The style: an achromatic ledger

The chrome holds no colour. Navigation, buttons, selection and switches are ink
on a stone ground; the one hue goes on the figure each screen answers with —
the space a delete gives back — and on the keyboard's focus ring. The only other
colours carry the decisions the app exists for: green on the copy a group
keeps, amber on a copy ticked to go, and red on destruction.

Each screen answers one question with its largest element:

| Screen | Question | Answered by |
| --- | --- | --- |
| New scan | What will this scan look at, and how will it decide? | The setup form: folders, what to find, keep rule, similarity, where deletes go, video support; **Scan** is the one primary action in the title row |
| Review duplicates | How much space can I get back, and which copies go? | The **Reclaimable** figure (30 pt, the one hue) beside the title, then one card per group with the keeper and the ticked copies marked |
| Cleanup findings | What did the cleanup scan find, and how much space does it hold? | The same title row and figure; the findings as one ruled list |
| Settings | What does each default do? | Two columns: each section's title and purpose beside a panel of rows that say what they change |

## Tokens

| Role | Light | Dark | Used for |
| --- | --- | --- | --- |
| `BACKGROUND` / `SIDEBAR` | `#f4f3f1` | `#0b0c0c` | the window ground; the rail, folder pane and settings nav sit on it |
| `CARD` | `#ffffff` | `#151718` | cards, fields, dialogs, menus, toasts |
| `PANE` | `#eeece8` | `#1f2123` | member tiles, secondary buttons, a hovered row |
| `ELEVATED` | `#e4e1dc` | `#2a2c2e` | the hover above a pane; the active nav pill |
| `INSET` | `#e9e7e3` | `#0f1011` | the segmented groove, the progress rail, empty preview boxes |
| `BORDER` | `#e3e0db` | `#272a2c` | card edges, the sidebar and status bar rules |
| `HAIRLINE` | black 6% | white 6% | rules inside a card |
| `CONTROL_BORDER` | `#d3cfc9` | `#3a3d40` | fields, select triggers, the off switch |
| `TEXT` / `PRIMARY` | `#1a1a18` | `#ededee` | ink; the primary button and the on switch |
| `ON_PRIMARY` | white | `#111213` | the label on `PRIMARY` |
| `MUTED` | `#65625d` | `#9b9ea0` | secondary copy (5.5:1 on the ground) |
| `ACCENT` | `#1d5fbf` | `#74b4ff` | the answer figure, the focus ring, the caret |
| `KEEP` / `KEEP_WASH` / `KEEP_INK` | `#1d8a47` / `#dcf3e3` / `#0f5c2c` | `#32d74b` / `#142e1b` / `#8bf09f` | a keeper's edge, its badge, the badge's words |
| `SELECT` / `SELECT_WASH` / `SELECT_INK` | `#c27500` / `#fdeed5` / `#7a4800` | `#ff9f0a` / `#33230a` / `#ffc870` | a ticked copy's edge, its badge, the badge's words |
| `DANGER` / `DANGER_SURFACE` / `ON_DANGER` / `DANGER_TEXT` | `#e03a2f` / `#c4221d` / white / `#b31f1a` | `#ff453a` / `#c4221d` / white / `#ff6b5e` | error edges, the Delete fill and its label, error copy |
| `SCRIM` | black 22% | black 59% | behind the delete dialog |

A wash is a backing, never a label colour: words on `KEEP_WASH` take
`KEEP_INK`, words on `SELECT_WASH` take `SELECT_INK`. A member tile itself is
neutral (`PANE`); only its edge and its badge carry the hue, so a group of five
reads as five tiles with two marks rather than five coloured blocks.

Scales are finite:

- `text::{CAPTION, BODY, TITLE, HEADING, FIGURE}` = 12, 14, 16, 24, 30 pt. One
  page title per screen (`heading`, read as a header); `figure` is reserved for
  the answer figure and uses tabular digits. Text is set in points and the
  engine applies the user's text size to all of it.
- `radius::{CARD, CONTROL, INSET}` = 8, 6, 4. Cards and floating surfaces take
  8, controls 6, and anything inset 2–4 pt inside a card or a control (a
  segment in its groove, a group header's hover, a preview in its tile) takes 4,
  so nested curves stay concentric. `rounded_full` is for the switch alone.
- `space::{XXS..XXL}` = 2, 4, 8, 12, 16, 24, 32.
- `size::{CONTROL, COMPACT}` = 32, 26 pt. Every button, field, select and
  segmented track in a row is `CONTROL` tall, so toolbars line up by
  construction; a member's tick and row actions are `COMPACT`.

## Surfaces

Flat, one line each. A `card` is `CARD` with a `BORDER` edge at radius 8 — no
tray, no inner bezel, no shadow. Rules inside a card are `HAIRLINE` (one device
pixel) and inset to the card's text column. Only what floats casts a shadow:
the delete dialog, select popups and toasts (`shadow_md`).

Segmented controls are one silhouette: `segment_track` is an `INSET` groove a
control tall; the chosen segment is the `CARD` surface with a `BORDER` edge and
ink label, the others sit in the groove in `MUTED`. The keep rule, the sort,
the layout and every settings choice use it. Eleven scan types do not fit one,
so "What to find" is an `option_select`.

## Layout

- Shell: a 56 pt icon rail and a 264 pt folder pane on the ground, separated
  from the work area by one `BORDER` rule. The pane opens with the product's
  mark and name; each folder row is the folder's name over its full path, cut
  from the front so the distinguishing end stays.
- Work area: 32 pt padding. Every screen opens with the same title row — the
  `heading`, one `description` line under it, the screen's action or figure at
  the far edge — so starting a scan changes the content under a header that
  stays put.
- New scan: a single card of labelled rows (148 pt label column, controls on
  one line down the card), every rule's explanation on the page rather than in
  a tooltip. The keep, similarity, delete and video rows appear only for a
  duplicate scan; the whole form fits the default 1120×640 window.
- Results: the title row with the Reclaimable figure; an action toolbar (Back,
  Delete, Clear selection, Undo | consolidate, export, expand, collapse); an
  options toolbar (similarity, keep rule | sort, layout); then one card per
  group.
- Settings: a 200 pt nav, then sections split by full-width rules: a 240 pt
  label column (title and purpose) beside a panel whose rows show their own
  descriptions.

## Interaction states

`nsh-base` draws no pressed, focused or disabled appearance, so every control
carries all three:

| State | Look | Where |
| --- | --- | --- |
| hover | one surface step up (`PANE` → `ELEVATED`, ground → `PANE`), or 86–90% opacity on a filled button | `.hover(..)` on each control |
| pressed | 90% opacity and scale 0.97 (buttons, segments) or `ELEVATED` (rail) | `.active(..)` |
| keyboard focus | 2 pt `ACCENT` outline, only after the keyboard moved focus | `.focus_visible(focus_ring)` |
| disabled | 45% opacity | `.disabled_style(..)` |
| selected | the lifted `CARD` segment, or the `ELEVATED` nav pill — a place, not a colour | `selection(..)` |

Small icon targets (26 pt) also take `.hit_slop(EdgeInsets::all(px(4.0)))`.
Selection is not a state style: a chosen segment or nav item is a static style
computed from the owner's value, so it survives rebuilds.

## Motion inventory

Durations are tokens in `widgets::timing`: `FAST` (150 ms) for a control
answering the pointer, `BASE` (180 ms) for a look change, `SLOW` (240 ms) for a
surface arriving. Every one goes through `widgets::motion(ms)`, which reads the
platform's reduce-motion preference reactively and returns 0 when it is on.

Arrivals use nsh's motion presets through two helpers: `widgets::entering()`
(fade while rising 24 pt, 450 ms) for a whole page, and `widgets::fading_in()`
(fade only, 300 ms) for anything that must not move under the reader. Both are
created where the pane is built, so a pane that mounts again plays again, and
nsh lands them at their end state when reduce-motion is on.

| Change | Tool | Tokens |
| --- | --- | --- |
| Page switch (workspace ⇄ Settings) | `entering()` bound to the page | `FadeInUp`, 450 ms |
| New scan page appearing | `entering()` | `FadeInUp`, 450 ms |
| Results list rebuilt (scan landing, filter, delete, Show more) | `fading_in()` on the list | `FadeIn`, 300 ms |
| Button hover | colour or opacity transition, ease-out | `BASE` |
| Button press dim and sink | state style + scale 0.97, transitioned | `FAST`, opacity 0.9 |
| Segment picked (keep rule, sort, layout, settings) | colour transition on each segment | `FAST` |
| Rail slot and settings nav moving | background transition / tab indicator | `BASE` / `FAST` |
| Group disclosure chevron opening | transform rotation 0° → 90° | `FAST`, one glyph turned rather than two swapped |
| Group body opening and closing | `collapsible().reveal(true)` | nsh spring |
| Toggle switch thumb | `transition-transform` class on the thumb | `FAST` |
| Text-field focus edge | colour transition | `BASE`, `CONTROL_BORDER` → `ACCENT` |
| Keeper pick and duplicate tick | colour transition on the member's edge and badge | `FAST` |
| Scan progress bar filling | `transition-all` on the indicator | `SLOW` |
| Delete dialog scrim | `fading_in()` on the scrim | `FadeIn`, 300 ms |
| Delete dialog panel | nsh's modal slide, panel-only (the scrim is a page-layer sibling) | platform `Slide` |
| Toast entering and leaving | all-property transition with starting/ending classes | `SLOW`, `translate-y-3` + opacity |

Three things deliberately do not move: the scrim (a full-window layer must not
travel), the results list on a rebuild (it keeps the reader's scroll place, so
only its opacity changes), and the answer figure and the Delete count, which
change on every tick of a selection — a fade there would flicker under the
pointer.

## Known gaps

- The duplicate-group list and the cleanup findings list are built from one page
  of results (up to `GROUP_PAGE`, with a "Show more" button) rather than
  `virtual_list`. Each group is a variable-height card with its own media
  previews, so virtualization needs per-cell measurement of nested content
  first. Feeds and any tabular view must use `virtual_list` from the start.
- Desktop only: no mobile or web target is shipped, so touch targets, sheets and
  the keyboard inset are unexercised.
