use libseatgeist::SeatgeistError;

use super::AtspiRef;

pub(super) type Result<T> = std::result::Result<T, SeatgeistError>;
const MAX_CACHE_ITEMS: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CacheItem {
    pub(super) node: AtspiRef,
    pub(super) parent: AtspiRef,
    pub(super) interfaces: Vec<String>,
    pub(super) name: String,
    pub(super) role: u32,
    pub(super) states: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    Quoted(String),
}

pub(super) fn parse_items(output: &str) -> Result<Vec<CacheItem>> {
    let tokens = tokenize(output)?;
    let mut cursor = Cursor::new(&tokens);
    let signature = cursor.word()?;
    if signature != "a((so)(so)(so)iiassusau)" {
        return Err(parse_error("unexpected AT-SPI cache signature"));
    }
    let count = cursor.usize()?;
    if count > MAX_CACHE_ITEMS {
        return Err(parse_error("AT-SPI cache item count exceeds safety limit"));
    }
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let node = cursor.object_ref()?;
        let _application = cursor.object_ref()?;
        let parent = cursor.object_ref()?;
        let _index_in_parent = cursor.i32()?;
        let _child_count = cursor.i32()?;
        let interface_count = cursor.usize()?;
        let mut interfaces = Vec::with_capacity(interface_count);
        for _ in 0..interface_count {
            interfaces.push(cursor.quoted()?.to_string());
        }
        let name = cursor.quoted()?.to_string();
        let role = cursor.u32()?;
        let _description = cursor.quoted()?;
        let state_count = cursor.usize()?;
        let mut states = Vec::with_capacity(state_count);
        for _ in 0..state_count {
            states.push(cursor.u32()?);
        }
        items.push(CacheItem {
            node,
            parent,
            interfaces,
            name,
            role,
            states,
        });
    }
    if cursor.remaining() != 0 {
        return Err(parse_error("trailing tokens in AT-SPI cache response"));
    }
    Ok(items)
}

struct Cursor<'a> {
    tokens: &'a [Token],
    index: usize,
}

impl<'a> Cursor<'a> {
    const fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, index: 0 }
    }

    fn remaining(&self) -> usize {
        self.tokens.len().saturating_sub(self.index)
    }

    fn next(&mut self) -> Result<&'a Token> {
        let token = self
            .tokens
            .get(self.index)
            .ok_or_else(|| parse_error("truncated AT-SPI cache response"))?;
        self.index += 1;
        Ok(token)
    }

    fn word(&mut self) -> Result<&'a str> {
        match self.next()? {
            Token::Word(value) => Ok(value),
            Token::Quoted(_) => Err(parse_error("expected unquoted AT-SPI cache token")),
        }
    }

    fn quoted(&mut self) -> Result<&'a str> {
        match self.next()? {
            Token::Quoted(value) => Ok(value),
            Token::Word(_) => Err(parse_error("expected quoted AT-SPI cache token")),
        }
    }

    fn usize(&mut self) -> Result<usize> {
        self.word()?
            .parse()
            .map_err(|_| parse_error("invalid AT-SPI cache count"))
    }

    fn i32(&mut self) -> Result<i32> {
        self.word()?
            .parse()
            .map_err(|_| parse_error("invalid AT-SPI cache integer"))
    }

    fn u32(&mut self) -> Result<u32> {
        self.word()?
            .parse()
            .map_err(|_| parse_error("invalid AT-SPI cache unsigned integer"))
    }

    fn object_ref(&mut self) -> Result<AtspiRef> {
        Ok(AtspiRef {
            service: self.quoted()?.to_string(),
            path: self.quoted()?.to_string(),
        })
    }
}

fn tokenize(input: &str) -> Result<Vec<Token>> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        if bytes[index] != b'"' {
            let start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            let value = std::str::from_utf8(&bytes[start..index])
                .map_err(|_| parse_error("non-UTF-8 AT-SPI cache token"))?;
            tokens.push(Token::Word(value.to_string()));
            continue;
        }

        index += 1;
        let mut value = Vec::new();
        let mut closed = false;
        while index < bytes.len() {
            match bytes[index] {
                b'"' => {
                    index += 1;
                    closed = true;
                    break;
                }
                b'\\' => {
                    index += 1;
                    if index >= bytes.len() {
                        return Err(parse_error("truncated AT-SPI cache escape"));
                    }
                    if index + 2 < bytes.len()
                        && bytes[index..index + 3].iter().all(u8::is_ascii_digit)
                    {
                        let octal = std::str::from_utf8(&bytes[index..index + 3])
                            .map_err(|_| parse_error("invalid AT-SPI cache escape"))?;
                        value.push(
                            u8::from_str_radix(octal, 8)
                                .map_err(|_| parse_error("invalid AT-SPI cache octal escape"))?,
                        );
                        index += 3;
                    } else {
                        value.push(bytes[index]);
                        index += 1;
                    }
                }
                byte => {
                    value.push(byte);
                    index += 1;
                }
            }
        }
        if !closed {
            return Err(parse_error("unterminated AT-SPI cache string"));
        }
        tokens.push(Token::Quoted(
            String::from_utf8(value).map_err(|_| parse_error("non-UTF-8 AT-SPI cache string"))?,
        ));
    }
    Ok(tokens)
}

fn parse_error(message: &str) -> SeatgeistError {
    SeatgeistError::InvalidRequest(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bulk_cache_items() {
        let output = concat!(
            "a((so)(so)(so)iiassusau) 1 ",
            "\":1.42\" \"/org/a11y/atspi/accessible/7\" ",
            "\":1.42\" \"/org/a11y/atspi/accessible/root\" ",
            "\":1.42\" \"/org/a11y/atspi/accessible/2\" ",
            "0 1 3 \"org.a11y.atspi.Accessible\" ",
            "\"org.a11y.atspi.Action\" \"org.a11y.atspi.Component\" ",
            "\"Seatgeist Step 12 Button\" 43 \"\" 2 1091045632 0"
        );

        let items = parse_items(output).expect("cache response parses");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].node.service, ":1.42");
        assert_eq!(items[0].node.path, "/org/a11y/atspi/accessible/7");
        assert_eq!(items[0].parent.path, "/org/a11y/atspi/accessible/2");
        assert_eq!(items[0].name, "Seatgeist Step 12 Button");
        assert_eq!(items[0].role, 43);
        assert_eq!(items[0].states, vec![1_091_045_632, 0]);
    }

    #[test]
    fn rejects_truncated_cache_items() {
        let error = parse_items("a((so)(so)(so)iiassusau) 1 \"only-service\"")
            .expect_err("truncated response fails");
        assert!(error.to_string().contains("truncated"));
    }

    #[test]
    fn rejects_oversized_cache_before_allocation() {
        let output = format!("a((so)(so)(so)iiassusau) {}", MAX_CACHE_ITEMS + 1);
        let error = parse_items(&output).expect_err("oversized response fails");
        assert!(error.to_string().contains("safety limit"));
    }
}
