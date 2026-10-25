use crate::units::Millimetres;

pub struct Square {
    pub side: Millimetres,
}

impl Square {
    pub fn area(&self) -> Millimetres {
        self.side * self.side
    }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::unit_square;

    #[test]
    fn a_unit_square_has_an_area_of_one() {
        assert_eq!(unit_square().area(), 1.0);
    }
}
