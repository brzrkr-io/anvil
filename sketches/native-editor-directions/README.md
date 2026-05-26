# Anvil Native Editor Direction Companion

Open:

```sh
open /Users/pjanderson/projects/caldera/anvil/sketches/native-editor-directions/index.html
```

## Research synthesis

Current native editor code already has more substrate than the product communicates:

- Native editor renderer: `crates/anvil-render/src/editor.rs` paints buffer text, gutter line numbers, syntax colors, diagnostics, git marks, selections, cursor bars.
- Workspace integration: `crates/anvil-render/src/workspace.rs` can route non-terminal panes through `draw_editor_into`.
- Editor state: `crates/anvil-workspace/src/editor_pane.rs` maps pane IDs to buffers and editor view state.
- Left dock: `crates/anvil-render/src/left_dock.rs` renders explorer and outline but currently reads as static/non-interactive.
- Rescue spec priority says the first “real editor” moment is: default IDE in project dirs + explorer click opens a file + pane chrome/tabs soon after.

The visual problem is not “draw text better.” It is product legibility: users need visible file identity, clickable explorer rows, pane/editor chrome, tabs, and bottom-terminal separation.

## Variants

### A — Ember Command Deck

Design stance: closest to Zed/VS Code, but with Anvil’s Ember operator console styling.

Key choices:
- Left explorer + outline dock.
- Top file tabs inside the editor pane.
- Filename/dirty state visible immediately.
- Terminal is a bottom drawer, not the main app identity.
- Medium density, easiest bridge from current implementation.

Trade-offs:
- Strongest first ship target; maps directly to D1-D5.
- Less distinctive than the other directions.

Best for:
- The next implementation slice and likely default native editor layout.

### B — Operator Instrument Panel

Design stance: denser Caldera control-plane/editor hybrid.

Key choices:
- Icon rail for mode surfaces.
- Explorer plus right-side symbols/diagnostics/next-action panel.
- Mini-map and evidence/action surfaces.
- High-density operator-console feel.

Trade-offs:
- More distinctive and powerful.
- More layout work; should not block D1-D2.

Best for:
- Follow-up after core editor affordances exist.

### C — Agent Workspace

Design stance: Anvil as editor + terminal + agent orchestration workspace, not a generic IDE clone.

Key choices:
- Work graph / task list integrated into the left dock.
- Editor primary; terminal/evidence drawer secondary.
- Agent telemetry is first-class in the top/bottom bars.
- Strongest product point of view.

Trade-offs:
- Highest product/design risk.
- Needs the native editor to be functional first, or it becomes another stub surface.

Best for:
- Long-term north star once D1-D6 are reliable.

## Recommendation

Pick A as the immediate implementation target. It gives the user the fastest visible proof that Anvil is a native editor: project opens into IDE mode, explorer row clicks produce editor tabs, pane chrome names the file, and the terminal moves into a bottom drawer later.

Use B/C as product direction pressure: preserve operator-console density and agent telemetry while shipping A’s lower-risk affordances first.
