# Design system

The GUI is a Native Shell (nsh) Rust app. `nsh-base` supplies behavior without a
look; everything visual is the app's own, and it lives in two places:

- `src/ui/palette.rs` — the only file with raw colour literals, and the scales.
  Semantic colour roles, each with a light and a dark value; `text`, `faces`,
  `radius`, `space` and `size` scales.
- `src/ui/widgets.rs` — the app's `ui` layer: the only module that imports
  `nsh_base` controls or writes a colour, radius or font size. Feature files
  (`root`, `shell`, `results`, `settings`) compose these and layout primitives,
  passing meanings (`BadgeTone`, `Decision`, `Ink`, enabled/active flags), never
  looks:
  - text roles: `heading`, `section_title`, `panel_title`, `name`, `body`,
    `amount`, `figure`, `caption`, `description`, `field_label`, `wordmark`,
    and `title_block` (a running head, a title and its line);
  - buttons: `primary_button`, `secondary_button`, `icon_secondary_button`,
    `ghost_button`, `danger_button`, `quiet_button`, `icon_button`,
    `icon_ghost_button`, `rail_button`, `row_button`, `row_icon_button`;
  - choices and inputs: `segmented`, `icon_segmented`, `radio_segmented`,
    `option_select` + `dropdown_trigger`, `number_field`, `field`,
    `toggle_switch`, `tick`;
  - surfaces and regions: `ground`, `side_pane`, `status_strip`, `card`,
    `notice`, `error_notice`, `tile`, `list_row`, `disclosure`, `alert_dialog`,
    `action_bar` + `action_group`, `divider`;
  - media and status: `preview_frame`, `preview_image`, `clip_glyph`,
    `kind_preview`, `press_to_play`, `video_player`, `progress_bar`, `badge`
    (`BadgeTone::Kind(Kind)` for a dyed kind tag), `keeper_badge`, `glyph`,
    `logo`, `hint`;
  - the brush marks: `seal` + `pressed_seal`, `turning_enso`, `rail_marker`
    + `move_rail_marker`, `ink_lift`, and `register_faces`, which hands the
    bundled face to the text system at start-up;
  - motion helpers: `entering`, `fading_in`, `arriving`, `count_up`.

## The style: washi and sumi

The window is a sheet of unbleached paper. The ground is kinari (`#f2ece0`)
with real kozo-fibre grain — a procedurally made alpha texture
(`assets/textures/washi-*.png`: tileable pulp clouds, long tapered fibres,
fine short ones, specks) laid once behind everything, brown fibres on light
paper and pale ones on the inkstone black of dark mode. Panes on the ground
(the rail, the folder pane, the settings nav, the status line) are
transparent, so one paper runs under the whole window; a card is a lighter
sheet laid on it and covers the grain.

Ink does the work. Text, the primary action and the switches are sumi; the one
hue is shu vermilion (`#c83e2b`), the red of a hanko seal, and it goes only on
the figure each screen answers with, the seal, the running head's stroke, the
keyboard's focus ring and destruction. The decisions take earth colours:
matcha on the copy a group keeps, kuchiba ochre on a copy ticked to go.

What a thing *is* takes a traditional dye, never what it *does*: each file
kind and each cleanup finding kind wears a pale wash with its words in the
dye's deep ink (`colors::Dye`). Ai indigo for documents and the largest
files, fuji wisteria for video and damaged or mislabelled files, yamabuki
gold for audio, sabi-asagi teal for archives and temp leftovers, sakura for
photos and their metadata, nezumi grey for anything else and for what holds
nothing. The dyes live on a group's kind tag, a finding's tag and the slip
that stands in for a file with no picture; the chrome stays sumi, and no
dye goes near an action, a state or the answer.

Restraint and ma: no gradients, no shadows on anything that does not float,
one soft edge per sheet, paper corners cut nearly square. The work area keeps
48 pt of empty margin either side (`space::MA`); the empty space is part of
the page.

