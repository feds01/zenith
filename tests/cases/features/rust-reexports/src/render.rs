use crate::{Document, Layout};

pub fn render(document: &Document) -> &Layout {
    &document.layout
}
