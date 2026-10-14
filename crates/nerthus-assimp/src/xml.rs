//! pugixml 1.13's parser with `parse_full` (CDATA, escapes, whitespace
//! conversion in attributes, end-of-line normalisation, comments,
//! declarations, doctype, processing instructions), as Assimp's
//! `XmlParser` runs it over a file plus one NUL: whitespace-only text is
//! dropped, only the five named entities and numeric references are
//! expanded, a NUL ends the document, and several top-level elements are
//! fine.

use crate::parsing::at;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Element,
    Pcdata,
    Cdata,
    Comment,
    Pi,
    Declaration,
    Doctype,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub name: Vec<u8>,
    pub value: Vec<u8>,
    pub attributes: Vec<(Vec<u8>, Vec<u8>)>,
    pub children: Vec<Node>,
}

impl Node {
    fn new(kind: NodeKind) -> Self {
        Node { kind, name: Vec::new(), value: Vec::new(), attributes: Vec::new(), children: Vec::new() }
    }

    /// `name()` (empty for text, comments and the document).
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    pub fn is_element(&self) -> bool {
        self.kind == NodeKind::Element
    }

    /// `child(name)`: the first child with the name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name.as_bytes())
    }

    /// `attribute(name)`: the first attribute with the name.
    pub fn attribute(&self, name: &str) -> Option<&[u8]> {
        self.attributes.iter().find(|(key, _)| key.as_slice() == name.as_bytes()).map(|(_, value)| value.as_slice())
    }

    /// `text().as_string()`: the first PCDATA or CDATA child's value.
    pub fn text(&self) -> &[u8] {
        self.children.iter().find(|child| matches!(child.kind, NodeKind::Pcdata | NodeKind::Cdata)).map_or(&[], |child| child.value.as_slice())
    }

    /// `find_node` with a name predicate: the first node below this one, in
    /// document order, whatever its kind, with the name.
    pub fn find_node(&self, name: &str) -> Option<&Node> {
        for child in &self.children {
            if child.name == name.as_bytes() {
                return Some(child);
            }
            if let Some(found) = child.find_node(name) {
                return Some(found);
            }
        }
        None
    }

    /// `XmlNodeIterator` in pre-order: every descendant element.
    pub fn descendants(&self) -> Vec<&Node> {
        let mut nodes = Vec::new();
        for child in &self.children {
            child.collect_pre_order(&mut nodes);
        }
        nodes
    }

    fn collect_pre_order<'a>(&'a self, nodes: &mut Vec<&'a Node>) {
        if self.kind == NodeKind::Element {
            nodes.push(self);
        }
        for child in &self.children {
            child.collect_pre_order(nodes);
        }
    }
}

const CT_PARSE_PCDATA: u8 = 1;
const CT_PARSE_ATTR_WS: u8 = 4;
const CT_SPACE: u8 = 8;
const CT_PARSE_CDATA: u8 = 16;
const CT_PARSE_COMMENT: u8 = 32;
const CT_SYMBOL: u8 = 64;
const CT_START_SYMBOL: u8 = 128;

/// `chartype_table`.
const CHARTYPE: [u8; 256] = {
    let mut table = [0u8; 256];
    let head: [u8; 128] = [
        55, 0, 0, 0, 0, 0, 0, 0, 0, 12, 12, 0, 0, 63, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
        8, 0, 6, 0, 0, 0, 7, 6, 0, 0, 0, 0, 0, 96, 64, 0, //
        64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 192, 0, 1, 0, 48, 0, //
        0, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, //
        192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 0, 0, 16, 0, 192, //
        0, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, //
        192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 0, 0, 0, 0, 0,
    ];
    let mut index = 0;
    while index < 256 {
        table[index] = if index < 128 { head[index] } else { 192 };
        index += 1;
    }
    table
};

fn is(byte: u8, class: u8) -> bool {
    CHARTYPE[byte as usize] & class != 0
}

/// The parser state: the buffer (with its NUL) and a cursor; `endch` is
/// always 0 for Assimp's NUL-terminated buffers.
struct Parser {
    buffer: Vec<u8>,
}

#[derive(Debug)]
pub struct XmlError;

type Parse<T> = Result<T, XmlError>;

impl Parser {
    fn at(&self, index: usize) -> u8 {
        self.buffer.get(index).copied().unwrap_or(0)
    }