One type voice: Inter (bundled, Light/Regular/Medium/SemiBold) for every word.
The app prints no CJK text; the Japanese world lives in the material and the
brush marks, so every string stays translatable:

- every page's title row opens with a running head: one vermilion brush
  stroke in the margin beside the title, drawn down as the page arrives;
- the active page's slot on the rail carries the same stroke at the rail's
  edge;
- a running scan shows an ensō (`assets/enso.path`: one tapered brush stroke
  round an open circle) turning slowly in vermilion beside its progress. It
  only ever appears turning, so an open circle always means work in
  progress;
- a clean-up that went through ends in a settled seal: a round vermilion
  disc with a rim ring and a tick left in the paper's colour, turned −6° as a
  hand-pressed seal is. Round and ringed, so it never reads as a checkbox.

The marks are decorative and hidden from screen readers; the words beside them
carry every meaning. Inter is SIL OFL 1.1 (`assets/fonts/OFL.txt`).

Each screen answers one question with its largest element:

| Screen | Question | Answered by |
| --- | --- | --- |
| New scan | What will this scan look at, and how will it decide? | The setup form: folders, what to find, keep rule, similarity, where deletes go, video support; **Scan** is the one primary action in the title row |
| Review duplicates | How much space can I get back, and which copies go? | The **Reclaimable** figure (36 pt Light, vermilion) beside the title — the ticked copies, the same set Delete counts, with "of N found" under it while the two differ — then one card per group with the keeper (a matcha "Keeping" badge) and the ticked copies (an ochre tick reading "Delete", an unticked one "Keep too") marked. Once a delete clears every group the figure reads **Freed** for a permanent delete, **In the trash** for a trashed one (with the way to the space back under it), and a card says where the copies went, with the settled seal pressed beside it |
| Cleanup findings | What did the cleanup scan find, and how much space does it hold? | The same title row and figure; one toolbar (Delete N, or Remove metadata for photo metadata; Export report…); the findings as one ruled list |
| Settings | What does each default do? | Two columns: each section's title and purpose beside a panel of rows that say what they change |

## Tokens

| Role | Light | Dark | Used for |
| --- | --- | --- | --- |
| `BACKGROUND` / `SIDEBAR` | `#f2ece0` | `#161513` | the paper ground under the grain; the rail, folder pane and settings nav are transparent on it |
| `CARD` | `#fbf8f1` | `#1e1c19` | cards, fields, dialogs, menus, toasts |
| `PANE` | `#ece5d7` | `#272421` | member tiles, secondary buttons, a hovered row |
| `ELEVATED` | `#e3daca` | `#312d29` | the hover above a pane; the active nav pill |
| `INSET` | `#e8e1d3` | `#121110` | the segmented groove, the progress rail, empty preview boxes |
| `BORDER` | `#e0d7c7` | `#2e2a26` | card edges, the sidebar and status bar rules |
| `HAIRLINE` | sumi 7% | paper 6% | rules inside a card |
| `CONTROL_BORDER` | `#cfc4b1` | `#423c36` | fields, select triggers, the off switch |
| `TEXT` / `PRIMARY` | `#1f1c18` | `#ede6d8` | sumi ink; the primary button and the on switch |
| `ON_PRIMARY` | `#fbf8f1` | `#161513` | the label on `PRIMARY` |
| `MUTED` | `#6a6257` | `#a69d8f` | diluted ink: secondary copy (5.2:1 on the ground) |
| `ACCENT` / `ON_ACCENT` | `#c83e2b` / `#fbf8f1` | `#e5654f` / `#161513` | the answer figure, the seal and its tick, the running head's stroke, the ensō, the focus ring |
| `KEEP` / `KEEP_WASH` / `KEEP_INK` | `#3c6e47` / `#e0eadb` / `#24502f` | `#8cc49a` / `#1d2a20` / `#b5e0be` | a keeper's edge, its badge, the badge's words |
| `SELECT` / `SELECT_WASH` / `SELECT_INK` | `#a86a12` / `#f4e5c9` / `#6e4508` | `#e3a54e` / `#33270f` / `#f2cc8a` | a ticked copy's edge, its badge, the badge's words |
| `DANGER` / `DANGER_SURFACE` / `ON_DANGER` / `DANGER_TEXT` | `ACCENT` / `#b8361f` / `#fbf8f1` / `#a8301e` | `ACCENT` / `#b8361f` / `#fbf8f1` / `#f08470` | error edges, the Delete fill and its label, error copy |
| `SCRIM` | sumi 25% | black 59% | behind the delete dialog |
| `AI` (wash / ink) | `#dfe5ee` / `#22385e` | `#1a2130` / `#aebfdc` | documents, the largest files (9.2:1 / 8.7:1) |
| `FUJI` | `#e8e1ee` / `#4f3a6b` | `#251f2d` / `#cdb9e3` | video; damaged or mislabelled files |
| `YAMABUKI` | `#f5ebc2` / `#5f4b00` | `#2c260f` / `#ecd486` | audio |
| `ASAGI` | `#d9e9e6` / `#1f5450` | `#16262a` / `#9fd3cc` | archives; temp leftovers |
| `SAKURA` | `#f3e0e2` / `#7a3446` | `#2e1c20` / `#ebb3be` | photos; photo metadata |
| `NEZUMI` | `#e6e1d8` / `#4f4a43` | `#24221f` / `#c2baae` | any other file; empty files, folders and dead links |

