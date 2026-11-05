//! An interactive M1 workbench. It exercises UI infrastructure without implying playback.
use super::{theme, *};
use crate::platform::{Canvas, InputEvent, Rect, TextureId, horizon_impl::ModAtlasPacker};
#[derive(Default)]
pub struct UiDemo {
    pub atlas: ModAtlasPacker,
    pub windows: WindowManager,
    theme_button: Option<WidgetId>,
    theme_index: usize,
    screen: Option<String>,
}
struct Preview {
    id: WidgetId,
    rect: Rect,
    texture: TextureId,
}
impl Widget for Preview {
    fn id(&self) -> WidgetId {
        self.id
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn layout(&mut self, r: Rect) {
        self.rect = r;
    }
    fn measure(&self, _: &dyn Canvas, max: [f32; 2]) -> [f32; 2] {
        [max[0].min(180.), max[1].min(45.)]
    }
    fn draw(&self, c: &mut dyn Canvas, _: &DrawContext<'_>) {
        c.texture(self.texture, self.rect, [0., 0., 1., 1.]);
    }
    fn event(&mut self, _: &InputEvent, _: &mut EventContext<'_>) -> bool {
        false
    }
}
impl UiDemo {
    pub fn observe_screen(&mut self, screen: &str, focused: bool) {
        if !focused || self.screen.as_deref() != Some(screen) {
            self.windows.cancel_input();
            self.screen = Some(screen.to_owned());
        }
    }

    pub fn open(&mut self) {
        if !self.windows.windows().is_empty() {
            self.windows.reveal();
            return;
        }
        // Reclaim the previous demonstration's handles when every window was closed.
        self.atlas = ModAtlasPacker::new();
        let pixels: Vec<u8> = (0..64 * 32)
            .flat_map(|i| {
                if (i % 64 / 8 + i / 64 / 8) % 2 == 0 {
                    [76, 166, 255, 255]
                } else {
                    [26, 40, 60, 255]
                }
            })
            .collect();
        let texture = self
            .atlas
            .allocate(64, 32, &pixels)
            .expect("demo image fits an empty atlas");
        let theme = Button::new("Cycle theme");
        self.theme_button = Some(theme.id());
        let field = TextField::new("Edit this text: café");
        let children: Vec<Box<dyn Widget>> = vec![
            Box::new(Label::new("Milestone 1 · UI preview")),
            Box::new(Stack::new(vec![Box::new(Preview {
                id: Label::new("").id(),
                rect: Rect::default(),
                texture,
            })])),
            Box::new(Row::new(vec![
                Box::new(theme),
                Box::new(IconButton::new("+")),
            ])),
            Box::new(Toggle::new("Example toggle", true)),
            Box::new(Label::new("Volume / seek (demonstration)")),
            Box::new(Slider::new(0.65)),
            Box::new(SeekBar::new(180.)),
            Box::new(field),
            Box::new(ListView::new(
                (1..=12).map(|i| format!("Example list item {i}")).collect(),
            )),
        ];
        self.windows.open(FloatingWindow::new(
            "Lia's Media Player · UI",
            Rect::new(20., 24., 270., 290.),
            Box::new(ScrollPane::new(Box::new(Column::new(children)))),
        ));
        self.windows.open(FloatingWindow::new(
            "Window controls",
            Rect::new(310., 44., 230., 150.),
            Box::new(Column::new(vec![
                Box::new(Label::new("Drag the title; resize any corner")),
                Box::new(Label::new("Ctrl + wheel: zoom window")),
                Box::new(Label::new("Shift + drag: disable snapping")),
                Box::new(Label::new("P: HUD pin   T: theatre / restore")),
                Box::new(Label::new("_: hide   X: close   M: reopen")),
                Box::new(Label::new("Tab: focus   Esc: return to game")),
            ])),
        ));
    }
    pub fn event(&mut self, e: &InputEvent) -> bool {
        let used = self.windows.event(e);
        for action in self.windows.take_actions() {
            if let UiAction::Click(id) = action
                && Some(id) == self.theme_button
            {
                self.theme_index = (self.theme_index + 1) % 4;
                self.windows.theme = theme::builtins()[self.theme_index].clone();
            }
        }
        used
    }
}
