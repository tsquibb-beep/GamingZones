//! Rectangles in physical pixels, with the same edge convention as Win32 `RECT`:
//! `left`/`top` are inside, `right`/`bottom` are one past the last pixel.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self { left, top, right, bottom }
    }

    pub const fn from_xywh(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self::new(x, y, x + w, y + h)
    }

    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub const fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    pub fn union(&self, other: &Rect) -> Rect {
        Rect::new(
            self.left.min(other.left),
            self.top.min(other.top),
            self.right.max(other.right),
            self.bottom.max(other.bottom),
        )
    }

    /// The part both cover (empty, possibly inverted, if they don't overlap).
    pub fn intersect(&self, other: &Rect) -> Rect {
        Rect::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        )
    }

    pub const fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.left + dx, self.top + dy, self.right + dx, self.bottom + dy)
    }

    /// How far each edge of `inner` sits inside `self`. Used for the invisible resize
    /// borders (window rect vs DWM frame bounds) and the non-client frame (window vs client).
    pub const fn insets_to(&self, inner: &Rect) -> Insets {
        Insets {
            left: inner.left - self.left,
            top: inner.top - self.top,
            right: self.right - inner.right,
            bottom: self.bottom - inner.bottom,
        }
    }

    /// Grows the rect outwards by `insets` (the inverse of [`Rect::insets_to`]).
    pub const fn inflate(&self, insets: &Insets) -> Rect {
        Rect::new(
            self.left - insets.left,
            self.top - insets.top,
            self.right + insets.right,
            self.bottom + insets.bottom,
        )
    }
}

impl fmt::Display for Rect {
    /// `(l, t)-(r, b) W×H`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})-({}, {}) {}×{}", self.left, self.top, self.right, self.bottom, self.width(), self.height())
    }
}

/// Distance of each edge of an inner rect from the matching edge of an outer rect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl fmt::Display for Insets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "left {}, top {}, right {}, bottom {}", self.left, self.top, self.right, self.bottom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_and_contains_use_exclusive_right_bottom() {
        let r = Rect::from_xywh(10, 20, 1920, 1080);
        assert_eq!((r.width(), r.height()), (1920, 1080));
        assert!(r.contains(10, 20));
        assert!(r.contains(1929, 1099));
        assert!(!r.contains(1930, 20));
        assert!(!r.contains(10, 1100));
    }

    #[test]
    fn empty_for_zero_or_negative_size() {
        assert!(Rect::new(0, 0, 0, 10).is_empty());
        assert!(Rect::new(0, 0, 10, -1).is_empty());
        assert!(!Rect::new(0, 0, 1, 1).is_empty());
    }

    #[test]
    fn union_is_bounding_box() {
        let a = Rect::new(0, 0, 100, 100);
        let b = Rect::new(50, 200, 300, 250);
        assert_eq!(a.union(&b), Rect::new(0, 0, 300, 250));
    }

    #[test]
    fn insets_round_trip() {
        // A typical Windows 11 window at 125%: 9 px invisible border left/right/bottom, none on top.
        let window = Rect::new(991, 500, 2929, 1589);
        let frame = Rect::new(1000, 500, 2920, 1580);
        let insets = window.insets_to(&frame);
        assert_eq!(insets, Insets { left: 9, top: 0, right: 9, bottom: 9 });
        assert_eq!(frame.inflate(&insets), window);
    }

    #[test]
    fn display_format() {
        assert_eq!(Rect::from_xywh(-8, 0, 1920, 1080).to_string(), "(-8, 0)-(1912, 1080) 1920×1080");
    }
}
