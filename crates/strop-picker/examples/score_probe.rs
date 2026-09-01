fn main() {
    println!("{:?}", strop_picker::fuzzy_score("ren", "src/render.rs"));
    println!("{:?}", strop_picker::fuzzy_score("ren", "different.txt"));
    println!("{:?}", strop_picker::fuzzy_score("rr", "src/render.rs"));
}
