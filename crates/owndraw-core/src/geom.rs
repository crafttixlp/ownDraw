//! Kleine Geometrie-Helfer (achsenparalleles Rechteck in Weltkoordinaten).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl RectF {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        RectF {
            left,
            top,
            right,
            bottom,
        }
    }
    pub fn width(&self) -> f32 {
        self.right - self.left
    }
    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }
    pub fn center_x(&self) -> f32 {
        (self.left + self.right) * 0.5
    }
    pub fn center_y(&self) -> f32 {
        (self.top + self.bottom) * 0.5
    }
    pub fn union(&mut self, o: &RectF) {
        self.left = self.left.min(o.left);
        self.top = self.top.min(o.top);
        self.right = self.right.max(o.right);
        self.bottom = self.bottom.max(o.bottom);
    }
    /// Verkleinert das Rechteck um d (negativ = vergroessern).
    pub fn inset(&mut self, dx: f32, dy: f32) {
        self.left += dx;
        self.top += dy;
        self.right -= dx;
        self.bottom -= dy;
    }
    pub fn intersects(a: &RectF, b: &RectF) -> bool {
        a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
    }
    pub fn contains_pt(&self, x: f32, y: f32) -> bool {
        self.left < self.right
            && self.top < self.bottom
            && x >= self.left
            && x < self.right
            && y >= self.top
            && y < self.bottom
    }
    pub fn contains(&self, o: &RectF) -> bool {
        self.left < self.right
            && self.top < self.bottom
            && self.left <= o.left
            && self.top <= o.top
            && self.right >= o.right
            && self.bottom >= o.bottom
    }
}

pub fn hypot(a: f32, b: f32) -> f32 {
    (a as f64).hypot(b as f64) as f32
}