A wash is a backing, never a label colour: words on `KEEP_WASH` take
`KEEP_INK`, words on `SELECT_WASH` take `SELECT_INK`. A member tile itself is
neutral (`PANE`); only its edge and its badge carry the hue, so a group of five
reads as five tiles with two marks rather than five coloured blocks.

Scales are finite:

- `text::{CAPTION, BODY, TITLE, HEADING, FIGURE}` = 12, 14, 16, 26, 36 pt.
  One page title per screen (`heading`, Inter Medium, −0.5 pt tracking, read
  as a header); `figure` is reserved for the answer figure (Inter Light,
  tabular digits). Text is set in points and the engine applies the user's
  text size to all of it. `faces::SANS` names the family; the ground sets it
  for everything under it.
- `radius::{CARD, CONTROL, INSET}` = 6, 4, 2: paper is cut nearly square.
  Cards and floating surfaces take 6, controls 4, and anything inset inside a
  card or a control (a segment in its groove, a group header's hover, a
  preview in its tile) takes 2, so nested curves stay concentric.
  `rounded_full` is for the switch and the seal alone.
- `space::{XXS..XXL}` = 2, 4, 8, 12, 16, 24, 32, and `MA` = 48, the margin
  either side of the work area and a settings page.
- `size::{CONTROL, COMPACT}` = 32, 26 pt. Every button, field, select and
  segmented track in a row is `CONTROL` tall, so toolbars line up by
  construction; a member's tick and row actions are `COMPACT`.

## Surfaces

Flat paper, one line each. A `card` is `CARD` with a `BORDER` edge at radius 6
— no tray, no inner bezel, no shadow. Rules inside a card are `HAIRLINE` (one
device pixel) and inset to the card's text column. Only what floats casts a
shadow: the delete dialog, select popups and toasts (`shadow_md`). The grain
lives on the ground alone (`widgets::ground`, an absolute layer that takes no
pointer and is hidden from screen readers); nothing else is textured.

Segmented controls are one silhouette: `segment_track` is an `INSET` groove a
control tall; the chosen segment is the `CARD` surface with a `BORDER` edge and
ink label, the others sit in the groove in `MUTED`. The keep rule, the sort,
the layout and every settings choice use it. Eleven scan types do not fit one,
so "What to find" is an `option_select`.

## Layout

