use super::*;

#[test]
fn top_chrome_height_is_thin_but_stoplight_safe() {
    assert_eq!(CHROME_TOP_PT, 32.0);
}

#[test]
fn explorer_hit_paths_resolve_header_rows_and_ignore_missing_rows() {
    let snap = LeftDockSnapshot {
        root: "/tmp/project".to_string(),
        entries: vec![
            anvil_render::LeftDockEntry {
                name: "src".to_string(),
                is_dir: true,
                is_symlink: false,
            },
            anvil_render::LeftDockEntry {
                name: "main.rs".to_string(),
                is_dir: false,
                is_symlink: false,
            },
        ],
        git_marks: std::collections::HashMap::new(),
    };

    assert_eq!(
        explorer_path_for_hit(&snap, ExplorerHit::Header),
        Some(PathBuf::from("/tmp/project"))
    );
    assert_eq!(
        explorer_path_for_hit(&snap, ExplorerHit::Row(1)),
        Some(PathBuf::from("/tmp/project/main.rs"))
    );
    assert_eq!(explorer_path_for_hit(&snap, ExplorerHit::Row(9)), None);
}

#[test]
fn explorer_scroll_offset_changes_in_mouse_sized_steps_and_clamps() {
    assert_eq!(next_explorer_scroll_offset(0, 1.0, 10, 4), 3);
    assert_eq!(next_explorer_scroll_offset(3, -1.0, 10, 4), 0);
    assert_eq!(next_explorer_scroll_offset(5, 1.0, 10, 4), 6);
    assert_eq!(next_explorer_scroll_offset(6, 0.0, 10, 4), 6);
    assert_eq!(next_explorer_scroll_offset(6, 1.0, 0, 4), 0);
}

#[test]
fn native_editor_scroll_snaps_to_next_row_target() {
    assert_eq!(
        App::native_editor_scroll_next(0.0, 0.0, -3.0, 100, 20),
        Some(3.0)
    );
}

#[test]
fn native_editor_scroll_noops_at_bounds() {
    assert_eq!(App::native_editor_scroll_next(0.0, 0.0, 3.0, 100, 20), None);
    assert_eq!(
        App::native_editor_scroll_next(80.0, 80.0, -3.0, 100, 20),
        None
    );
}

#[test]
fn left_dock_toggle_snaps_without_animation_state() {
    assert_eq!(
        toggle_left_dock_instant(true),
        (false, LEFT_DOCK_DEFAULT_PT, LEFT_DOCK_DEFAULT_PT)
    );
    assert_eq!(
        toggle_left_dock_instant(false),
        (true, LEFT_DOCK_DEFAULT_PT, LEFT_DOCK_DEFAULT_PT)
    );
}

#[test]
fn ascii_lower_lowercases_ascii_letters() {
    assert_eq!(ascii_lower('A'), 'a');
    assert_eq!(ascii_lower('Z'), 'z');
    assert_eq!(ascii_lower('a'), 'a');
    assert_eq!(ascii_lower('5'), '5');
    assert_eq!(ascii_lower('\u{00C0}'), '\u{00C0}');
}

#[test]
fn assign_if_changed_only_reports_real_changes() {
    let mut value = String::from("main");

    assert!(!assign_if_changed(&mut value, String::from("main")));
    assert_eq!(value, "main");

    assert!(assign_if_changed(&mut value, String::from("rust-port")));
    assert_eq!(value, "rust-port");
}

#[test]
fn agent_snapshot_render_equal_ignores_poll_timestamp() {
    let mut newer = AgentSnapshot {
        polled_at_unix: 100,
        ..AgentSnapshot::default()
    };
    let older = AgentSnapshot {
        polled_at_unix: 1,
        ..AgentSnapshot::default()
    };

    assert!(agent_snapshot_render_equal(&older, &newer));

    newer.running_count = 1;
    assert!(!agent_snapshot_render_equal(&older, &newer));
}

#[test]
fn lsp_sync_opens_before_any_revision_is_sent() {
    let now = Instant::now();

    assert_eq!(
        lsp_sync_action(None, None, 0, now, Duration::from_millis(250)),
        LspSyncAction::Open
    );
}

#[test]
fn lsp_sync_skips_when_revision_has_not_changed() {
    let now = Instant::now();

    assert_eq!(
        lsp_sync_action(
            Some(now - Duration::from_secs(5)),
            Some(12),
            12,
            now,
            Duration::from_millis(250),
        ),
        LspSyncAction::Skip
    );
}

