use std::{fs, thread};

pub fn load(path: &str) -> String {
    let path = path.to_owned();
    thread::spawn(move || fs::read_to_string(path).unwrap_or_default()).join().unwrap()
}