- Shell: a 56 pt icon rail (Home, Settings, and the theme switch
  at its foot) and a 264 pt folder pane on the ground, separated from the work
  area by one `BORDER` rule. The pane opens with the product's mark and name;
  each folder row is the folder's name over its full path, cut from the front
  so the distinguishing end stays.
- Work area: 48 pt (`MA`) either side, 32 pt top and bottom. Every screen
  opens with the same title row — `title_block`: the running head, the
  `heading` and one `description` line under it; the screen's action or
  figure at the far edge — so starting a scan changes the content under a
  header that stays put.
- New scan: a single card of labelled rows (148 pt label column, controls on
  one line down the card), every rule's explanation on the page rather than in
  a tooltip. The keep, similarity, delete and video rows appear only for a
  duplicate scan. At the default 1120×640 window the form scrolls by its last
  row: the margins keep their ma rather than the form squeezing to fit.
- Results: the title row with the Reclaimable figure; an action toolbar (Back,
  Delete / Move / Link, Clear selection, Restore from trash | Consolidate,
  Export report…, expand, collapse); a list toolbar (sort, layout); then one card per group. The keep
  rule and the similarity floor belong to the scan and live on New scan only:
  on this page they would re-pick every keeper and drop the user's ticks. With
  no group left only Back stays, and the list shows the cleared card (what
  went, where to, Scan again, Restore from trash where the OS can restore) or
  "No duplicates found" for a scan that found nothing.