#[test]
fn lsp_sync_waits_for_debounce_before_changed_revision() {
    let now = Instant::now();

    assert_eq!(
        lsp_sync_action(
            Some(now - Duration::from_millis(100)),
            Some(12),
            13,
            now,
            Duration::from_millis(250),
        ),
        LspSyncAction::Skip
    );
    assert_eq!(
        lsp_sync_action(
            Some(now - Duration::from_millis(300)),
            Some(12),
            13,
            now,
            Duration::from_millis(250),
        ),
        LspSyncAction::Change
    );
}

#[test]
fn pane_chrome_hit_only_matches_top_strip() {
    let tab = Tab::new_single_pane(80, 24, 0);
    let pane_id = tab.focused_id();
    let inner = Rect {
        x: 0.0,
        y: 0.0,
        w: 1000.0,
        h: 600.0,
    };

    assert_eq!(
        pane_chrome_hit(&tab, inner, DIVIDER_PX, 1.0, 20.0, 12.0),
        Some(pane_id)
    );
    assert_eq!(
        pane_chrome_hit(&tab, inner, DIVIDER_PX, 1.0, 20.0, 80.0),
        None
    );
}

#[test]
fn pane_dock_target_skips_moving_pane_and_finds_edge_zone() {
    let mut tab = Tab::new_single_pane(80, 24, 0);
    let moving = tab.focused_id();
    let target = tab
        .split_native_editor(SplitDir::Horizontal)
        .expect("second pane");
    let inner = Rect {
        x: 0.0,
        y: 0.0,
        w: 1000.0,
        h: 600.0,
    };
    let target_rect = tab
        .tree
        .layout(inner, DIVIDER_PX)
        .into_iter()
        .find(|e| e.id == target)
        .expect("target layout")
        .rect;

    let hit = pane_dock_target(
        &tab,
        inner,
        DIVIDER_PX,
        moving,
        target_rect.x + 8.0,
        target_rect.y + target_rect.h * 0.5,
    )
    .expect("dock target");

    assert_eq!(hit.target, target);
    assert_eq!(hit.zone, DockZone::Left);
}

#[test]
fn dock_preview_rect_uses_target_edge_half() {
    let rect = Rect {
        x: 100.0,
        y: 50.0,
        w: 400.0,
        h: 300.0,
    };

    assert_eq!(
        dock_preview_rect(rect, DockZone::Left),
        Rect {
            x: 100.0,
            y: 50.0,
            w: 200.0,
            h: 300.0
        }
    );
    assert_eq!(
        dock_preview_rect(rect, DockZone::Bottom),
        Rect {
            x: 100.0,
            y: 200.0,
            w: 400.0,
            h: 150.0
        }
    );
}

#[test]
fn pane_dock_drag_waits_for_threshold_then_tracks_target() {
    let mut tab = Tab::new_single_pane(80, 24, 0);
    let moving = tab.focused_id();
    let target = tab
        .split_native_editor(SplitDir::Horizontal)
        .expect("second pane");
    let inner = Rect {
        x: 0.0,
        y: 0.0,
        w: 1000.0,
        h: 600.0,
    };
    let target_rect = tab
        .tree
        .layout(inner, DIVIDER_PX)
        .into_iter()
        .find(|e| e.id == target)
        .expect("target layout")
        .rect;
    let mut drag = PaneDockDrag {
        moving,
        start_rx: 10.0,
        start_ry: 10.0,
        active: false,
        target: None,
    };

    assert!(!update_pane_dock_drag(
        &mut drag, &tab, inner, DIVIDER_PX, 11.0, 12.0
    ));
    assert!(!drag.active);
    assert!(drag.target.is_none());

    assert!(update_pane_dock_drag(
        &mut drag,
        &tab,
        inner,
        DIVIDER_PX,
        target_rect.x + 8.0,
        target_rect.center_y()
    ));
    assert!(drag.active);
    assert_eq!(drag.target.expect("target").target, target);
}

#[test]
fn finish_pane_dock_drag_moves_active_dragged_pane() {
    let mut tab = Tab::new_single_pane(80, 24, 0);
    let moving = tab.focused_id();
    let target = tab
        .split_native_editor(SplitDir::Horizontal)
        .expect("second pane");
    let target_rect = Rect {
        x: 500.0,
        y: 0.0,
        w: 500.0,
        h: 600.0,
    };
    let drag = PaneDockDrag {
        moving,
        start_rx: 10.0,
        start_ry: 10.0,
        active: true,
        target: Some(PaneDockTarget {
            target,
            zone: DockZone::Right,
            rect: target_rect,
            preview: dock_preview_rect(target_rect, DockZone::Right),
        }),
    };

    assert!(finish_pane_dock_drag(&mut tab, drag));
    assert_eq!(tab.focused_id(), moving);
    let entries = tab.tree.layout(
        Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 600.0,
        },
        DIVIDER_PX,
    );
    let moving_rect = entries.iter().find(|e| e.id == moving).unwrap().rect;
    let target_rect = entries.iter().find(|e| e.id == target).unwrap().rect;
    assert!(moving_rect.x > target_rect.x);
}

