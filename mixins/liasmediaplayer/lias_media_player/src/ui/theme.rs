//! Built-in Java v3 palettes, including partial addon overrides. Colors are ARGB.
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThemeRole {
    WindowBg,
    TitleBarBg,
    ControlBarBg,
    Placeholder,
    PanelBg,
    ListBg,
    RowBg,
    RowHover,
    RowSelected,
    FieldBg,
    ChipBg,
    ChipHover,
    BannerBg,
    Border,
    BorderFocused,
    Track,
    Fill,
    Knob,
    ScrollTrack,
    ScrollThumb,
    Text,
    TextSubtle,
    TextDim,
    Icon,
    IconHover,
    IconActive,
    IconDisabled,
    Danger,
    Muted,
    Spinner,
}
#[derive(Clone, Debug)]
pub struct ThemeSpec {
    pub id: String,
    pub name: String,
    pub colors: BTreeMap<ThemeRole, u32>,
}
impl ThemeSpec {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            colors: BTreeMap::new(),
        }
    }
    pub fn with(mut self, role: ThemeRole, color: u32) -> Self {
        self.colors.insert(role, color);
        self
    }
    pub fn color(&self, role: ThemeRole) -> u32 {
        self.colors
            .get(&role)
            .copied()
            .unwrap_or_else(|| dark_color(role))
    }
}
fn dark_color(role: ThemeRole) -> u32 {
    match role {
        ThemeRole::WindowBg => 0xF0141821,
        ThemeRole::TitleBarBg => 0xFF202838,
        ThemeRole::ControlBarBg => 0xFF1A2230,
        ThemeRole::Placeholder => 0xFF000000,
        ThemeRole::PanelBg => 0xF01A2230,
        ThemeRole::ListBg => 0xE0111620,
        ThemeRole::RowBg => 0xFF253043,
        ThemeRole::RowHover => 0xFF34435B,
        ThemeRole::RowSelected => 0xFF34597D,
        ThemeRole::FieldBg => 0xFF000000,
        ThemeRole::ChipBg => 0xD0181818,
        ThemeRole::ChipHover => 0xF0303030,
        ThemeRole::BannerBg => 0xF01A2230,
        ThemeRole::Border => 0xFF3C4A60,
        ThemeRole::BorderFocused => 0xFF69BDFF,
        ThemeRole::Track => 0xFF4A4A4A,
        ThemeRole::Fill => 0xFF69BDFF,
        ThemeRole::Knob => 0xFFFFFFFF,
        ThemeRole::ScrollTrack => 0xFF333333,
        ThemeRole::ScrollThumb => 0xFF6A6A6A,
        ThemeRole::Text => 0xFFFFFFFF,
        ThemeRole::TextSubtle => 0xFFBAC5D6,
        ThemeRole::TextDim => 0xFF93A1B7,
        ThemeRole::Icon => 0xFFE0E0E0,
        ThemeRole::IconHover => 0xFFFFD23F,
        ThemeRole::IconActive => 0xFF69BDFF,
        ThemeRole::IconDisabled => 0xFF555555,
        ThemeRole::Danger => 0xFFFF6B6B,
        ThemeRole::Muted => 0xFFFF5555,
        ThemeRole::Spinner => 0xFFE0E0E0,
    }
}
pub fn dark() -> ThemeSpec {
    ThemeSpec::new("liasmediaplayer:dark", "Dark")
        .with(ThemeRole::WindowBg, 0xF0141821)
        .with(ThemeRole::TitleBarBg, 0xFF202838)
        .with(ThemeRole::ControlBarBg, 0xFF1A2230)
        .with(ThemeRole::Placeholder, 0xFF000000)
        .with(ThemeRole::PanelBg, 0xF01A2230)
        .with(ThemeRole::ListBg, 0xE0111620)
        .with(ThemeRole::RowBg, 0xFF253043)
        .with(ThemeRole::RowHover, 0xFF34435B)
        .with(ThemeRole::RowSelected, 0xFF34597D)
        .with(ThemeRole::FieldBg, 0xFF000000)
        .with(ThemeRole::ChipBg, 0xD0181818)
        .with(ThemeRole::ChipHover, 0xF0303030)
        .with(ThemeRole::BannerBg, 0xF01A2230)
        .with(ThemeRole::Border, 0xFF3C4A60)
        .with(ThemeRole::BorderFocused, 0xFF69BDFF)
        .with(ThemeRole::Track, 0xFF4A4A4A)
        .with(ThemeRole::Fill, 0xFF69BDFF)
        .with(ThemeRole::Knob, 0xFFFFFFFF)
        .with(ThemeRole::ScrollTrack, 0xFF333333)
        .with(ThemeRole::ScrollThumb, 0xFF6A6A6A)
        .with(ThemeRole::Text, 0xFFFFFFFF)
        .with(ThemeRole::TextSubtle, 0xFFBAC5D6)
        .with(ThemeRole::TextDim, 0xFF93A1B7)
        .with(ThemeRole::Icon, 0xFFE0E0E0)
        .with(ThemeRole::IconHover, 0xFFFFD23F)
        .with(ThemeRole::IconActive, 0xFF69BDFF)
        .with(ThemeRole::IconDisabled, 0xFF555555)
        .with(ThemeRole::Danger, 0xFFFF6B6B)
        .with(ThemeRole::Muted, 0xFFFF5555)
        .with(ThemeRole::Spinner, 0xFFE0E0E0)
}
pub fn contrast() -> ThemeSpec {
    ThemeSpec::new("liasmediaplayer:contrast", "High Contrast")
        .with(ThemeRole::WindowBg, 0xFF000000)
        .with(ThemeRole::TitleBarBg, 0xFF000000)
        .with(ThemeRole::ControlBarBg, 0xFF000000)
        .with(ThemeRole::PanelBg, 0xFF000000)
        .with(ThemeRole::ListBg, 0xFF000000)
        .with(ThemeRole::RowBg, 0xFF000000)
        .with(ThemeRole::RowHover, 0xFF303030)
        .with(ThemeRole::RowSelected, 0xFF0057B8)
        .with(ThemeRole::ChipBg, 0xFF000000)
        .with(ThemeRole::ChipHover, 0xFF444444)
        .with(ThemeRole::BannerBg, 0xFF000000)
        .with(ThemeRole::Border, 0xFFFFFFFF)
        .with(ThemeRole::BorderFocused, 0xFF00E5FF)
        .with(ThemeRole::Track, 0xFF666666)
        .with(ThemeRole::Fill, 0xFF00E5FF)
        .with(ThemeRole::ScrollThumb, 0xFFBAC5D6)
        .with(ThemeRole::TextSubtle, 0xFFDDDDDD)
        .with(ThemeRole::TextDim, 0xFFBBBBBB)
        .with(ThemeRole::Icon, 0xFFFFFFFF)
        .with(ThemeRole::IconHover, 0xFFFFE000)
        .with(ThemeRole::IconActive, 0xFF00E5FF)
        .with(ThemeRole::IconDisabled, 0xFF666666)
        .with(ThemeRole::Danger, 0xFFFF5555)
        .with(ThemeRole::Muted, 0xFFFF3333)
}
pub fn minecraft() -> ThemeSpec {
    ThemeSpec::new("liasmediaplayer:minecraft", "Minecraft")
        .with(ThemeRole::WindowBg, 0xE0111620)
        .with(ThemeRole::TitleBarBg, 0xE0202020)
        .with(ThemeRole::ControlBarBg, 0xE0202020)
        .with(ThemeRole::PanelBg, 0xF0141821)
        .with(ThemeRole::ListBg, 0xE0111620)
        .with(ThemeRole::RowBg, 0xFF2B2B2B)
        .with(ThemeRole::RowHover, 0xFF3C3C3C)
        .with(ThemeRole::RowSelected, 0xFF505050)
        .with(ThemeRole::ChipBg, 0xE0111620)
        .with(ThemeRole::ChipHover, 0xE0404040)
        .with(ThemeRole::BannerBg, 0xF0141821)
        .with(ThemeRole::Border, 0xFF6C6C6C)
        .with(ThemeRole::BorderFocused, 0xFFFFFFFF)
        .with(ThemeRole::Track, 0xFF6C6C6C)
        .with(ThemeRole::Fill, 0xFF55FF55)
        .with(ThemeRole::ScrollTrack, 0xFF000000)
        .with(ThemeRole::ScrollThumb, 0xFF8B8B8B)
        .with(ThemeRole::TextSubtle, 0xFFA0A0A0)
        .with(ThemeRole::TextDim, 0xFF808080)
        .with(ThemeRole::IconHover, 0xFFFFFF55)
        .with(ThemeRole::IconActive, 0xFF55FF55)
        .with(ThemeRole::IconDisabled, 0xFF6C6C6C)
        .with(ThemeRole::Danger, 0xFFFF5555)
}
pub fn light() -> ThemeSpec {
    ThemeSpec::new("liasmediaplayer:light", "Light")
        .with(ThemeRole::WindowBg, 0xF0F2F2F2)
        .with(ThemeRole::TitleBarBg, 0xFFE2E2E2)
        .with(ThemeRole::ControlBarBg, 0xFFE8E8E8)
        .with(ThemeRole::Placeholder, 0xFFD8D8D8)
        .with(ThemeRole::PanelBg, 0xF0EDEDED)
        .with(ThemeRole::ListBg, 0xE0F2F2F2)
        .with(ThemeRole::RowBg, 0xFFE4E4E4)
        .with(ThemeRole::RowHover, 0xFFD2DAEA)
        .with(ThemeRole::RowSelected, 0xFFAFC6EE)
        .with(ThemeRole::FieldBg, 0xFFFFFFFF)
        .with(ThemeRole::ChipBg, 0xE0E8E8E8)
        .with(ThemeRole::ChipHover, 0xFFFFFFFF)
        .with(ThemeRole::BannerBg, 0xF0EDEDED)
        .with(ThemeRole::Border, 0x60000000)
        .with(ThemeRole::BorderFocused, 0xFF1B72C4)
        .with(ThemeRole::Track, 0xFFBEBEBE)
        .with(ThemeRole::Fill, 0xFF1B72C4)
        .with(ThemeRole::Knob, 0xFF253043)
        .with(ThemeRole::ScrollTrack, 0xFFD0D0D0)
        .with(ThemeRole::ScrollThumb, 0xFF909090)
        .with(ThemeRole::Text, 0xFF161616)
        .with(ThemeRole::TextSubtle, 0xFF4A4A4A)
        .with(ThemeRole::TextDim, 0xFF7A7A7A)
        .with(ThemeRole::Icon, 0xFF303030)
        .with(ThemeRole::IconHover, 0xFFB07000)
        .with(ThemeRole::IconActive, 0xFF1B72C4)
        .with(ThemeRole::IconDisabled, 0xFFB4B4B4)
        .with(ThemeRole::Danger, 0xFFC62828)
        .with(ThemeRole::Muted, 0xFFD32F2F)
        .with(ThemeRole::Spinner, 0xFF303030)
}
pub fn builtins() -> [ThemeSpec; 4] {
    [dark(), contrast(), minecraft(), light()]
}