    fn skip_whitespace(&self, mut index: usize) -> usize {
        while is(self.at(index), CT_SPACE) {
            index += 1;
        }
        index
    }

    /// `utf8_writer::any`.
    fn push_codepoint(output: &mut Vec<u8>, codepoint: u32) {
        if codepoint < 0x80 {
            output.push(codepoint as u8);
        } else if codepoint < 0x800 {
            output.push((0xC0 | (codepoint >> 6)) as u8);
            output.push((0x80 | (codepoint & 0x3F)) as u8);
        } else if codepoint < 0x10000 {
            output.push((0xE0 | (codepoint >> 12)) as u8);
            output.push((0x80 | ((codepoint >> 6) & 0x3F)) as u8);
            output.push((0x80 | (codepoint & 0x3F)) as u8);
        } else {
            output.push((0xF0 | (codepoint >> 18)) as u8);
            output.push((0x80 | ((codepoint >> 12) & 0x3F)) as u8);
            output.push((0x80 | ((codepoint >> 6) & 0x3F)) as u8);
            output.push((0x80 | (codepoint & 0x3F)) as u8);
        }
    }

    /// `strconv_escape` at an '&': the text consumed and what it decodes
    /// to (an unrecognised reference is kept as it is).
    fn escape(&self, start: usize, output: &mut Vec<u8>) -> usize {
        let mut cursor = start + 1;
        match self.at(cursor) {
            b'#' => {
                let mut codepoint = 0u32;
                if self.at(cursor + 1) == b'x' {
                    cursor += 2;
                    let mut byte = self.at(cursor);
                    if byte == b';' {
                        output.extend_from_slice(&self.buffer[start..cursor]);
                        return cursor;
                    }
                    loop {
                        if byte.wrapping_sub(b'0') <= 9 {
                            codepoint = codepoint.wrapping_mul(16).wrapping_add((byte - b'0') as u32);
                        } else if (byte | b' ').wrapping_sub(b'a') <= 5 {
                            codepoint = codepoint.wrapping_mul(16).wrapping_add(((byte | b' ') - b'a' + 10) as u32);
                        } else if byte == b';' {
                            break;
                        } else {
                            output.extend_from_slice(&self.buffer[start..cursor]);
                            return cursor;
                        }
                        cursor += 1;
                        byte = self.at(cursor);
                    }
                    cursor += 1;
                } else {
                    cursor += 1;
                    let mut byte = self.at(cursor);
                    if byte == b';' {
                        output.extend_from_slice(&self.buffer[start..cursor]);
                        return cursor;
                    }
                    loop {
                        if byte.wrapping_sub(b'0') <= 9 {
                            codepoint = codepoint.wrapping_mul(10).wrapping_add((byte - b'0') as u32);
                        } else if byte == b';' {
                            break;
                        } else {
                            output.extend_from_slice(&self.buffer[start..cursor]);
                            return cursor;
                        }
                        cursor += 1;
                        byte = self.at(cursor);
                    }
                    cursor += 1;
                }
                Self::push_codepoint(output, codepoint);
                cursor
            }
            b'a' => {
                cursor += 1;
                if self.at(cursor) == b'm' {
                    cursor += 1;
                    if self.at(cursor) == b'p' {
                        cursor += 1;
                        if self.at(cursor) == b';' {
                            output.push(b'&');
                            return cursor + 1;
                        }
                    }
                } else if self.at(cursor) == b'p' {
                    let mut ok = true;
                    for expected in *b"os;" {
                        cursor += 1;
                        if self.at(cursor) != expected {
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        output.push(b'\'');
                        return cursor + 1;
                    }
                }
                output.extend_from_slice(&self.buffer[start..cursor]);
                cursor
            }
            b'g' | b'l' => {
                let replacement = if self.at(cursor) == b'g' { b'>' } else { b'<' };
                cursor += 1;
                if self.at(cursor) == b't' {
                    cursor += 1;
                    if self.at(cursor) == b';' {
                        output.push(replacement);
                        return cursor + 1;
                    }
                }
                output.extend_from_slice(&self.buffer[start..cursor]);
                cursor
            }
            b'q' => {
                let mut ok = true;
                for expected in *b"uot;" {
                    cursor += 1;
                    if self.at(cursor) != expected {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    output.push(b'"');
                    return cursor + 1;
                }
                output.extend_from_slice(&self.buffer[start..cursor]);
                cursor
            }
            _ => {
                output.extend_from_slice(&self.buffer[start..cursor]);
                cursor
            }
        }
    }

    /// `strconv_pcdata` (escapes, end-of-line normalisation): the text and
    /// the position of the '<' ending it (`None` at the end).
    fn pcdata(&self, mut cursor: usize) -> (Vec<u8>, Option<usize>) {
        let mut output = Vec::new();
        loop {
            let byte = self.at(cursor);
            if !is(byte, CT_PARSE_PCDATA) {
                output.push(byte);
                cursor += 1;
                continue;
            }
            match byte {
                b'<' => return (output, Some(cursor)),
                b'\r' => {
                    output.push(b'\n');
                    cursor += 1;
                    if self.at(cursor) == b'\n' {
                        cursor += 1;
                    }
                }
                b'&' => cursor = self.escape(cursor, &mut output),
                _ => return (output, None),
            }
        }
    }

    /// `strconv_attribute_impl<true>::parse_wconv`: the value and the
    /// position after its quote (none at the end of the text, where the
    /// value is as far as it went).
    fn attribute_value(&self, mut cursor: usize, quote: u8) -> (Vec<u8>, Option<usize>) {
        let mut output = Vec::new();
        loop {
            let byte = self.at(cursor);
            if !is(byte, CT_PARSE_ATTR_WS) {
                output.push(byte);
                cursor += 1;
                continue;
            }
            if byte == quote {
                return (output, Some(cursor + 1));
            }
            if is(byte, CT_SPACE) {
                output.push(b' ');
                cursor += 1;
                if byte == b'\r' && self.at(cursor) == b'\n' {
                    cursor += 1;
                }
            } else if byte == b'&' {
                cursor = self.escape(cursor, &mut output);
            } else if byte == 0 {
                return (output, None);
            } else {
                output.push(byte);
                cursor += 1;
            }
        }
    }

    /// `strconv_comment`.
    fn comment(&self, mut cursor: usize) -> Parse<(Vec<u8>, usize)> {
        let mut output = Vec::new();
        loop {
            let byte = self.at(cursor);
            if !is(byte, CT_PARSE_COMMENT) {
                output.push(byte);
                cursor += 1;
                continue;
            }
            if byte == b'\r' {
                output.push(b'\n');
                cursor += 1;
                if self.at(cursor) == b'\n' {
                    cursor += 1;
                }
            } else if byte == b'-' && self.at(cursor + 1) == b'-' && self.at(cursor + 2) == b'>' {
                return Ok((output, cursor + 3));
            } else if byte == 0 {
                return Err(XmlError);
            } else {
                output.push(byte);
                cursor += 1;
            }
        }
    }

    /// `strconv_cdata`: the text and the position after "]]>".
    fn cdata(&self, mut cursor: usize) -> Parse<(Vec<u8>, usize)> {
        let mut output = Vec::new();
        loop {
            let byte = self.at(cursor);
            if !is(byte, CT_PARSE_CDATA) {
                output.push(byte);
                cursor += 1;
                continue;
            }
            if byte == b'\r' {
                output.push(b'\n');
                cursor += 1;
                if self.at(cursor) == b'\n' {
                    cursor += 1;
                }
            } else if byte == b']' && self.at(cursor + 1) == b']' && self.at(cursor + 2) == b'>' {
                return Ok((output, cursor + 3));
            } else if byte == 0 {
                return Err(XmlError);
            } else {
                output.push(byte);
                cursor += 1;
            }
        }
    }

    /// `parse_doctype_primitive`.
    fn doctype_primitive(&self, mut cursor: usize) -> Parse<usize> {
        let byte = self.at(cursor);
        if byte == b'"' || byte == b'\'' {
            cursor += 1;
            while self.at(cursor) != 0 && self.at(cursor) != byte {
                cursor += 1;
            }
            if self.at(cursor) == 0 {
                return Err(XmlError);
            }
            return Ok(cursor + 1);
        }
        if byte == b'<' && self.at(cursor + 1) == b'?' {
            cursor += 2;
            while self.at(cursor) != 0 && !(self.at(cursor) == b'?' && self.at(cursor + 1) == b'>') {
                cursor += 1;
            }
            if self.at(cursor) == 0 {
                return Err(XmlError);
            }
            return Ok(cursor + 2);
        }
        if byte == b'<' && self.at(cursor + 1) == b'!' && self.at(cursor + 2) == b'-' && self.at(cursor + 3) == b'-' {
            cursor += 4;
            while self.at(cursor) != 0 && !(self.at(cursor) == b'-' && self.at(cursor + 1) == b'-' && self.at(cursor + 2) == b'>') {
                cursor += 1;
            }
            if self.at(cursor) == 0 {
                return Err(XmlError);
            }
            return Ok(cursor + 3);
        }
        Err(XmlError)
    }

    /// `parse_doctype_ignore`.
    fn doctype_ignore(&self, mut cursor: usize) -> Parse<usize> {
        let mut depth = 0usize;
        cursor += 3;
        while self.at(cursor) != 0 {
            if self.at(cursor) == b'<' && self.at(cursor + 1) == b'!' && self.at(cursor + 2) == b'[' {
                cursor += 3;
                depth += 1;
            } else if self.at(cursor) == b']' && self.at(cursor + 1) == b']' && self.at(cursor + 2) == b'>' {
                cursor += 3;
                if depth == 0 {
                    return Ok(cursor);
                }
                depth -= 1;
            } else {
                cursor += 1;
            }
        }
        Err(XmlError)
    }

    /// `parse_doctype_group`: the position of the closing '>'.
    fn doctype_group(&self, mut cursor: usize) -> Parse<usize> {
        let mut depth = 0usize;
        cursor += 2;
        while self.at(cursor) != 0 {
            let byte = self.at(cursor);
            if byte == b'<' && self.at(cursor + 1) == b'!' && self.at(cursor + 2) != b'-' {
                if self.at(cursor + 2) == b'[' {
                    cursor = self.doctype_ignore(cursor)?;
                } else {
                    cursor += 2;
                    depth += 1;
                }
            } else if byte == b'<' || byte == b'"' || byte == b'\'' {
                cursor = self.doctype_primitive(cursor)?;
            } else if byte == b'>' {
                if depth == 0 {
                    return Ok(cursor);
                }
                depth -= 1;
                cursor += 1;
            } else {
                cursor += 1;
            }
        }
        Err(XmlError)
    }

    /// The attribute list of an element (or declaration) up to '>' or "/>";
    /// returns the position after it and whether the element closed. Each
    /// attribute is the node's from its name on (pugixml appends it then),
    /// so one that fails keeps its value as far as it went.
    fn attributes(&self, node: &mut Node, mut cursor: usize) -> Parse<(usize, bool)> {
        loop {
            cursor = self.skip_whitespace(cursor);
            let byte = self.at(cursor);
            if is(byte, CT_START_SYMBOL) {
                let name_start = cursor;
                while is(self.at(cursor), CT_SYMBOL) {
                    cursor += 1;
                }
                let name = self.buffer[name_start..cursor].to_vec();
                let mut terminator = self.at(cursor);
                cursor += 1;
                if is(terminator, CT_SPACE) {
                    cursor = self.skip_whitespace(cursor);
                    terminator = self.at(cursor);
                    cursor += 1;
                }
                if terminator != b'=' {
                    node.attributes.push((name, Vec::new()));
                    return Err(XmlError);
                }
                cursor = self.skip_whitespace(cursor);
                let quote = self.at(cursor);
                if quote != b'"' && quote != b'\'' {
                    node.attributes.push((name, Vec::new()));
                    return Err(XmlError);
                }
                let (mut value, after) = self.attribute_value(cursor + 1, quote);
                truncate_at_nul(&mut value);
                node.attributes.push((name, value));
                let Some(after) = after else {
                    return Err(XmlError);
                };
                cursor = after;
                if is(self.at(cursor), CT_START_SYMBOL) {
                    return Err(XmlError);
                }
            } else if byte == b'/' {
                cursor += 1;
                if self.at(cursor) == b'>' {
                    return Ok((cursor + 1, true));
                }
                return Err(XmlError);
            } else if byte == b'>' {
                return Ok((cursor + 1, false));
            } else {
                return Err(XmlError);
            }
        }
    }

    /// `parse_tree` from `cursor` into the document, with the open nodes
    /// on `stack` (an element whose start tag fails is left there too, as
    /// pugixml has it in the tree from its name on).
    fn tree(&mut self, document: &mut Node, stack: &mut Vec<Node>) -> Parse<()> {
        let mut cursor = 0usize;
        // UTF-8 BOM.
        if self.at(0) == 0xEF && self.at(1) == 0xBB && self.at(2) == 0xBF {
            cursor = 3;
        }
        fn append(stack: &mut [Node], document: &mut Node, node: Node) {
            match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None => document.children.push(node),
            }
        }
        while self.at(cursor) != 0 {
            if self.at(cursor) == b'<' {
                cursor += 1;
                {
                    // LOC_TAG
                    let byte = self.at(cursor);
                    if is(byte, CT_START_SYMBOL) {
                        let name_start = cursor;
                        while is(self.at(cursor), CT_SYMBOL) {
                            cursor += 1;
                        }
                        let mut element = Node::new(NodeKind::Element);
                        element.name = self.buffer[name_start..cursor].to_vec();
                        let terminator = self.at(cursor);
                        cursor += 1;
                        if terminator == b'>' {
                            stack.push(element);
                        } else if is(terminator, CT_SPACE) {
                            let (after, closed) = match self.attributes(&mut element, cursor) {
                                Ok(parsed) => parsed,
                                Err(error) => {
                                    stack.push(element);
                                    return Err(error);
                                }
                            };
                            cursor = after;
                            if closed {
                                append(stack, document, element);
                            } else {
                                stack.push(element);
                            }
                        } else if terminator == b'/' {
                            if self.at(cursor) != b'>' {
                                stack.push(element);
                                return Err(XmlError);
                            }
                            cursor += 1;
                            append(stack, document, element);
                        } else {
                            stack.push(element);
                            return Err(XmlError);
                        }
                    } else if byte == b'/' {
                        cursor += 1;
                        let Some(open) = stack.pop() else {
                            return Err(XmlError);
                        };
                        // (On an error the element stays in the tree, as
                        // pugixml built it in place.)
                        let mut name_index = 0usize;
                        let mut matches = true;
                        while is(self.at(cursor), CT_SYMBOL) {
                            if open.name.get(name_index) != Some(&self.at(cursor)) {
                                matches = false;
                                break;
                            }
                            cursor += 1;
                            name_index += 1;
                        }
                        if !matches || name_index != open.name.len() {
                            append(stack, document, open);
                            return Err(XmlError);
                        }
                        append(stack, document, open);
                        cursor = self.skip_whitespace(cursor);
                        if self.at(cursor) == 0 {
                            return Err(XmlError);
                        }
                        if self.at(cursor) != b'>' {
                            return Err(XmlError);
                        }
                        cursor += 1;
                    } else if byte == b'?' {
                        cursor += 1;
                        let target_start = cursor;
                        if !is(self.at(cursor), CT_START_SYMBOL) {
                            return Err(XmlError);
                        }
                        while is(self.at(cursor), CT_SYMBOL) {
                            cursor += 1;
                        }
                        if self.at(cursor) == 0 {
                            return Err(XmlError);
                        }
                        let target = self.buffer[target_start..cursor].to_vec();
                        let declaration = target.len() == 3 && (target[0] | b' ') == b'x' && (target[1] | b' ') == b'm' && (target[2] | b' ') == b'l';
                        let mut node = Node::new(if declaration { NodeKind::Declaration } else { NodeKind::Pi });
                        if declaration && !stack.is_empty() {
                            return Err(XmlError);
                        }
                        node.name = target;
                        let terminator = self.at(cursor);
                        cursor += 1;
                        if terminator == b'?' {
                            if self.at(cursor) != b'>' {
                                return Err(XmlError);
                            }
                            cursor += 1;
                            append(stack, document, node);
                        } else if is(terminator, CT_SPACE) {
                            cursor = self.skip_whitespace(cursor);
                            let value_start = cursor;
                            while self.at(cursor) != 0 && !(self.at(cursor) == b'?' && self.at(cursor + 1) == b'>') {
                                cursor += 1;
                            }
                            if self.at(cursor) == 0 {
                                return Err(XmlError);
                            }
                            if declaration {
                                // The '?' becomes '/' and the content parses as
                                // attributes closed by "/>"; a '>' before it
                                // leaves the declaration open, as an element.
                                self.buffer[cursor] = b'/';
                                let (after, closed) = self.attributes(&mut node, value_start)?;
                                cursor = after;
                                if closed {
                                    append(stack, document, node);
                                } else {
                                    stack.push(node);
                                }
                            } else {
                                node.value = self.buffer[value_start..cursor].to_vec();
                                cursor += 2;
                                append(stack, document, node);
                            }
                        } else {
                            return Err(XmlError);
                        }
                    } else if byte == b'!' {
                        cursor += 1;
                        if self.at(cursor) == b'-' {
                            cursor += 1;
                            if self.at(cursor) != b'-' {
                                return Err(XmlError);
                            }
                            let (value, after) = self.comment(cursor + 1)?;
                            let mut node = Node::new(NodeKind::Comment);
                            node.value = value;
                            append(stack, document, node);
                            cursor = after;
                        } else if self.at(cursor) == b'[' {
                            let opening = b"CDATA[";
                            for &expected in opening {
                                cursor += 1;
                                if self.at(cursor) != expected {
                                    return Err(XmlError);
                                }
                            }
                            let (value, after) = self.cdata(cursor + 1)?;
                            let mut node = Node::new(NodeKind::Cdata);
                            node.value = value;
                            append(stack, document, node);
                            cursor = after;
                        } else if self.buffer.get(cursor..cursor + 7) == Some(b"DOCTYPE".as_slice()) {
                            let start = cursor - 2;
                            if !stack.is_empty() {
                                return Err(XmlError);
                            }
                            let mut mark = start + 9;
                            let close = self.doctype_group(start)?;
                            mark = self.skip_whitespace(mark).min(close);
                            let mut node = Node::new(NodeKind::Doctype);
                            node.value = self.buffer[mark..close].to_vec();
                            append(stack, document, node);
                            cursor = close + 1;
                        } else {
                            return Err(XmlError);
                        }
                    } else {
                        return Err(XmlError);
                    }
                }
            } else {
                let mark = cursor;
                cursor = self.skip_whitespace(cursor);
                if self.at(cursor) == b'<' || self.at(cursor) == 0 {
                    continue;
                }
                cursor = mark;
                if !stack.is_empty() {
                    let (mut value, tag) = self.pcdata(cursor);
                    truncate_at_nul(&mut value);
                    let mut node = Node::new(NodeKind::Pcdata);
                    node.value = value;
                    append(stack, document, node);
                    // At the end, or a '<' with nothing after it: stop. Else
                    // the loop takes the tag at the '<'.
                    let Some(tag) = tag else {
                        break;
                    };
                    if self.at(tag + 1) == 0 {
                        break;
                    }
                    cursor = tag;
                } else {
                    while self.at(cursor) != 0 && self.at(cursor) != b'<' {
                        cursor += 1;
                    }
                    if self.at(cursor) == 0 {
                        break;
                    }
                }
            }
        }
        if !stack.is_empty() {
            return Err(XmlError);
        }
        Ok(())
    }
}

/// `xml_document::load_buffer(data, size, parse_full)` on the file bytes
/// plus a NUL: the document node, or an error when parsing fails or no
/// element was found.
pub fn parse(bytes: &[u8]) -> Result<Node, XmlError> {
    let (document, complete) = parse_document(bytes);
    if !complete || !document.children.iter().any(Node::is_element) {
        return Err(XmlError);
    }
    Ok(document)
}

/// The document pugixml leaves after a parse, failed or not (a caller that
/// ignores the result reads it): what was parsed, the elements still open
/// in place where they opened.
pub fn parse_partial(bytes: &[u8]) -> Node {
    parse_document(bytes).0
}

/// The document and whether it parsed.
fn parse_document(bytes: &[u8]) -> (Node, bool) {
    let mut data = bytes.to_vec();
    data.push(0);
    let buffer = convert_to_utf8(&data);
    let mut parser = Parser { buffer };
    let mut document = Node::new(NodeKind::Document);
    let mut stack = Vec::new();
    let complete = parser.tree(&mut document, &mut stack).is_ok();
    while let Some(open) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(open),
            None => document.children.push(open),
        }
    }
    (document, complete)
}