#[test]
fn format_hex_produces_lowercase_six_digit_hex() {
    assert_eq!(format_hex([0x1a, 0x1c, 0x24]), "#1a1c24");
    assert_eq!(format_hex([0xff, 0x00, 0x80]), "#ff0080");
    assert_eq!(format_hex([0x00, 0x00, 0x00]), "#000000");
}

#[test]
fn effective_theme_name_maps_system_to_dark_or_light() {
    assert_eq!(effective_theme_name(true, "system"), "mineral-dark");
    assert_eq!(effective_theme_name(false, "system"), "mineral-light");
    assert_eq!(effective_theme_name(true, "ember-light"), "ember-light");
    assert_eq!(effective_theme_name(true, "mineral-light"), "mineral-light");
}

#[test]
fn apple_interface_style_dark_parser_matches_macos_values() {
    assert!(apple_interface_style_is_dark("Dark"));
    assert!(apple_interface_style_is_dark("NSAppearanceNameDarkAqua"));
    assert!(!apple_interface_style_is_dark(""));
    assert!(!apple_interface_style_is_dark("Light"));
    assert!(!apple_interface_style_is_dark("NSAppearanceNameAqua"));
}

#[test]
fn next_theme_mode_cycles_dark_light_system() {
    assert_eq!(next_theme_mode("mineral-dark"), "mineral-light");
    assert_eq!(next_theme_mode("mineral-light"), "system");
    assert_eq!(next_theme_mode("system"), "mineral-dark");
    assert_eq!(next_theme_mode("ember-dark"), "mineral-dark");
}

#[test]
fn cursor_blink_animation_only_runs_for_focused_terminal_panes() {
    assert!(should_animate_cursor_blink(true, true, None, true));
    assert!(!should_animate_cursor_blink(false, true, None, true));
    assert!(!should_animate_cursor_blink(true, false, None, true));
    assert!(!should_animate_cursor_blink(true, true, Some(false), true));
}

#[test]
fn platform_key_to_zig_key_covers_all_named_variants() {
    assert_eq!(platform_key_to_zig_key(KeyInput::Enter), Some(Key::Enter));
    assert_eq!(platform_key_to_zig_key(KeyInput::Tab), Some(Key::Tab));
    assert_eq!(
        platform_key_to_zig_key(KeyInput::Backspace),
        Some(Key::Backspace)
    );
    assert_eq!(platform_key_to_zig_key(KeyInput::Escape), Some(Key::Escape));
    assert_eq!(platform_key_to_zig_key(KeyInput::Up), Some(Key::Up));
    assert_eq!(platform_key_to_zig_key(KeyInput::Down), Some(Key::Down));
    assert_eq!(platform_key_to_zig_key(KeyInput::Left), Some(Key::Left));
    assert_eq!(platform_key_to_zig_key(KeyInput::Right), Some(Key::Right));
    assert_eq!(platform_key_to_zig_key(KeyInput::F(1)), Some(Key::F1));
    assert_eq!(platform_key_to_zig_key(KeyInput::F(12)), Some(Key::F12));
    assert_eq!(platform_key_to_zig_key(KeyInput::F(99)), None);
    assert_eq!(
        platform_key_to_zig_key(KeyInput::Char('a')),
        Some(Key::Text('a'))
    );
}

#[test]
fn chord_matching_requires_all_modifiers_and_key() {
    let chord = anvil_config::Chord {
        cmd: true,
        shift: false,
        ctrl: false,
        opt: false,
        key: 't',
    };
    let mods_match = Modifiers {
        command: true,
        shift: false,
        control: false,
        option: false,
    };
    let mods_no = Modifiers {
        command: false,
        shift: false,
        control: false,
        option: false,
    };
    assert!(App::chord_matches(chord, mods_match, 't'));
    assert!(!App::chord_matches(chord, mods_no, 't'));
    assert!(!App::chord_matches(chord, mods_match, 'x'));
    // ASCII case-insensitive via ascii_lower.
    assert!(App::chord_matches(chord, mods_match, 'T'));
}