- What happens to the ticked copies is chosen in the confirmation, where the
  plan is on screen: a `radio_segmented` of **Delete** (trash or permanent, per
  the setting), **Move to folder** (the CLI's `--move-dir`) and **Hard link**
  (`--hardlink`). The choice is remembered, and the toolbar button names it
  (`Delete 5`, `Move 5`, `Link 5`), so the button and its dialog always make
  the same promise. Move asks for its folder inside the dialog, as
  Consolidate does; Hard link marks the groups it cannot link (similar, not
  identical) in the plan instead of hiding them.
- Cleanup findings: the title row, then one action bar — the act on the
  findings (Delete N per the trash setting; Remove metadata for Photo
  metadata; nothing for Duplicate folders, which stay report-only like the
  CLI) and Export report…. The act opens an `alert_dialog` that lists the
  first items, counts the rest, and names what a protected folder keeps.
- Export report… opens the system save dialog with JSON, CSV and HTML types;
  the chosen extension picks the format, the same writers as `--json`,
  `--csv` and `--html`.
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
| Results list rebuilt (filter, delete, Show more) | `fading_in()` on the list | `FadeIn`, 300 ms |
| A scan landing: its group cards | `arriving(order)` on each card instead of the list fade: fade + settle 12 pt, each a beat after the one above, the eighth and later together; once per scan (`scan_gen`), never on a page coming back | 320 ms out-expo, 40 ms apart, ≤ 280 ms delay |
| A scan landing: the Reclaimable figure | `count_up()`: the figure counts from 0 to its value frame by frame, once per scan; every tick after that moves it at once | 700 ms out-expo |
| Button hover | colour or opacity transition, ease-out | `BASE` |
| Button press dim and sink | state style + scale 0.97, transitioned | `FAST`, opacity 0.9 |
| Segment picked (keep rule, sort, layout, settings) | colour transition on each segment | `FAST` |
| Rail slot and settings nav moving | background transition / tab indicator; the rail's vermilion stroke (`rail_marker`, the running head's) is drawn down when the rail first shows, then travels to the new slot on a page change, drawn out long mid-way like a dragged brush (`TranslateY` + `ScaleY` 1→1.8→1) | `BASE` / `FAST`; stroke 360 ms out-expo |
| Running head drawn (every title row as it mounts) | `ScaleY` 0→1, `TranslateY` −8→0, fade | 360 ms, out-expo |
| Scan running | the ensō turns, linear and endless, on a part of the card the progress ticks never rebuild | 2600 ms a turn |
| Scan phase that cannot count (listing folders, finishing) | a stroke of sumi a quarter of the groove long (40–220 pt) is carried across it, again and again, in-out ease; the bar rebuilds only when its fill moves, so the sweep never restarts on a tick. Reduce-motion: a faint full line | 1500 ms a pass |
| A folder added to the folder pane | `arriving(n)` on the new row only (the restored list at start-up and the rows already there stay still) | 320 ms out-expo |
| Group disclosure chevron opening | transform rotation 0° → 90° | `FAST`, one glyph turned rather than two swapped |
| Group body opening and closing | `collapsible().reveal(true)` | nsh spring |
| Toggle switch thumb | `transition-transform` class on the thumb | `FAST` |
| Text-field focus edge | colour transition | `BASE`, `CONTROL_BORDER` → `ACCENT` |
| Keeper pick and duplicate tick | colour transition on the member's edge; the new decision's wash (matcha "Keeping", ochre "Delete") soaks in from the tag's leading edge (`ScaleX` 0→1 on a layer under the words, origin left), only when that copy's decision changed | `FAST` edge; `BASE` soak, out-expo |
| Scan progress bar filling | `transition-all` on the indicator | `SLOW` |
| Delete and Consolidate dialog scrim | `fading_in()` on the scrim | `FadeIn`, 300 ms |
| Delete and Consolidate dialog panel | nsh's modal slide, panel-only (the scrim is a page-layer sibling) | platform `Slide` |
| Delete dialog: Delete / Move to folder / Hard link picked | segment colour transition; the title, summary, plan notes and commit label swap in place and the panel re-centres at its new height, without animation (the reader's eye stays on the segment just pressed) | `FAST` |
| Delete dialog: move folder chosen | the folder line's text swaps; the Move commit appears beside Cancel | none — an instant swap of a value, like a field |
| Findings action bar arriving with the findings | part of the page's `entering()` | `FadeInUp`, 450 ms |
| Findings confirmation scrim and panel | the same `alert_dialog` as Delete | `FadeIn` 300 ms / platform `Slide` |
| Findings removed (delete or metadata) | the list rebuilds under `fading_in()`; a toast reports the outcome | `FadeIn`, 300 ms; toast `SLOW` |
| Toast entering and leaving | all-property transition with starting/ending classes | `SLOW`, `translate-y-3` + opacity |
| A clean-up settling (the last group or finding goes) | `ink_lift()`: a WGSL `shader_transition` (`assets/shaders/ink_lift.wgsl`) around the list. The old cards let go of their ink along fibre noise (dense ink first, drifting a point or two along the fibre), the ground's real grain shows through, and the outcome card is inked in behind through the same fibres. Only the moment of settling runs it; a restore or a new scan does not. No GPU: nsh's cross-fade | 900 ms |
| Cleared card's seal | `pressed_seal()`: the seal comes down larger and more turned (`Scale` 1.45→0.94→1, `Rotate` −9°→1°→0 on a wrapper, so the seal keeps its −6°) and settles as the lift finishes | 560 ms, 720 ms delay |

With reduce-motion on, every line above lands at its end state: the
transitions take 0 ms through `motion(ms)` and the presets land without
moving.

Three things deliberately do not move: the scrim (a full-window layer must not
travel), the results list on an ordinary rebuild (it keeps the reader's scroll
place, so only its opacity changes), and the answer figure and the Delete
count after a scan has landed, which change on every tick of a selection — a
fade there would flicker under the pointer.

## Known gaps

- The duplicate-group list and the cleanup findings list are built from one page
  of results (up to `GROUP_PAGE`, with a "Show more" button) rather than
  `virtual_list`. Each group is a variable-height card with its own media
  previews, so virtualization needs per-cell measurement of nested content
  first. Feeds and any tabular view must use `virtual_list` from the start.
- Desktop only: no mobile or web target is shipped, so touch targets, sheets and
  the keyboard inset are unexercised.