/// pugixml's `string_to_integer<unsigned int>`.
fn string_to_integer(value: &[u8], minimum: u32, maximum: u32) -> u32 {
    let mut index = 0;
    while matches!(at(value, index), b' ' | b'\t' | b'\n' | b'\r') {
        index += 1;
    }
    let negative = at(value, index) == b'-';
    if matches!(at(value, index), b'+' | b'-') {
        index += 1;
    }
    let mut result = 0u32;
    let overflow = if at(value, index) == b'0' && (at(value, index + 1) | b' ') == b'x' {
        index += 2;
        while at(value, index) == b'0' {
            index += 1;
        }
        let start = index;
        loop {
            let byte = at(value, index);
            if byte.wrapping_sub(b'0') < 10 {
                result = result.wrapping_mul(16).wrapping_add((byte - b'0') as u32);
            } else if (byte | b' ').wrapping_sub(b'a') < 6 {
                result = result.wrapping_mul(16).wrapping_add(((byte | b' ') - b'a' + 10) as u32);
            } else {
                break;
            }
            index += 1;
        }
        index - start > 8
    } else {
        while at(value, index) == b'0' {
            index += 1;
        }
        let start = index;
        while at(value, index).wrapping_sub(b'0') < 10 {
            result = result.wrapping_mul(10).wrapping_add((at(value, index) - b'0') as u32);
            index += 1;
        }
        let digits = index - start;
        let lead = at(value, start);
        let fits = digits == 10 && (lead < b'4' || (lead == b'4' && result >> 31 != 0));
        digits >= 10 && !fits
    };
    if negative {
        let limit = 0u32.wrapping_sub(minimum);
        if overflow || result > limit { minimum } else { 0u32.wrapping_sub(result) }
    } else if overflow || result > maximum {
        maximum
    } else {
        result
    }
}