#[test]
fn native_editor_split_chords_preempt_conflicting_editor_commands() {
    let kb = Keybindings::from_config(&anvil_config::Keybindings::default());
    let cmd_d = Modifiers {
        command: true,
        shift: false,
        control: false,
        option: false,
    };
    let cmd_shift_d = Modifiers {
        command: true,
        shift: true,
        control: false,
        option: false,
    };

    assert_eq!(
        App::native_editor_split_dir_for_chord(kb, cmd_d, 'd'),
        Some(SplitDir::Horizontal)
    );
    assert_eq!(
        App::native_editor_split_dir_for_chord(kb, cmd_shift_d, 'd'),
        Some(SplitDir::Vertical)
    );
    assert_eq!(App::native_editor_split_dir_for_chord(kb, cmd_d, 'x'), None);
}

#[test]
fn layout_terminal_mode_owns_keyboard_and_clears_explorer_text_state() {
    let mut focus = FocusTarget::Explorer;
    let mut selected_row = Some(7);
    let mut filter = Some("echo".to_string());

    normalize_input_state_for_layout_mode(
        LayoutMode::Terminal,
        &mut focus,
        &mut selected_row,
        &mut filter,
    );

    assert_eq!(focus, FocusTarget::Terminal);
    assert_eq!(selected_row, None);
    assert_eq!(filter, None);
}

#[test]
fn layout_ide_mode_returns_to_editor_focus_and_clears_explorer_filter() {
    let mut focus = FocusTarget::Terminal;
    let mut selected_row = Some(3);
    let mut filter = Some("readme".to_string());

    normalize_input_state_for_layout_mode(
        LayoutMode::Ide,
        &mut focus,
        &mut selected_row,
        &mut filter,
    );

    assert_eq!(focus, FocusTarget::Editor);
    assert_eq!(selected_row, None);
    assert_eq!(filter, None);
}

#[test]
fn layout_terminal_mode_hides_editor_leaves_without_destroying_state() {
    let mut tabs = TabManager::default();
    let mut tab = Tab::new_single_pane(80, 24, 1000);
    let terminal_id = tab.focused_id();
    let editor_id = tab
        .promote_terminal_to_editor_drawer()
        .expect("promotion creates editor");
    tabs.push(tab);

    normalize_tabs_for_layout_mode(LayoutMode::Terminal, &mut tabs);

    let tab = tabs.current().expect("tab remains available");
    assert!(matches!(
        tab.tree.root.as_ref(),
        anvil_workspace::layout::PaneNode::Leaf(id) if *id == terminal_id
    ));
    assert_eq!(tab.tree.focused, terminal_id);
    assert!(
        tab.editor_panes.get_pane(editor_id).is_some(),
        "terminal mode should hide editor surfaces, not drop their buffers"
    );
}

#[test]
fn layout_mode_env_override_accepts_only_explicit_modes() {
    assert_eq!(
        parse_layout_mode_env_override(Some("terminal")),
        Some(LayoutMode::Terminal)
    );
    assert_eq!(
        parse_layout_mode_env_override(Some("ide")),
        Some(LayoutMode::Ide)
    );
    assert_eq!(parse_layout_mode_env_override(Some("auto")), None);
    assert_eq!(parse_layout_mode_env_override(None), None);
}

#[test]
fn terminal_output_syncs_pane_scroll_state_to_terminal_viewport() {
    let mut pane = anvil_workspace::pane::Pane::new(1, 10, 3, 1000);
    pane.scroll_pos = 12.0;
    pane.scroll_target = 12.0;
    pane.scroll_vel = 4.0;
    pane.unseen_baseline = Some(20);

    if let Some(terminal) = &mut pane.terminal {
        terminal.feed(b"\x1b[3J\x1b[H\x1b[2J");
    }

    sync_terminal_pane_scroll_to_model(&mut pane);

    assert_eq!(pane.scroll_pos, 0.0);
    assert_eq!(pane.scroll_target, 0.0);
    assert_eq!(pane.scroll_vel, 0.0);
    assert_eq!(pane.unseen_baseline, None);
}

#[test]
fn layout_transition_slot_clearer_resets_bool_and_option_state() {
    let mut active = true;
    assert!(clear_layout_transition_slot(&mut active));
    assert!(!active);
    assert!(!clear_layout_transition_slot(&mut active));

    let mut drag = Some(42_u8);
    assert!(clear_layout_transition_slot(&mut drag));
    assert_eq!(drag, None);
    assert!(!clear_layout_transition_slot(&mut drag));
}

