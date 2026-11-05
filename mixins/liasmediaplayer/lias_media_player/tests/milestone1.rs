use lias_media_player::platform::{horizon_impl::*, *};
use lias_media_player::ui::{theme, *};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct Recording {
    rects: Vec<Rect>,
    clips: Vec<(usize, Rect)>,
    sprites: Vec<[f32; 4]>,
    texts: Vec<String>,
}
struct Backend(Rc<RefCell<Recording>>);
impl HudBackend for Backend {
    fn size(&self) -> [f32; 2] {
        [640., 480.]
    }
    fn fill(&mut self, r: Rect, _: u32) {
        self.0.borrow_mut().rects.push(r);
    }
    fn sprite(&mut self, source: [f32; 4], r: Rect) {
        let mut log = self.0.borrow_mut();
        log.sprites.push(source);
        log.rects.push(r);
    }
    fn text(&mut self, t: &str, _: [f32; 2], _: u32, _: bool) {
        self.0.borrow_mut().texts.push(t.into());
    }
    fn text_width(&self, t: &str) -> f32 {
        t.chars().count() as f32 * 6.
    }
    fn vertex_count(&self) -> usize {
        self.0.borrow().rects.len()
    }
    fn clip_since(&mut self, start: usize, r: Rect) {
        let mut log = self.0.borrow_mut();
        log.clips.push((start, r));
        let tail = log
            .rects
            .drain(start..)
            .map(|q| q.intersect(r))
            .filter(|q| q.w > 0. && q.h > 0.)
            .collect::<Vec<_>>();
        log.rects.extend(tail);
    }
}
fn recording() -> (Rc<RefCell<Recording>>, Backend) {
    let log = Rc::new(RefCell::new(Recording::default()));
    (log.clone(), Backend(log))
}
fn down(x: f32, y: f32) -> InputEvent {
    InputEvent::Down {
        at: [x, y],
        button: 0,
        mods: Modifiers::default(),
    }
}
fn up(x: f32, y: f32) -> InputEvent {
    InputEvent::Up {
        at: [x, y],
        button: 0,
    }
}
fn drag(x: f32, y: f32) -> InputEvent {
    InputEvent::Drag {
        at: [x, y],
        button: 0,
        mods: Modifiers {
            shift: true,
            ..Default::default()
        },
    }
}
fn key(key: Key) -> InputEvent {
    InputEvent::Key {
        key,
        mods: Modifiers::default(),
    }
}

