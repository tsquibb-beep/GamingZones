//! Readable names for window style bits (debug output only).

/// Style bits, composites first so they win over their parts.
const STYLES: &[(u32, &str)] = &[
    (0x00C0_0000, "CAPTION"),
    (0x8000_0000, "POPUP"),
    (0x4000_0000, "CHILD"),
    (0x2000_0000, "MINIMIZE"),
    (0x1000_0000, "VISIBLE"),
    (0x0800_0000, "DISABLED"),
    (0x0400_0000, "CLIPSIBLINGS"),
    (0x0200_0000, "CLIPCHILDREN"),
    (0x0100_0000, "MAXIMIZE"),
    (0x0080_0000, "BORDER"),
    (0x0040_0000, "DLGFRAME"),
    (0x0020_0000, "VSCROLL"),
    (0x0010_0000, "HSCROLL"),
    (0x0008_0000, "SYSMENU"),
    (0x0004_0000, "THICKFRAME"),
    // On top-level windows these two bits mean the min/max buttons (GROUP/TABSTOP on controls).
    (0x0002_0000, "MINIMIZEBOX"),
    (0x0001_0000, "MAXIMIZEBOX"),
];

const EX_STYLES: &[(u32, &str)] = &[
    (0x0000_0001, "DLGMODALFRAME"),
    (0x0000_0004, "NOPARENTNOTIFY"),
    (0x0000_0008, "TOPMOST"),
    (0x0000_0010, "ACCEPTFILES"),
    (0x0000_0020, "TRANSPARENT"),
    (0x0000_0040, "MDICHILD"),
    (0x0000_0080, "TOOLWINDOW"),
    (0x0000_0100, "WINDOWEDGE"),
    (0x0000_0200, "CLIENTEDGE"),
    (0x0000_0400, "CONTEXTHELP"),
    (0x0000_1000, "RIGHT"),
    (0x0000_2000, "RTLREADING"),
    (0x0000_4000, "LEFTSCROLLBAR"),
    (0x0001_0000, "CONTROLPARENT"),
    (0x0002_0000, "STATICEDGE"),
    (0x0004_0000, "APPWINDOW"),
    (0x0008_0000, "LAYERED"),
    (0x0010_0000, "NOINHERITLAYOUT"),
    (0x0020_0000, "NOREDIRECTIONBITMAP"),
    (0x0040_0000, "LAYOUTRTL"),
    (0x0200_0000, "COMPOSITED"),
    (0x0800_0000, "NOACTIVATE"),
];

fn describe(value: u32, table: &[(u32, &str)]) -> String {
    let mut rest = value;
    let mut names = Vec::new();
    for &(bits, name) in table {
        if rest & bits == bits {
            names.push(name.to_string());
            rest &= !bits;
        }
    }
    if rest != 0 {
        names.push(format!("{rest:#X}"));
    }
    format!("{value:#010X} [{}]", names.join(" "))
}

/// e.g. `0x16CF0000 [CAPTION VISIBLE CLIPSIBLINGS ... MAXIMIZEBOX]`
pub fn describe_style(style: u32) -> String {
    describe(style, STYLES)
}

pub fn describe_ex_style(ex_style: u32) -> String {
    describe(ex_style, EX_STYLES)
}

/// Raw style bits the apps need.
pub mod ws {
    pub const OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
    pub const POPUP: u32 = 0x8000_0000;
    pub const CHILD: u32 = 0x4000_0000;
    pub const VISIBLE: u32 = 0x1000_0000;
    pub const CLIPCHILDREN: u32 = 0x0200_0000;
    pub const CAPTION: u32 = 0x00C0_0000;
    pub const SYSMENU: u32 = 0x0008_0000;
    pub const THICKFRAME: u32 = 0x0004_0000;
    pub const MINIMIZEBOX: u32 = 0x0002_0000;
    pub const MAXIMIZEBOX: u32 = 0x0001_0000;
    pub const TABSTOP: u32 = 0x0001_0000;
    pub const EX_DLGMODALFRAME: u32 = 0x0000_0001;
    pub const EX_TOPMOST: u32 = 0x0000_0008;
    pub const EX_TRANSPARENT: u32 = 0x0000_0020;
    pub const EX_TOOLWINDOW: u32 = 0x0000_0080;
    pub const EX_LAYERED: u32 = 0x0008_0000;
    pub const EX_NOACTIVATE: u32 = 0x0800_0000;
    pub const EX_WINDOWEDGE: u32 = 0x0000_0100;
    pub const EX_CLIENTEDGE: u32 = 0x0000_0200;
    pub const EX_STATICEDGE: u32 = 0x0002_0000;
    pub const EX_CONTROLPARENT: u32 = 0x0001_0000;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapped_window() {
        assert_eq!(
            describe_style(0x14CF_0000),
            "0x14CF0000 [CAPTION VISIBLE CLIPSIBLINGS SYSMENU THICKFRAME MINIMIZEBOX MAXIMIZEBOX]"
        );
    }

    #[test]
    fn border_without_dlgframe_and_unknown_bits() {
        assert_eq!(describe_style(0x0080_0001), "0x00800001 [BORDER 0x1]");
        assert_eq!(describe_ex_style(0), "0x00000000 []");
        assert_eq!(describe_ex_style(0x0000_0188), "0x00000188 [TOPMOST TOOLWINDOW WINDOWEDGE]");
    }
}