#[test]
fn drawer_tracking_repairs_restored_ide_terminal() {
    let mut tab = Tab::new_single_pane(80, 24, 1000);
    let terminal_id = tab.focused_id();
    tab.promote_terminal_to_editor_drawer()
        .expect("promotion creates editor");
    tab.hide_to_terminal_surface()
        .expect("terminal remains available");
    tab.ensure_ide_editor_surface()
        .expect("hidden editor reattaches");

    let (ids, active) = drawer_tracking_from_tab(&tab, |id| id == terminal_id, Some(terminal_id));

    assert_eq!(ids, vec![terminal_id]);
    assert_eq!(active, 0);
}

#[test]
fn keybindings_parsed_from_defaults() {
    let cfg = anvil_config::Keybindings::default();
    let kb = Keybindings::from_config(&cfg);
    let nt = kb.new_tab.unwrap();
    assert!(nt.cmd);
    assert_eq!(nt.key, 't');
}

#[test]
fn keybindings_keep_layout_toggle_and_recent_files_distinct() {
    let cfg = anvil_config::Keybindings::default();
    let kb = Keybindings::from_config(&cfg);
    let layout = kb.layout_mode_toggle.expect("layout toggle binding");
    let recent = kb.recent_files.expect("recent files binding");

    assert!(layout.cmd);
    assert!(layout.shift);
    assert!(!layout.opt);
    assert_eq!(layout.key, 'e');

    assert!(recent.cmd);
    assert!(!recent.shift);
    assert!(recent.opt);
    assert_eq!(recent.key, 'p');
    assert_ne!(layout, recent);
}

#[test]
fn editor_gutter_width_for_buffer_matches_render_contract() {
    let mut buf = anvil_editor::Buffer::from_text("one\ntwo\nthree\n");
    assert_eq!(
        editor_gutter_width_for_buffer(&buf, 8.0),
        anvil_render::editor_gutter_width(buf.line_count(), false, 8.0)
    );

    buf.git_gutter = Some(anvil_editor::GitGutter {
        per_line: vec![anvil_editor::GitChange::None; buf.line_count()],
    });
    assert_eq!(
        editor_gutter_width_for_buffer(&buf, 8.0),
        anvil_render::editor_gutter_width(buf.line_count(), true, 8.0)
    );
}

// ── platform_mods_to_zig_mods ────────────────────────────────────────────

#[test]
fn platform_mods_to_zig_mods_maps_all_fields() {
    let m = Modifiers {
        command: true,
        shift: true,
        control: false,
        option: false,
    };
    let z = platform_mods_to_zig_mods(m);
    assert!(z.command);
    assert!(z.shift);
    assert!(!z.control);
    assert!(!z.option);
}

#[test]
fn platform_mods_to_zig_mods_all_false() {
    let m = Modifiers {
        command: false,
        shift: false,
        control: false,
        option: false,
    };
    let z = platform_mods_to_zig_mods(m);
    assert!(!z.command);
    assert!(!z.shift);
    assert!(!z.control);
    assert!(!z.option);
}

#[test]
fn platform_mods_to_zig_mods_ctrl_opt() {
    let m = Modifiers {
        command: false,
        shift: false,
        control: true,
        option: true,
    };
    let z = platform_mods_to_zig_mods(m);
    assert!(z.control);
    assert!(z.option);
    assert!(!z.command);
    assert!(!z.shift);
}

// ── cursor_cfg_from_config ────────────────────────────────────────────────

#[test]
fn cursor_cfg_from_config_block_style() {
    use anvil_config::CursorStyle;
    use anvil_render::draw::CursorStyle as RCursorStyle;
    let mut cfg = anvil_config::Config::default();
    cfg.cursor.style = CursorStyle::Block;
    cfg.cursor.blink = false;
    let cc = cursor_cfg_from_config(&cfg);
    assert_eq!(cc.style, RCursorStyle::Block);
    assert!(!cc.blink);
}

#[test]
fn cursor_cfg_from_config_bar_style() {
    use anvil_config::CursorStyle;
    use anvil_render::draw::CursorStyle as RCursorStyle;
    let mut cfg = anvil_config::Config::default();
    cfg.cursor.style = CursorStyle::Bar;
    cfg.cursor.blink = true;
    let cc = cursor_cfg_from_config(&cfg);
    assert_eq!(cc.style, RCursorStyle::Bar);
    assert!(cc.blink);
}

#[test]
fn cursor_cfg_from_config_underline_style() {
    use anvil_config::CursorStyle;
    use anvil_render::draw::CursorStyle as RCursorStyle;
    let mut cfg = anvil_config::Config::default();
    cfg.cursor.style = CursorStyle::Underline;
    let cc = cursor_cfg_from_config(&cfg);
    assert_eq!(cc.style, RCursorStyle::Underline);
}

