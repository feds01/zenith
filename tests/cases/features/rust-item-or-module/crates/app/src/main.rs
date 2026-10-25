use spice::{parse, write};

fn main() {
    let lines = parse("R1 a b 1k");
    println!("{}", write(&lines));
}