#[test]
fn atlas_full_reuse_stale_handles_and_row_copy() {
    let mut a = ModAtlasPacker::new();
    assert_eq!(a.allocate(0, 1, &[]), Err(AtlasError::InvalidSize));
    assert_eq!(a.allocate(1, 1, &[]), Err(AtlasError::InvalidPixels));
    let first = a.allocate(512, 1024, &vec![7; 512 * 1024 * 4]).unwrap();
    let second = a.allocate(512, 1024, &vec![9; 512 * 1024 * 4]).unwrap();
    assert_eq!(a.allocate(1, 1, &[0; 4]), Err(AtlasError::Full));
    a.flush(|w, h, p| {
        assert_eq!((w, h), (1024, 1024));
        assert_eq!(&p[2044..2052], &[7, 7, 7, 7, 9, 9, 9, 9]);
        true
    });
    assert!(a.free(first));
    assert!(!a.free(first));
    let third = a.allocate(512, 1024, &vec![3; 512 * 1024 * 4]).unwrap();
    assert_ne!(first, third);
    assert_eq!(a.region(third).unwrap().x, 0);
    assert_eq!(a.update(first, &[]), Err(AtlasError::UnknownTexture));
    a.free(second);
    a.free(third);
    assert!(a.allocate(1024, 1024, &vec![1; 1024 * 1024 * 4]).is_ok());
}
#[test]
fn atlas_upload_retry_dirty_updates_and_clear() {
    let mut a = ModAtlasPacker::new();
    let id = a.allocate(2, 2, &[1; 16]).unwrap();
    assert!(!a.flush(|_, _, _| false));
    assert!(a.is_dirty());
    a.flush(|_, _, _| true);
    assert!(!a.is_dirty());
    a.flush(|_, _, _| panic!("unchanged atlas uploaded"));
    assert_eq!(a.update(id, &[2; 4]), Err(AtlasError::InvalidPixels));
    assert!(!a.is_dirty());
    a.update(id, &[2; 16]).unwrap();
    assert!(a.is_dirty());
    a.free(id);
    a.flush(|_, _, p| {
        assert!(p.iter().all(|&b| b == 0));
        true
    });
    a.invalidate();
    assert!(a.is_dirty());
}
#[test]
fn nested_clips_survive_removed_vertices_and_restore_parent_for_siblings() {
    let atlas = ModAtlasPacker::new();
    let (log, backend) = recording();
    {
        let mut c = HorizonCanvas::new(backend, &atlas);
        c.fill(Rect::new(500., 0., 1., 1.), 0);
        c.push_clip(Rect::new(0., 0., 100., 100.));
        c.fill(Rect::new(90., 90., 30., 30.), 0);
        c.push_clip(Rect::new(200., 200., 20., 20.));
        c.fill(Rect::new(200., 200., 10., 10.), 0);
        c.pop_clip();
        c.push_clip(Rect::new(10., 10., 10., 10.));
        c.fill(Rect::new(0., 0., 30., 30.), 0);
        c.pop_clip();
        c.fill(Rect::new(80., 80., 30., 30.), 0);
        // Drop balances a caller's remaining clip.
    }
    assert_eq!(
        log.borrow().rects,
        vec![
            Rect::new(500., 0., 1., 1.),
            Rect::new(90., 90., 10., 10.),
            Rect::new(10., 10., 10., 10.),
            Rect::new(80., 80., 20., 20.)
        ]
    );
}
#[test]
fn texture_uvs_stay_within_allocated_region() {
    let mut a = ModAtlasPacker::new();
    a.allocate(8, 8, &[0; 256]).unwrap();
    let id = a.allocate(4, 4, &[0; 64]).unwrap();
    let (log, b) = recording();
    let mut c = HorizonCanvas::new(b, &a);
    c.texture(id, Rect::new(0., 0., 40., 40.), [0.25, 0.5, 1., 1.]);
    assert_eq!(log.borrow().sprites, vec![[9., 2., 3., 2.]]);
}
#[test]
fn themes_preserve_java_ids_and_fallback_roles() {
    assert_eq!(
        theme::builtins().map(|t| t.id),
        [
            "liasmediaplayer:dark",
            "liasmediaplayer:contrast",
            "liasmediaplayer:minecraft",
            "liasmediaplayer:light"
        ]
    );
    let addon = ThemeSpec::new("test:custom", "Custom").with(ThemeRole::Fill, 0xff112233);
    assert_eq!(addon.color(ThemeRole::Fill), 0xff112233);
    assert_eq!(
        addon.color(ThemeRole::Text),
        theme::dark().color(ThemeRole::Text)
    );
}
#[test]
fn button_requires_matching_press_and_release_inside() {
    let mut b = Button::new("Test");
    b.layout(Rect::new(0., 0., 50., 20.));
    let mut input = InputState::default();
    let mut actions = vec![];
    let mut ctx = EventContext {
        input: &mut input,
        actions: &mut actions,
    };
    assert!(!b.event(&up(10., 10.), &mut ctx));
    b.event(&down(10., 10.), &mut ctx);
    assert!(b.event(&up(100., 100.), &mut ctx));
    assert!(ctx.actions.is_empty());
    b.event(&down(10., 10.), &mut ctx);
    b.event(&up(10., 10.), &mut ctx);
    assert_eq!(ctx.actions, &[UiAction::Click(b.id())]);
}
#[test]
fn slider_captures_outside_and_keyboard_clamps() {
    let mut s = Slider::new(0.5);
    s.layout(Rect::new(0., 0., 100., 20.));
    let mut input = InputState::default();
    let mut actions = vec![];
    let mut ctx = EventContext {
        input: &mut input,
        actions: &mut actions,
    };
    s.event(&down(50., 10.), &mut ctx);
    s.event(&drag(300., 300.), &mut ctx);
    assert_eq!(s.value(), 1.);
    s.event(&up(-100., 50.), &mut ctx);
    assert_eq!(s.value(), 0.);
    assert_eq!(ctx.input.capture, None);
    s.event(&key(Key::End), &mut ctx);
    s.event(&key(Key::Right), &mut ctx);
    assert_eq!(s.value(), 1.);
}
#[test]
fn text_editing_preserves_utf8_boundaries_and_selection_limit() {
    let mut f = TextField::new("aé🙂");
    f.max_chars = 3;
    f.layout(Rect::new(0., 0., 100., 20.));
    let mut input = InputState::default();
    let mut actions = vec![];
    let mut ctx = EventContext {
        input: &mut input,
        actions: &mut actions,
    };
    f.event(&down(10., 10.), &mut ctx);
    f.event(&InputEvent::Char('x'), &mut ctx);
    assert_eq!(f.text(), "aé🙂");
    f.event(&key(Key::Left), &mut ctx);
    f.event(&key(Key::Backspace), &mut ctx);
    assert_eq!(f.text(), "a🙂");
    f.event(&key(Key::Delete), &mut ctx);
    assert_eq!(f.text(), "a");
    f.event(
        &InputEvent::Key {
            key: Key::A,
            mods: Modifiers {
                ctrl: true,
                ..Default::default()
            },
        },
        &mut ctx,
    );
    f.event(&InputEvent::Char('界'), &mut ctx);
    assert_eq!(f.text(), "界");
    f.event(&key(Key::Enter), &mut ctx);
    assert_eq!(
        ctx.actions.last(),
        Some(&UiAction::Submit(f.id(), "界".into()))
    );
}
#[test]
fn row_column_and_stack_use_measured_sizes() {
    let a = ModAtlasPacker::new();
    let (_, b) = recording();
    let c = HorizonCanvas::new(b, &a);
    let mut row = Row::new(vec![
        Box::new(Button::new("A")),
        Box::new(Button::new("Longer")),
    ]);
    row.arrange(&c, Rect::new(10., 10., 200., 20.));
    assert_eq!(row.0.children[0].rect().w, 18.);
    assert_eq!(row.0.children[1].rect().x, 32.);
    let mut column = Column::new(vec![Box::new(Label::new("A")), Box::new(row)]);
    column.arrange(&c, Rect::new(10., 10., 200., 100.));
    assert_eq!(column.0.children[1].rect().y, 23.);
    let mut stack = Stack::new(vec![Box::new(Label::new("A")), Box::new(Label::new("B"))]);
    stack.arrange(&c, Rect::new(5., 5., 40., 40.));
    assert_eq!(stack.0.children[0].rect(), stack.0.children[1].rect());
}
#[test]
fn scrollpane_clips_and_offscreen_children_cannot_receive_clicks() {
    let a = ModAtlasPacker::new();
    let (log, b) = recording();
    let mut c = HorizonCanvas::new(b, &a);
    let mut pane = ScrollPane::new(Box::new(Column::new(
        (0..10)
            .map(|i| Box::new(Button::new(i.to_string())) as Box<dyn Widget>)
            .collect(),
    )));
    pane.arrange(&c, Rect::new(10., 10., 100., 40.));
    let mut input = InputState::default();
    let mut actions = vec![];
    let mut ctx = EventContext {
        input: &mut input,
        actions: &mut actions,
    };
    assert!(!pane.event(&down(20., 60.), &mut ctx));
    assert!(pane.event(
        &InputEvent::Scroll {
            at: [20., 20.],
            delta: -100.,
            mods: Modifiers::default()
        },
        &mut ctx
    ));
    assert!(pane.offset > 0.);
    pane.draw(
        &mut c,
        &DrawContext {
            theme: &theme::dark(),
            mouse: None,
            focus: None,
        },
    );
    assert!(
        log.borrow()
            .rects
            .iter()
            .all(|r| r.y >= 10. && r.bottom() <= 50.)
    );
}
fn manager() -> WindowManager {
    let mut wm = WindowManager::new();
    wm.open(FloatingWindow::new(
        "A",
        Rect::new(20., 20., 180., 120.),
        Box::new(Button::new("A")),
    ));
    wm.open(FloatingWindow::new(
        "B",
        Rect::new(250., 20., 180., 120.),
        Box::new(Button::new("B")),
    ));
    wm
}
fn render(wm: &mut WindowManager, interactive: bool) {
    let a = ModAtlasPacker::new();
    let (_, b) = recording();
    let mut c = HorizonCanvas::new(b, &a);
    wm.draw(&mut c, interactive, None);
}
#[test]
fn windows_raise_drag_resize_and_keep_release_ownership() {
    let mut wm = manager();
    render(&mut wm, true);
    let id = wm.windows()[0].id;
    assert!(wm.event(&down(40., 30.)));
    assert_eq!(wm.windows().last().unwrap().id, id);
    wm.event(&drag(80., 60.));
    wm.event(&up(80., 60.));
    assert_eq!(
        wm.windows().last().unwrap().rect,
        Rect::new(60., 50., 180., 120.)
    );
    wm.event(&down(239., 169.));
    wm.event(&drag(279., 199.));
    wm.event(&up(600., 400.));
    assert_eq!(
        wm.windows().last().unwrap().rect,
        Rect::new(60., 50., 220., 150.)
    );
    assert!(!wm.event(&up(60., 60.)));
    assert!(!wm.event(&down(600., 400.)));
}
#[test]
fn snapping_prefers_nearest_edge_and_can_align_neighbours() {
    assert_eq!(SnapEngine::axis(3., 50., &[0., 100.], 6.), 0.);
    assert_eq!(SnapEngine::axis(8., 50., &[0.], 6.), 8.);
    assert_eq!(
        SnapEngine::snap(
            Rect::new(103., 20., 40., 40.),
            Rect::new(0., 0., 640., 480.),
            [Rect::new(50., 100., 50., 50.)].into_iter()
        )
        .x,
        100.
    );
}
#[test]
fn zoom_theatre_restore_hide_and_hud_pin() {
    let mut wm = manager();
    render(&mut wm, true);
    let original = wm.windows()[1].rect;
    wm.event(&InputEvent::Scroll {
        at: [300., 80.],
        delta: 2.,
        mods: Modifiers {
            ctrl: true,
            ..Default::default()
        },
    });
    assert!(wm.windows()[1].rect.w > original.w);
    let r = wm.windows()[1].rect;
    wm.event(&down(r.right() - 50., r.y + 10.));
    wm.event(&up(0., 0.));
    assert!(wm.windows()[1].theatre);
    render(&mut wm, true);
    wm.event(&down(584., 16.));
    wm.event(&up(0., 0.));
    assert!(!wm.windows()[1].theatre);
    assert_eq!(wm.windows()[1].rect, r);
    wm.event(&down(r.right() - 30., r.y + 10.));
    wm.event(&up(0., 0.));
    assert!(wm.windows()[1].hidden);
    wm.reveal();
    assert!(!wm.windows()[1].hidden);
    wm.event(&down(r.right() - 70., r.y + 10.));
    wm.event(&up(0., 0.));
    assert!(!wm.windows()[1].pinned);
    let atlas = ModAtlasPacker::new();
    let (log, b) = recording();
    let mut c = HorizonCanvas::new(b, &atlas);
    wm.draw(&mut c, false, None);
    assert!(!log.borrow().texts.iter().any(|t| t == "B"));
}
#[test]
fn overlapping_windows_and_cancel_do_not_leak_captured_input() {
    let mut wm = manager();
    render(&mut wm, true);
    wm.event(&down(40., 60.));
    wm.event(&InputEvent::Cancel);
    assert!(!wm.event(&up(40., 60.)));
    assert!(wm.take_actions().is_empty());
    wm.event(&down(40., 60.));
    wm.event(&up(40., 60.));
    assert_eq!(wm.take_actions().len(), 1);
    assert!(wm.event(&key(Key::Tab)));
    assert!(wm.event(&key(Key::Enter)));
    assert_eq!(wm.take_actions().len(), 1);
}
#[test]
fn demo_can_reopen_after_close_and_survives_world_sized_viewports() {
    let mut demo = lias_media_player::ui::demo::UiDemo::default();
    demo.open();
    assert_eq!(demo.windows.windows().len(), 2);
    render(&mut demo.windows, true);
    render(&mut demo.windows, false);
    demo.windows.clear();
    demo.open();
    assert_eq!(demo.windows.windows().len(), 2);
}

