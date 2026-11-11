fn sharpen(blade: &mut Blade, passes: &[Pass]) -> Edge {
    let mut edge = blade.edge();
    for pass in passes {
        edge = pass.hone(edge);
        // deburr between passes: the strop's whole job
        edge.deburr(Direction::Away, 0.02);
    }
    edge.polish(Finish::Mirror)
}
