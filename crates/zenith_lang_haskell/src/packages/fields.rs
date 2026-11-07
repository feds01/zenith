//! The syntax of fields and sections that Cabal's files are written in: a
//! package's `.cabal` description, and each entry of a package database.
//!
//! A field is a name, a `:` and a value, which goes on over the lines after
//! it that are indented further. A section is a line of a name and what
//! follows it, such as `library`, `executable acme` or `if flag(dev)`, and
//! the lines after it that are indented further, which are its fields and
//! sections. A line that starts with `--` is a comment. Names are in any
//! case. The braces that may stand in for indentation are not read, as
//! packages are written without them.

/// A field or a section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Item {
    /// A field.
    Field(Field),

    /// A section.
    Section(Section),
}

/// A field: `exposed-modules: Acme.Core`, say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    /// Its name, in lowercase.
    pub(crate) name: String,

    /// Its value, with the lines that continue it, each trimmed, apart by
    /// `\n`.
    pub(crate) value: String,
}

/// A section: `executable acme` and its fields, say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Section {
    /// Its name, in lowercase, such as `executable` or `if`.
    pub(crate) name: String,

    /// What follows its name on its line, such as `acme` or `flag(dev)`.
    pub(crate) arguments: String,

    /// Its fields and sections.
    pub(crate) items: Vec<Item>,
}

/// The fields and sections of `text`, in order.
pub(crate) fn parse(text: &str) -> Vec<Item> {
    // The sections being read, outermost first, each with its indentation.
    // The file is the outermost, indented less than anything in it.
    let file = Section { name: String::new(), arguments: String::new(), items: Vec::new() };
    let mut open: Vec<(Option<usize>, Section)> = vec![(None, file)];
    // The indentation of the last field, while lines further in may continue it.
    let mut field: Option<usize> = None;

    for line in text.lines() {
        let content = line.trim();
        if content.is_empty() || content.starts_with("--") {
            continue;
        }
        let indent = line.len() - line.trim_start().len();

        // 1. A line indented further than the last field continues its value.
        if let Some(field_indent) = field
            && indent > field_indent
            && let Some((_, section)) = open.last_mut()
            && let Some(Item::Field(last)) = section.items.last_mut()
        {
            last.value.push('\n');
            last.value.push_str(content);
            continue;
        }

        // 2. A line indented no further than a section's own line ends it.
        close(&mut open, Some(indent));

        // 3. The line starts a field, or a section.
        let (_, section) = open.last_mut().expect("the file is never closed");
        match field_name(content) {
            Some((name, value)) => {
                let value = value.trim().to_owned();
                section.items.push(Item::Field(Field { name, value }));
                field = Some(indent);
            }
            None => {
                let (name, arguments) =
                    content.split_once(char::is_whitespace).unwrap_or((content, ""));
                let name = name.to_ascii_lowercase();
                let arguments = arguments.trim().to_owned();
                open.push((Some(indent), Section { name, arguments, items: Vec::new() }));
                field = None;
            }
        }
    }
    close(&mut open, None);
    open.pop().map(|(_, file)| file.items).unwrap_or_default()
}

/// Close the sections of `open` whose line is indented as far as `indent`,
/// or further, or every one but the file when `indent` is `None`, each into
/// the section around it.
fn close(open: &mut Vec<(Option<usize>, Section)>, indent: Option<usize>) {
    while open.len() > 1
        && open.last().is_some_and(|(own, _)| indent.is_none_or(|indent| *own >= Some(indent)))
    {
        let (_, section) = open.pop().expect("a section is open");
        let (_, outer) = open.last_mut().expect("the file is open");
        outer.items.push(Item::Section(section));
    }
}

/// The name, in lowercase, and the value of the field that `content` starts,
/// if it starts one: a name of letters, digits, `-` and `_`, then a `:`.
fn field_name(content: &str) -> Option<(String, &str)> {
    let length = content.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))?;
    let (name, rest) = content.split_at(length);
    let value = rest.trim_start().strip_prefix(':')?;
    (!name.is_empty()).then(|| (name.to_ascii_lowercase(), value))
}

/// The values of the fields named `name` among `items`, in order.
pub(crate) fn fields<'a, 'n>(
    items: &'a [Item],
    name: &'n str,
) -> impl Iterator<Item = &'a str> + use<'a, 'n> {
    items.iter().filter_map(move |item| match item {
        Item::Field(field) if field.name == name => Some(field.value.as_str()),
        _ => None,
    })
}

/// The sections among `items`, in order.
pub(crate) fn sections(items: &[Item]) -> impl Iterator<Item = &Section> {
    items.iter().filter_map(|item| match item {
        Item::Section(section) => Some(section),
        Item::Field(_) => None,
    })
}

/// The words of a list, such as of modules or directories, apart by commas
/// or whitespace. A word in double quotes, as a directory with a space in its
/// name is written, is taken without its quotes.
pub(crate) fn words(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut rest = value;
    loop {
        rest = rest.trim_start_matches(|c: char| c == ',' || c.is_whitespace());
        if rest.is_empty() {
            return words;
        }
        let (word, after) = match rest.strip_prefix('"') {
            Some(quoted) => quoted.split_once('"').unwrap_or((quoted, "")),
            None => rest
                .split_at(rest.find(|c: char| c == ',' || c.is_whitespace()).unwrap_or(rest.len())),
        };
        words.push(word.to_owned());
        rest = after;
    }
}
