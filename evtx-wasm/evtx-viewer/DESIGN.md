# EVTX Viewer UI

A homage to the classic Windows Event Viewer (tree, grid, preview pane, gray title bands), drawn with Windows 11 metrics and colors, with the density of a log tool like Datadog for the modern parts: column filters, facets, timeline, query bar. It should feel like a desktop utility, not a web page. When in doubt, look at Event Viewer first and keep it plain.

## Type

- `"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif`. 12px everywhere in the grid, trees, facets and menus; 12px semibold for headers and title bands; 11px for secondary text (counts, axis labels). No other sizes except the empty state title (20px semibold).
- `font-variant-numeric: tabular-nums` on anything numeric or temporal (grid, counts, status bar, timeline labels).
- Monospace (`"Cascadia Mono", Consolas, monospace`, 12px) only for raw values in the details pane: EventData values, SIDs, hex, JSON/XML.

## Color (tokens in `src/styles/theme.ts`, light / dark)

- Base surface `#F3F3F3` / `#202020` behind the menu bar, toolbar and status bar. Panes (tree, grid, facets, details) are `#FFFFFF` / `#2B2B2B`, separated by 1px `#E5E5E5` / `#1D1D1D` lines.
- Text `#1A1A1A` / `#FFFFFF`, secondary `#5D5D5D` / `#C5C5C5`, tertiary `#8A8A8A` / `#9A9A9A`.
- Accent `#005FB8` / `#60CDFF` (text on accent: white / black). Hover and pressed are the WinUI steps (`#1A6DBF`, `#3380C7` / `#5AB8E6`, `#52A6CF`).
- Title bands (grid header "Security · 62,031 events", details header "Event 4688, Microsoft-Windows-Security-Auditing"): `#6E6E6E` / `#3A3A3A` background, white text. This is the Event Viewer signature; use it only for those two bands.
- Severity: critical/error `#C42B1C` / `#FF99A4`, warning `#9D5D00` / `#FCE100`, verbose uses tertiary text. Information has no color. Severity color appears only in the level icon, a 3px left edge on warning/error/critical rows, and the timeline stack.
- Follow `prefers-color-scheme` until the user picks System / Light / Dark in the View menu. Respect `forced-colors`.

## Grid

- 22px rows, 24px header. No zebra stripes, no vertical lines in the body; the header has 1px column separators. Numbers (Event ID, Task, Opcode) are right-aligned.
- Hover: `rgba(0,0,0,.04)` / `rgba(255,255,255,.06)`. Selected: accent at 12% / 18% fill; keyboard focus adds a 1px inset accent outline.
- Header filter and sort glyphs appear on hover; a column with an active filter or sort keeps a filled glyph.
- Level column: 16px icon (`*16Filled`) + name. Level 0 reads "Information", as in Event Viewer.
- Default sort is time, newest first. Column widths come from a sample of content on first load; the user's resized widths win.

## Panes and controls

- Docked panes are square. Only floating things are rounded: controls 4px, menus/flyouts/tooltips/dialogs 8px with a 1px stroke and `0 8px 16px rgba(0,0,0,.14)` shadow.
- Controls are 24px in the toolbar and 28px elsewhere. Toolbar buttons are subtle (no border until hover). One accent button per view at most.
- Every menu, flyout, tooltip and filter popover uses the shared popover in `src/components/Windows/` (native `popover`, anchored to its trigger, flips at viewport edges, Esc and outside click close it, focus returns to the trigger). Menu items are 28px with a 4px hover fill, a check column, and shortcuts in tertiary text on the right.
- Flyouts fade and slide 4px in over 120ms; nothing moves with `prefers-reduced-motion`.
- Scrollbars: `scrollbar-width: thin` with theme colors; no custom scrollbar drawing.
- Icons: `@fluentui/react-icons` at their native size (`*16Regular` / `*16Filled`), never scaled from 20px.

## Details pane

- Title band, then General / Details tabs (Details shows the raw JSON). Attributes are a two-column key/value list with 22px rows; long values wrap.
- Filter in, filter out, add column and copy appear next to the value on row hover or focus, and in the row's context menu.

## Facets and timeline

- Facet rows are 22px: checkbox, value, right-aligned count, and a faint bar behind the row sized to its share of the facet total.
- The timeline is a short stacked histogram (information neutral, warning, error) with a few axis ticks. Drag selects a range; typed ranges live in a "Custom range…" flyout. A selected interval includes its start and excludes its end.

## Time

- One setting, View > Time zone (Local / UTC, default Local), drives the grid, details, facets, chips and timeline. Format with the browser locale plus milliseconds, and name the zone in the Date and Time header.

## Feedback and access

- Status bar: `62,031 events · 3,648 shown · Ready`, with thousands separators; diagnostics go in tooltips.
- Specific error text next to the field that caused it. Query syntax help is secondary, not a permanent paragraph.
- Every icon-only action has an accessible name and a tooltip, every field a label, and keyboard focus stays visible. Refreshing results never replaces a draft the user is editing.
- Supported layout is 1024px wide and up; below that the side panes become overlays and nothing may overflow the page.
