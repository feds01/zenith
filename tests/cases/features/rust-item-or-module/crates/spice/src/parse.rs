pub fn parse(netlist: &str) -> Vec<&str> {
    netlist.lines().collect()
}

pub mod tokens {
    pub fn count(netlist: &str) -> usize {
        netlist.split_whitespace().count()
    }
}