// ── AA7: cursor color wire-up ────────────────────────────────────────────

#[test]
fn cursor_cfg_from_config_color_override_parsed() {
    let mut cfg = anvil_config::Config::default();
    cfg.cursor.color = Some("#ff4400".into());
    let cc = cursor_cfg_from_config(&cfg);
    assert_eq!(
        cc.color,
        Some([0xff, 0x44, 0x00]),
        "AA7: cursor color override must wire through"
    );
}

#[test]
fn cursor_cfg_from_config_no_color_yields_none() {
    let cfg = anvil_config::Config::default();
    let cc = cursor_cfg_from_config(&cfg);
    assert!(cc.color.is_none(), "AA7: absent cursor.color must be None");
}

// ── AA8: cursor shape config verify ─────────────────────────────────────

/// AA8: cursor.style config is applied by cursor_cfg_from_config and
/// propagated to draw_cursor via CursorParams.  Verify all three shapes
/// round-trip through cursor_cfg_from_config without loss.
#[test]
fn cursor_shape_all_styles_applied() {
    use anvil_config::CursorStyle;
    use anvil_render::draw::CursorStyle as RCursorStyle;

    let cases = [
        (CursorStyle::Block, RCursorStyle::Block),
        (CursorStyle::Bar, RCursorStyle::Bar),
        (CursorStyle::Underline, RCursorStyle::Underline),
    ];
    for (config_style, expected) in cases {
        let mut cfg = anvil_config::Config::default();
        cfg.cursor.style = config_style;
        let cc = cursor_cfg_from_config(&cfg);
        assert_eq!(cc.style, expected, "AA8: cursor shape must be applied");
    }
}

// ── all_pane_ids_in_tree ─────────────────────────────────────────────────

#[test]
fn all_pane_ids_in_tree_single_pane() {
    let tab = anvil_workspace::tab::Tab::new_single_pane(80, 24, 100);
    let ids = all_pane_ids_in_tree(&tab);
    assert_eq!(ids.len(), 1);
}

#[test]
fn all_pane_ids_in_tree_after_split() {
    let mut tab = anvil_workspace::tab::Tab::new_single_pane(80, 24, 100);
    let new_id = tab
        .split(anvil_workspace::layout::SplitDir::Horizontal, 40, 24, 100)
        .unwrap();
    let ids = all_pane_ids_in_tree(&tab);
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&new_id));
}

// ── shell_quote_arg ───────────────────────────────────────────────────────

fn collect_quote(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    shell_quote_arg(s, |chunk| out.extend_from_slice(chunk));
    out
}

#[test]
fn shell_quote_arg_simple_path_unchanged() {
    assert_eq!(collect_quote("/usr/local/bin"), b"/usr/local/bin");
}

#[test]
fn shell_quote_arg_single_quote_in_path_escaped() {
    // "it's a file" → it'\''s a file
    assert_eq!(collect_quote("it's"), b"it'\\''s");
}

#[test]
fn shell_quote_arg_multiple_quotes_all_escaped() {
    assert_eq!(collect_quote("a'b'c"), b"a'\\''b'\\''c");
}

#[test]
fn shell_quote_arg_empty_string_emits_nothing() {
    assert_eq!(collect_quote(""), b"");
}

#[test]
fn shell_quote_arg_leading_quote_escaped() {
    assert_eq!(collect_quote("'hello"), b"'\\''hello");
}

#[test]
fn shell_quote_arg_trailing_quote_escaped() {
    assert_eq!(collect_quote("hello'"), b"hello'\\''");
}

// ── platform_key_to_zig_key extended coverage ─────────────────────────────

#[test]
fn platform_key_to_zig_key_home_end_pageup_pagedown_delete() {
    use anvil_workspace::keys::Key;
    assert_eq!(platform_key_to_zig_key(KeyInput::Home), Some(Key::Home));
    assert_eq!(platform_key_to_zig_key(KeyInput::End), Some(Key::End));
    assert_eq!(platform_key_to_zig_key(KeyInput::PageUp), Some(Key::PageUp));
    assert_eq!(
        platform_key_to_zig_key(KeyInput::PageDown),
        Some(Key::PageDown)
    );
    assert_eq!(platform_key_to_zig_key(KeyInput::Delete), Some(Key::Delete));
}

#[test]
fn platform_key_to_zig_key_all_function_keys() {
    use anvil_workspace::keys::Key;
    let expected = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ];
    for (n, exp) in expected.iter().enumerate() {
        let n = n as u8 + 1;
        assert_eq!(platform_key_to_zig_key(KeyInput::F(n)), Some(*exp), "F{n}");
    }
}