#[test]
fn pointer_text_selection_and_tab_reveal_use_measured_geometry() {
    let a = ModAtlasPacker::new();
    let (_, b) = recording();
    let c = HorizonCanvas::new(b, &a);
    let mut field = TextField::new("aé🙂z");
    field.arrange(&c, Rect::new(0., 0., 100., 20.));
    let mut input = InputState::default();
    let mut actions = vec![];
    let mut ctx = EventContext {
        input: &mut input,
        actions: &mut actions,
    };
    field.event(&down(10., 10.), &mut ctx);
    field.event(&drag(22., 10.), &mut ctx);
    field.event(&up(22., 10.), &mut ctx);
    field.event(&InputEvent::Char('X'), &mut ctx);
    assert_eq!(field.text(), "aXz");
    let last = Button::new("Last");
    let id = last.id();
    let mut pane = ScrollPane::new(Box::new(Column::new(vec![
        Box::new(Label::new("First")),
        Box::new(Label::new("Second")),
        Box::new(last),
    ])));
    pane.arrange(&c, Rect::new(0., 0., 100., 20.));
    pane.reveal_focus(id);
    let focused = pane.focused_rect(id).unwrap();
    assert!(focused.y >= 0. && focused.bottom() <= 20.);
}
#[test]
fn each_resize_corner_keeps_opposite_corner_fixed() {
    for (at, to, expected) in [
        ([21., 21.], [11., 11.], Rect::new(10., 10., 190., 130.)),
        ([199., 21.], [209., 11.], Rect::new(20., 10., 190., 130.)),
        ([21., 139.], [11., 149.], Rect::new(10., 20., 190., 130.)),
        ([199., 139.], [209., 149.], Rect::new(20., 20., 190., 130.)),
    ] {
        let mut wm = manager();
        render(&mut wm, true);
        wm.event(&down(at[0], at[1]));
        wm.event(&drag(to[0], to[1]));
        wm.event(&up(to[0], to[1]));
        assert_eq!(wm.windows().last().unwrap().rect, expected);
    }
}
#[test]
fn overlapped_window_only_activates_topmost_and_close_owns_release() {
    let mut wm = WindowManager::new();
    let lower = Button::new("Lower");
    let upper = Button::new("Upper");
    let id = upper.id();
    wm.open(FloatingWindow::new(
        "Lower",
        Rect::new(20., 20., 180., 120.),
        Box::new(lower),
    ));
    wm.open(FloatingWindow::new(
        "Upper",
        Rect::new(20., 20., 180., 120.),
        Box::new(upper),
    ));
    render(&mut wm, true);
    wm.event(&down(40., 60.));
    wm.event(&up(40., 60.));
    assert_eq!(wm.take_actions(), vec![UiAction::Click(id)]);
    wm.event(&down(190., 30.));
    assert_eq!(wm.windows().len(), 1);
    assert!(wm.event(&up(190., 30.)));
    assert!(wm.take_actions().is_empty());
}
#[test]
fn toggle_icon_seek_and_virtual_list_emit_actions() {
    let mut input = InputState::default();
    let mut actions = vec![];
    let mut ctx = EventContext {
        input: &mut input,
        actions: &mut actions,
    };
    let mut toggle = Toggle::new("On", false);
    toggle.layout(Rect::new(0., 0., 100., 20.));
    toggle.event(&down(10., 10.), &mut ctx);
    toggle.event(&up(10., 10.), &mut ctx);
    assert_eq!(ctx.actions.pop(), Some(UiAction::Toggle(toggle.id(), true)));
    let mut icon = IconButton::new("+");
    icon.layout(Rect::new(0., 0., 20., 20.));
    icon.event(&down(10., 10.), &mut ctx);
    icon.event(&up(10., 10.), &mut ctx);
    assert_eq!(ctx.actions.pop(), Some(UiAction::Click(icon.id())));
    let mut seek = SeekBar::new(120.);
    seek.layout(Rect::new(0., 0., 100., 20.));
    seek.event(&down(50., 10.), &mut ctx);
    seek.event(&up(50., 10.), &mut ctx);
    assert_eq!(seek.position(), 60.);
    let mut list = ListView::new((0..1000).map(|i| i.to_string()).collect());
    list.layout(Rect::new(0., 0., 100., 40.));
    list.event(&down(10., 10.), &mut ctx);
    list.event(&up(10., 10.), &mut ctx);
    list.event(&key(Key::End), &mut ctx);
    assert_eq!(list.selected, Some(999));
    let a = ModAtlasPacker::new();
    let (log, b) = recording();
    let mut c = HorizonCanvas::new(b, &a);
    list.draw(
        &mut c,
        &DrawContext {
            theme: &theme::dark(),
            mouse: None,
            focus: None,
        },
    );
    assert!(log.borrow().texts.len() <= 3);
    assert!(log.borrow().texts.contains(&"999".to_owned()));
}
