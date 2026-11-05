//! Tiny pixel icons, so transport controls do not depend on font glyph coverage.
use crate::platform::{Canvas, Rect};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Previous,
    Next,
    Back,
    Forward,
    Speaker,
    Repeat,
    Shuffle,
    Queue,
    More,
    Close,
    Pin,
    Expand,
    Restore,
    Minimize,
}
pub fn draw(c: &mut dyn Canvas, r: Rect, icon: Icon, color: u32) {
    let pattern: &[&str] = match icon {
        Icon::Play => &[
            "##      ", "####    ", "######  ", "########", "######  ", "####    ", "##      ",
        ],
        Icon::Pause => &[
            "###  ###", "###  ###", "###  ###", "###  ###", "###  ###", "###  ###", "###  ###",
        ],
        Icon::Previous => &[
            "##    ##", "##  ####", "########", "########", "########", "##  ####", "##    ##",
        ],
        Icon::Next => &[
            "##    ##", "####  ##", "########", "########", "########", "####  ##", "##    ##",
        ],
        Icon::Back => &[
            "   #   #", "  ##  ##", " ### ###", "########", " ### ###", "  ##  ##", "   #   #",
        ],
        Icon::Forward => &[
            "#   #   ", "##  ##  ", "### ### ", "########", "### ### ", "##  ##  ", "#   #   ",
        ],
        Icon::Speaker => &[
            "   #  # ", "  ##   #", "#### # #", "#### # #", "#### # #", "  ##   #", "   #  # ",
        ],
        Icon::Repeat => &[
            "     #  ", " ###### ", "#    # #", "#      #", "# #    #", " ###### ", "  #     ",
        ],
        Icon::Shuffle => &[
            "##    # ", "  #  ###", "   ## # ", "   ##   ", "   ## # ", "  #  ###", "##    # ",
        ],
        Icon::Queue => &[
            "## #####", "        ", "## #####", "        ", "## #####", "        ", "## #####",
        ],
        Icon::More => &[
            "        ", "        ", "## ## ##", "## ## ##", "        ", "        ", "        ",
        ],
        Icon::Close => &[
            "##    ##", " ##  ## ", "  ####  ", "   ##   ", "  ####  ", " ##  ## ", "##    ##",
        ],
        Icon::Pin => &[
            "  ####  ", "   ##   ", "   ##   ", "  ####  ", " ###### ", "   #    ", "   #    ",
        ],
        Icon::Expand => &[
            "###  ###", "#      #", "#      #", "        ", "#      #", "#      #", "###  ###",
        ],
        Icon::Restore => &[
            "  ######", "  #    #", "###### #", "#    ###", "#    #  ", "#    #  ", "######  ",
        ],
        Icon::Minimize => &[
            "        ", "        ", "        ", "        ", "        ", "########", "########",
        ],
    };
    let x = (r.x + (r.w - 8.) / 2.).floor();
    let y = (r.y + (r.h - 7.) / 2.).floor();
    for (dy, row) in pattern.iter().enumerate() {
        for (dx, ch) in row.bytes().enumerate() {
            if ch == b'#' {
                c.fill(Rect::new(x + dx as f32, y + dy as f32, 1., 1.), color);
            }
        }
    }
}