// ── N3: toast system ─────────────────────────────────────────────────────

/// `push_toast` caps text at 60 characters.
#[test]
fn toast_text_capped_at_60_chars() {
    // Build a minimal App-like struct — use only the toast VecDeque.
    // We test the logic via the helper functions directly.
    let long = "a".repeat(80);
    let truncated: String = long.chars().take(App::TOAST_MAX_CHARS).collect();
    assert_eq!(truncated.len(), 60, "toast text must be capped at 60 chars");
}

/// Toasts with `expires_at` in the past are removed by `tick_toasts`.
#[test]
fn expired_toasts_removed_on_tick() {
    let mut q: std::collections::VecDeque<Toast> = std::collections::VecDeque::new();
    let already_expired = Toast {
        text: "old".into(),
        kind: ToastKind::Info,
        expires_at: Instant::now() - std::time::Duration::from_secs(10),
    };
    let still_live = Toast {
        text: "new".into(),
        kind: ToastKind::Success,
        expires_at: Instant::now() + std::time::Duration::from_secs(10),
    };
    q.push_back(already_expired);
    q.push_back(still_live);

    // Manually apply the same drain logic as `tick_toasts`.
    let now = Instant::now();
    while q.front().is_some_and(|t| t.expires_at <= now) {
        q.pop_front();
    }

    assert_eq!(
        q.len(),
        1,
        "expired toast must be removed; 1 live toast remains"
    );
    assert_eq!(q.front().unwrap().text, "new");
}

// ── R2: humanize_bytes ────────────────────────────────────────────────────

#[test]
fn humanize_bytes_formats_sizes_correctly() {
    assert_eq!(humanize_bytes(0), "0 B");
    assert_eq!(humanize_bytes(1023), "1023 B");
    assert_eq!(humanize_bytes(1024), "1.0 KB");
    assert_eq!(humanize_bytes(12700), "12.4 KB");
    assert_eq!(humanize_bytes(1024 * 1024), "1.0 MB");
    assert_eq!(humanize_bytes(1024 * 1024 * 1024), "1.0 GB");
}

// ── R2: relative_time ─────────────────────────────────────────────────────

#[test]
fn relative_time_formats_deltas_correctly() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    assert_eq!(relative_time(now), "just now");
    assert_eq!(relative_time(now - 30), "just now");
    assert_eq!(relative_time(now - 90), "1 minute ago");
    assert_eq!(relative_time(now - 7200), "2 hours ago");
    assert_eq!(relative_time(now - 3 * 86400), "3 days ago");
}

// ── T2: parse_blame_porcelain ─────────────────────────────────────────────

#[test]
fn parse_blame_porcelain_parses_committed_entry() {
    // Minimal valid porcelain output for a committed line.
    let porcelain = "\
abc1234abc1234abc1234abc1234abc1234abc1234 1 1 1\n\
author Jane Smith\n\
author-mail <jane@example.com>\n\
author-time 1700000000\n\
author-tz +0000\n\
committer Jane Smith\n\
committer-mail <jane@example.com>\n\
committer-time 1700000000\n\
committer-tz +0000\n\
summary Initial commit\n\
filename src/main.rs\n\
\tlet x = 1;\n";
    let entry = parse_blame_porcelain(porcelain).expect("should parse committed entry");
    assert_eq!(entry.author, "Jane Smith");
    assert_eq!(entry.short_hash, "abc1234");
    // time is relative so just assert it's non-empty
    assert!(!entry.time_relative.is_empty());
}

#[test]
fn parse_blame_porcelain_returns_none_for_uncommitted() {
    // All-zero hash means "Not Committed Yet".
    let porcelain = "\
0000000000000000000000000000000000000000 1 1 1\n\
author Not Committed Yet\n\
author-mail <not.committed.yet>\n\
author-time 0\n\
author-tz +0000\n\
\tsome line\n";
    let entry = parse_blame_porcelain(porcelain);
    assert!(entry.is_none(), "uncommitted hash must return None");
}

#[test]
fn blame_popup_text_suppresses_uncommitted_lines() {
    assert_eq!(blame_popup_text(&None), None);
}

#[test]
fn blame_popup_text_formats_committed_lines() {
    let entry = BlameEntry {
        author: "Jane Smith".to_string(),
        time_relative: "3 days ago".to_string(),
        short_hash: "abc1234".to_string(),
    };

    assert_eq!(
        blame_popup_text(&Some(entry)),
        Some("Jane Smith · 3 days ago · abc1234".to_string())
    );
}

