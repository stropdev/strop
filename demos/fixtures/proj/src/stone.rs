//! The stones a blade is taken through, coarsest first.

pub fn progression() -> Vec<Pass> {
    vec![
        Pass::new(Grit::Coarse, 12),
        Pass::new(Grit::Coarse, 8),
        Pass::new(Grit::Coarse, 4),
    ]
}