/// `xml_attribute::as_int()` (and `xml_text::as_int()`).
pub(crate) fn as_int(value: &[u8]) -> i32 {
    string_to_integer(value, i32::MIN as u32, i32::MAX as u32) as i32
}

/// `xml_attribute::as_uint()`.
pub(crate) fn as_uint(value: &[u8]) -> u32 {
    string_to_integer(value, 0, u32::MAX)
}

/// `xml_attribute::as_float()`: `strtod`, as a float.
pub(crate) fn as_float(value: &[u8]) -> f32 {
    crate::parsing::strtod(value) as f32
}

/// `xml_attribute::as_bool()`: whether the first character is '1', 't',
/// 'T', 'y' or 'Y'.
pub(crate) fn as_bool(value: &[u8]) -> bool {
    matches!(value.first(), Some(b'1' | b't' | b'T' | b'y' | b'Y'))
}

/// A pugixml string: everything up to the first NUL.
fn truncate_at_nul(value: &mut Vec<u8>) {
    if let Some(end) = value.iter().position(|&byte| byte == 0) {
        value.truncate(end);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Utf8,
    Utf16 { big_endian: bool },
    Utf32 { big_endian: bool },
    Latin1,
}

/// `parse_declaration_encoding`: the `encoding` value of a leading
/// `<?xml ` declaration.
fn declaration_encoding(data: &[u8]) -> Option<&[u8]> {
    let size = data.len();
    let declaration_start = size >= 6 && data.starts_with(b"<?xml") && is(data[5], CT_SPACE);
    if !declaration_start {
        return None;
    }
    let mut index = 6;
    while index + 1 < size {
        if data[index] == b'?' {
            return None;
        }
        if data[index] == b'e' && data[index + 1] == b'n' {
            let mut offset = index;
            for &expected in b"encoding" {
                if offset >= size || data[offset] != expected {
                    return None;
                }
                offset += 1;
            }
            while offset < size && is(data[offset], CT_SPACE) {
                offset += 1;
            }
            if offset >= size || data[offset] != b'=' {
                return None;
            }
            offset += 1;
            while offset < size && is(data[offset], CT_SPACE) {
                offset += 1;
            }
            let delimiter = if offset < size && data[offset] == b'"' { b'"' } else { b'\'' };
            if offset >= size || data[offset] != delimiter {
                return None;
            }
            offset += 1;
            let start = offset;
            while offset < size && is(data[offset], CT_SYMBOL) {
                offset += 1;
            }
            let end = offset;
            if offset >= size || data[offset] != delimiter {
                return None;
            }
            return Some(&data[start..end]);
        }
        index += 1;
    }
    None
}

/// `guess_buffer_encoding`.
fn guess_encoding(data: &[u8]) -> Encoding {
    if data.len() < 4 {
        return Encoding::Utf8;
    }
    let (d0, d1, d2, d3) = (data[0], data[1], data[2], data[3]);
    if d0 == 0 && d1 == 0 && d2 == 0xFE && d3 == 0xFF {
        return Encoding::Utf32 { big_endian: true };
    }
    if d0 == 0xFF && d1 == 0xFE && d2 == 0 && d3 == 0 {
        return Encoding::Utf32 { big_endian: false };
    }
    if d0 == 0xFE && d1 == 0xFF {
        return Encoding::Utf16 { big_endian: true };
    }
    if d0 == 0xFF && d1 == 0xFE {
        return Encoding::Utf16 { big_endian: false };
    }
    if d0 == 0xEF && d1 == 0xBB && d2 == 0xBF {
        return Encoding::Utf8;
    }
    if d0 == 0 && d1 == 0 && d2 == 0 && d3 == 0x3C {
        return Encoding::Utf32 { big_endian: true };
    }
    if d0 == 0x3C && d1 == 0 && d2 == 0 && d3 == 0 {
        return Encoding::Utf32 { big_endian: false };
    }
    if d0 == 0 && d1 == 0x3C && d2 == 0 && d3 == 0x3F {
        return Encoding::Utf16 { big_endian: true };
    }
    if d0 == 0x3C && d1 == 0 && d2 == 0x3F && d3 == 0 {
        return Encoding::Utf16 { big_endian: false };
    }
    if d0 == 0 && d1 == 0x3C {
        return Encoding::Utf16 { big_endian: true };
    }
    if d0 == 0x3C && d1 == 0 {
        return Encoding::Utf16 { big_endian: false };
    }
    if d0 == 0x3C && d1 == 0x3F && d2 == 0x78 && d3 == 0x6D
        && let Some(name) = declaration_encoding(data) {
            let lower = |index: usize| name[index] | b' ';
            let iso_8859_1 = name.len() == 10 && lower(0) == b'i' && lower(1) == b's' && lower(2) == b'o' && &name[3..] == b"-8859-1";
            let latin1 = name.len() == 6 && lower(0) == b'l' && lower(1) == b'a' && lower(2) == b't' && lower(3) == b'i' && lower(4) == b'n' && name[5] == b'1';
            if iso_8859_1 || latin1 {
                return Encoding::Latin1;
            }
        }
    Encoding::Utf8
}

/// `convert_buffer` into UTF-8, ending in a NUL (the data already ends in
/// Assimp's).
fn convert_to_utf8(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len() + 1);
    match guess_encoding(data) {
        Encoding::Utf8 => output.extend_from_slice(data),
        Encoding::Latin1 => {
            for &byte in data {
                Parser::push_codepoint(&mut output, byte as u32);
            }
        }
        Encoding::Utf32 { big_endian } => {
            for chunk in data.as_chunks::<4>().0 {
                let word = [chunk[0], chunk[1], chunk[2], chunk[3]];
                let codepoint = if big_endian { u32::from_be_bytes(word) } else { u32::from_le_bytes(word) };
                Parser::push_codepoint(&mut output, codepoint);
            }
        }
        Encoding::Utf16 { big_endian } => {
            let mut units = Vec::with_capacity(data.len() / 2);
            for pair in data.as_chunks::<2>().0 {
                let unit = if big_endian { u16::from_be_bytes([pair[0], pair[1]]) } else { u16::from_le_bytes([pair[0], pair[1]]) };
                units.push(unit as u32);
            }
            let mut index = 0;
            while index < units.len() {
                let lead = units[index];
                if lead < 0xD800 || lead.wrapping_sub(0xE000) < 0x2000 {
                    Parser::push_codepoint(&mut output, lead);
                    index += 1;
                } else if lead.wrapping_sub(0xD800) < 0x400 && index + 1 < units.len() {
                    let next = units[index + 1];
                    if next.wrapping_sub(0xDC00) < 0x400 {
                        Parser::push_codepoint(&mut output, 0x10000 + ((lead & 0x3FF) << 10) + (next & 0x3FF));
                        index += 2;
                    } else {
                        index += 1;
                    }
                } else {
                    index += 1;
                }
            }
        }
    }
    output.push(0);
    output
}