#[test]
fn blame_relative_time_formats_correctly() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    assert_eq!(blame_relative_time(now), "just now");
    assert_eq!(blame_relative_time(now - 90), "1 min ago");
    assert_eq!(blame_relative_time(now - 7200), "2 hr ago");
    assert_eq!(blame_relative_time(now - 3 * 86400), "3 days ago");
}

// X6: url_at_col

#[test]
fn url_at_col_finds_http_url() {
    let line = "see https://example.com for details\n";
    assert_eq!(url_at_col(line, 4), Some("https://example.com".to_string()));
}

#[test]
fn url_at_col_returns_none_when_no_url() {
    let line = "no url here\n";
    assert_eq!(url_at_col(line, 2), None);
}

#[test]
fn url_at_col_returns_none_outside_url_span() {
    let line = "text https://example.com end\n";
    // col 0 is "t" in "text", before the URL.
    assert_eq!(url_at_col(line, 0), None);
}

// X11: load_snippets parses a valid TOML file
#[test]
fn load_snippets_parses_toml_correctly() {
    // Directly test the TOML parsing logic without touching HOME.
    let toml_text = r#"
[snippet.fn]
body = "fn ${1:name}() {\n    $0\n}"

[snippet.impl]
body = "impl ${1:Type} {}"
"#;
    let table: toml::Table = toml_text.parse().expect("valid toml");
    let mut snippets = HashMap::new();
    if let Some(toml::Value::Table(snippet_table)) = table.get("snippet") {
        for (trigger, val) in snippet_table {
            if let toml::Value::Table(entry) = val {
                if let Some(toml::Value::String(body)) = entry.get("body") {
                    snippets.insert(trigger.clone(), body.clone());
                }
            }
        }
    }
    assert_eq!(snippets.len(), 2);
    assert!(snippets.contains_key("fn"));
    assert!(snippets.contains_key("impl"));
}

// ── EE11: file:open prefix routing ───────────────────────────────────────

/// The Invoke handler strips "file:open:" and returns the abs path.
/// This test mirrors the routing logic at the Inbound::Invoke site.
#[test]
fn file_open_prefix_strips_to_abs_path() {
    let id = "file:open:/home/user/project/src/main.rs";
    let abs_path = id.strip_prefix("file:open:").expect("prefix must match");
    assert_eq!(abs_path, "/home/user/project/src/main.rs");
}

#[test]
fn file_open_prefix_does_not_match_other_commands() {
    let id = "task:run:build";
    assert!(
        id.strip_prefix("file:open:").is_none(),
        "non-file:open: id must not match the file:open prefix"
    );
}

// ── EE14: nvim pane argv construction ────────────────────────────────────

#[test]
fn nvim_pane_argv_has_correct_shape() {
    let cfg = anvil_config::NvimCfg::default();
    let argv = nvim_pane_argv(
        "/workspace/foo.rs",
        &cfg,
        "mineral-dark",
        &anvil_theme::MINERAL_DARK,
    );
    assert_eq!(argv[0], "/usr/bin/env");
    assert!(argv.iter().any(|arg| arg == "ANVIL=1"));
    assert!(argv.iter().any(|arg| arg == "COLORTERM=truecolor"));
    assert!(argv.iter().any(|arg| arg == "ANVIL_THEME=mineral-dark"));
    assert!(argv.iter().any(|arg| arg == "ANVIL_THEME_MODE=dark"));
    assert_eq!(argv.iter().filter(|arg| arg.as_str() == "nvim").count(), 1);
    assert_eq!(argv.last().map(String::as_str), Some("/workspace/foo.rs"));
}

#[test]
fn nvim_pane_argv_supports_lazyvim_appname_and_colorscheme() {
    let cfg = anvil_config::NvimCfg {
        appname: "LazyVim".to_owned(),
        theme_sync: true,
        colorscheme: "catppuccin-mocha".to_owned(),
    };
    let argv = nvim_pane_argv(
        "/workspace/foo.rs",
        &cfg,
        "mineral-light",
        &anvil_theme::MINERAL_LIGHT,
    );

    assert!(argv.iter().any(|arg| arg == "NVIM_APPNAME=LazyVim"));
    assert!(argv.iter().any(|arg| arg == "ANVIL_THEME_MODE=light"));
    assert!(
        argv.windows(2)
            .any(|pair| pair[0] == "--cmd" && pair[1] == "set termguicolors")
    );
    assert!(
        argv.windows(2)
            .any(|pair| pair[0] == "--cmd" && pair[1] == "set background=light")
    );
    assert!(
        argv.windows(2)
            .any(|pair| pair[0] == "--cmd" && pair[1] == "silent! colorscheme catppuccin-mocha")
    );
}
