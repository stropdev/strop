pub struct Blade {
    edge: Edge,
}

impl Edge {
    /// Strokes the burr off the edge.
    pub fn deburr(&mut self, direction: Direction, amount: f32) {
        self.burr = (self.burr - amount).max(0.0);
        self.last_stroke = direction;
    }
}
