mod blade;
mod sharpen;
mod stone;

fn main() {
    let mut blade = blade::Blade::new();
    let edge = sharpen::sharpen(&mut blade, &stone::progression());
    println!("{edge:?}");
}
