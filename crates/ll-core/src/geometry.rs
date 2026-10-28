use serde::{Deserialize, Serialize};

/// Pixel coordinates in the ORIGINAL page image: origin top-left, y grows downward.
/// Models needing other spaces (e.g. 0..1000) convert at the model boundary only.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BBox { pub x0: f32, pub y0: f32, pub x1: f32, pub y1: f32 }

impl BBox {
    /// Normalizes swapped corners so a malformed box cannot exist.
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1) }
    }
    pub fn width(&self) -> f32 { self.x1 - self.x0 }
    pub fn height(&self) -> f32 { self.y1 - self.y0 }
    pub fn right(&self) -> f32 { self.x1 }
    pub fn bottom(&self) -> f32 { self.y1 }
    pub fn center_y(&self) -> f32 { (self.y0 + self.y1) / 2.0 }
    pub fn area(&self) -> f32 { self.width() * self.height() }
    pub fn center(&self) -> (f32, f32) { ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0) }

    pub fn union(&self, o: &BBox) -> BBox {
        BBox::new(self.x0.min(o.x0), self.y0.min(o.y0), self.x1.max(o.x1), self.y1.max(o.y1))
    }

    pub fn intersection_area(&self, o: &BBox) -> f32 {
        let w = (self.x1.min(o.x1) - self.x0.max(o.x0)).max(0.0);
        let h = (self.y1.min(o.y1) - self.y0.max(o.y0)).max(0.0);
        w * h
    }

    pub fn iou(&self, o: &BBox) -> f32 {
        let inter = self.intersection_area(o);
        let union = self.area() + o.area() - inter;
        if union <= 0.0 { 0.0 } else { inter / union }
    }

    /// LayoutLM-style normalized box in 0..=1000.
    pub fn to_norm_1000(&self, page_w: f32, page_h: f32) -> [u16; 4] {
        let f = |v: f32, d: f32| ((v / d) * 1000.0).clamp(0.0, 1000.0).round() as u16;
        [f(self.x0, page_w), f(self.y0, page_h), f(self.x1, page_w), f(self.y1, page_h)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn swapped_corners_are_normalized() {
        let b = BBox::new(10.0, 20.0, 0.0, 5.0);
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (0.0, 5.0, 10.0, 20.0));
    }
    #[test]
    fn iou_cases() {
        let a = BBox::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(a.iou(&a), 1.0);
        assert_eq!(a.iou(&BBox::new(20.0, 20.0, 30.0, 30.0)), 0.0);
        let half = BBox::new(5.0, 0.0, 15.0, 10.0);
        assert!((a.iou(&half) - 1.0 / 3.0).abs() < 1e-6);
    }
    #[test]
    fn normalization_clamps() {
        let b = BBox::new(0.0, 0.0, 2000.0, 500.0);
        assert_eq!(b.to_norm_1000(1000.0, 1000.0), [0, 0, 1000, 500]);
    }
}
