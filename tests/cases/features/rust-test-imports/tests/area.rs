use geometry::shapes::Square;

#[test]
fn squares_have_areas() {
    assert_eq!(Square { side: 2.0 }.area(), 4.0);
}
