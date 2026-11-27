use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlaceholderError {
    #[error("unknown placeholder \"{{{0}}}\"")]
    Unknown(String),
    #[error("unclosed {{")]
    Unclosed,
    #[error("unmatched }}")]
    Unmatched,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Piece<P> {
    Text(String),
    Placeholder(P),
}

// Every brace is part of a placeholder or doubled, so that any other form can
// be given a meaning later without changing what a working text does.
pub(crate) fn pieces<P>(
    text: &str,
    placeholder: impl Fn(&str) -> Option<P>,
) -> Result<Vec<Piece<P>>, PlaceholderError> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        match c {
            '{' => {
                if let Some(after) = rest.strip_prefix('{') {
                    literal.push('{');
                    rest = after;
                    continue;
                }
                let (inside, after) = rest.split_once('}').ok_or(PlaceholderError::Unclosed)?;
                rest = after;
                let placeholder = placeholder(inside)
                    .ok_or_else(|| PlaceholderError::Unknown(inside.to_owned()))?;
                if !literal.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut literal)));
                }
                pieces.push(Piece::Placeholder(placeholder));
            }
            '}' => {
                rest = rest.strip_prefix('}').ok_or(PlaceholderError::Unmatched)?;
                literal.push('}');
            }
            _ => literal.push(c),
        }
    }
    if !literal.is_empty() {
        pieces.push(Piece::Text(literal));
    }
    Ok(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<Vec<Piece<()>>, PlaceholderError> {
        pieces(text, |name| (name == "x").then_some(()))
    }

    #[test]
    fn pieces_splits_the_text_at_its_placeholders() {
        let text = |text: &str| Piece::Text(text.to_owned());

        assert_eq!(
            read("a{x}{x}b"),
            Ok(vec![
                text("a"),
                Piece::Placeholder(()),
                Piece::Placeholder(()),
                text("b")
            ])
        );
    }

    #[test]
    fn pieces_reads_doubled_braces_as_text() {
        let text = |text: &str| Piece::Text(text.to_owned());

        assert_eq!(read("{{x}} }}{{"), Ok(vec![text("{x} }{")]));
        assert_eq!(
            read("{{{x}}}"),
            Ok(vec![text("{"), Piece::Placeholder(()), text("}")])
        );
    }

    #[test]
    fn pieces_rejects_braces_that_are_neither_placeholders_nor_doubled() {
        let unknown = |name: &str| PlaceholderError::Unknown(name.to_owned());
        let cases = [
            ("{}", unknown("")),
            ("{X}", unknown("X")),
            ("{ x }", unknown(" x ")),
            ("{a{x}", unknown("a{x")),
            ("{x", PlaceholderError::Unclosed),
            ("a}", PlaceholderError::Unmatched),
            ("{{x}", PlaceholderError::Unmatched),
            ("}{", PlaceholderError::Unmatched),
        ];
        for (text, error) in cases {
            assert_eq!(read(text), Err(error), "{text:?}");
        }
    }

    #[test]
    fn errors_quote_the_unknown_placeholder() {
        assert_eq!(
            read("{ y }").unwrap_err().to_string(),
            "unknown placeholder \"{ y }\""
        );
    }
}
