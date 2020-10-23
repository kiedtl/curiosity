/// This module implements a simple text/gemini parser based on the description
/// here: https://gemini.circumlunar.space/docs/specification.html
use std::io::{self, Write};

/// Build a gemini document up from a series of nodes.
#[derive(Default)]
pub struct Builder {
    nodes: Vec<Node>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder::default()
    }

    pub fn text<T: Into<String>>(mut self, data: T) -> Builder {
        self.nodes.push(Node::Text(data.into()));
        self
    }

    pub fn link<T: Into<String>>(mut self, to: T, name: Option<String>) -> Builder {
        self.nodes.push(Node::Link {
            to: to.into(),
            name: name,
        });
        self
    }

    pub fn preformatted<T: Into<String>>(mut self, data: T) -> Builder {
        self.nodes.push(Node::Preformatted(data.into()));
        self
    }

    pub fn heading<T: Into<String>>(mut self, level: u8, body: T) -> Builder {
        self.nodes.push(Node::Heading {
            level: level,
            body: body.into(),
        });
        self
    }

    pub fn list_item<T: Into<String>>(mut self, item: T) -> Builder {
        self.nodes.push(Node::ListItem(item.into()));
        self
    }

    pub fn quote<T: Into<String>>(mut self, body: T) -> Builder {
        self.nodes.push(Node::Quote(body.into()));
        self
    }

    pub fn build(self) -> Vec<Node> {
        self.nodes
    }
}

/// Render a set of nodes as a document to a writer.
pub fn render(nodes: Vec<Node>, out: &mut impl Write) -> io::Result<()> {
    use Node::*;

    for node in nodes {
        match node {
            Text(body) => {
                let special_prefixes = ["=>", "```", "#", "*", ">"];
                if special_prefixes.iter().any(|prefix| body.starts_with(prefix)) {
                    write!(out, " ")?;
                }
                write!(out, "{}\n", body)?
            },
            Link { to, name } => match name {
                Some(name) => write!(out, "=> {} {}\n", to, name)?,
                None => write!(out, "=> {}\n", to)?,
            },
            Preformatted(body) => write!(out, "```\n{}\n```\n", body)?,
            Heading { level, body } => write!(out, "{} {}\n", "#".repeat(level as usize), body)?,
            ListItem(body) => write!(out, "* {}\n", body)?,
            Quote(body) => write!(out, "> {}\n", body)?,
        };
    }

    Ok(())
}

/// Individual nodes of the document. Each node correlates to a line in the file.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Node {
    /// Text lines are the most fundamental line type - any line which does not
    /// match the definition of another line type defined below defaults to
    /// being a text line. The majority of lines in a typical text/gemini document will be text lines.
    Text(String),

    /// Lines beginning with the two characters "=>" are link lines, which have the following syntax:
    ///
    /// ```gemini
    /// =>[<whitespace>]<URL>[<whitespace><USER-FRIENDLY LINK NAME>]
    /// ```
    ///
    /// where:
    ///
    /// * `<whitespace>` is any non-zero number of consecutive spaces or tabs
    /// * Square brackets indicate that the enclosed content is optional.
    /// * `<URL>` is a URL, which may be absolute or relative. If the URL
    ///   does not include a scheme, a scheme of `gemini://` is implied.
    Link { to: String, name: Option<String> },

    /// Any line whose first three characters are "```" (i.e. three consecutive
    /// back ticks with no leading whitespace) are preformatted toggle lines.
    /// These lines should NOT be included in the rendered output shown to the
    /// user. Instead, these lines toggle the parser between preformatted mode
    /// being "on" or "off". Preformatted mode should be "off" at the beginning
    /// of a document. The current status of preformatted mode is the only
    /// internal state a parser is required to maintain. When preformatted mode
    /// is "on", the usual rules for identifying line types are suspended, and
    /// all lines should be identified as preformatted text lines (see 5.4.4).
    ///
    /// Preformatted text lines should be presented to the user in a "neutral",
    /// monowidth font without any alteration to whitespace or stylistic
    /// enhancements. Graphical clients should use scrolling mechanisms to present
    /// preformatted text lines which are longer than the client viewport, in
    /// preference to wrapping. In displaying preformatted text lines, clients
    /// should keep in mind applications like ASCII art and computer source
    /// code: in particular, source code in languages with significant whitespace
    /// (e.g. Python) should be able to be copied and pasted from the client into
    /// a file and interpreted/compiled without any problems arising from the
    /// client's manner of displaying them.
    Preformatted(String),

    /// Lines beginning with "#" are heading lines. Heading lines consist of one,
    /// two or three consecutive "#" characters, followed by optional whitespace,
    /// followed by heading text. The number of # characters indicates the "level"
    /// of header; #, ## and ### can be thought of as analogous to `<h1>`, `<h2>`
    /// and `<h3>` in HTML.
    ///
    /// Heading text should be presented to the user, and clients MAY use special
    /// formatting, e.g. a larger or bold font, to indicate its status as a header
    /// (simple clients may simply print the line, including its leading #s,
    /// without any styling at all). However, the main motivation for the
    /// definition of heading lines is not stylistic but to provide a
    /// machine-readable representation of the internal structure of the document.
    /// Advanced clients can use this information to, e.g. display an automatically
    /// generated and hierarchically formatted "table of contents" for a long
    /// document in a side-pane, allowing users to easily jump to specific sections
    /// without excessive scrolling. CMS-style tools automatically generating menus
    /// or Atom/RSS feeds for a directory of text/gemini files can use first
    /// heading in the file as a human-friendly title.
    Heading { level: u8, body: String },

    /// Lines beginning with "* " are unordered list items. This line type exists
    /// purely for stylistic reasons. The * may be replaced in advanced clients by
    /// a bullet symbol. Any text after the "* " should be presented to the user as
    /// if it were a text line, i.e. wrapped to fit the viewport and formatted
    /// "nicely". Advanced clients can take the space of the bullet symbol into
    /// account when wrapping long list items to ensure that all lines of text
    /// corresponding to the item are offset an equal distance from the left of the screen.
    ListItem(String),

    /// Lines beginning with ">" are quote lines. This line type exists so that
    /// advanced clients may use distinct styling to convey to readers the important
    /// semantic information that certain text is being quoted from an external
    /// source. For example, when wrapping long lines to the the viewport, each
    /// resultant line may have a ">" symbol placed at the front.
    Quote(String),
}

impl Node {
    pub fn blank() -> Node {
        Node::Text("".to_string())
    }
}

pub fn parse(doc: &str) -> Vec<Node> {
    let mut result: Vec<Node> = vec![];
    let mut collect_preformatted: bool = false;
    let mut preformatted_buffer: Vec<u8> = vec![];

    for line in doc.lines() {
        if line.starts_with("```") {
            collect_preformatted = !collect_preformatted;
            if !collect_preformatted {
                result.push(Node::Preformatted(
                    String::from_utf8(preformatted_buffer)
                        .unwrap()
                        .trim_end()
                        .to_string(),
                ));
                preformatted_buffer = vec![];
            }
            continue;
        }

        if collect_preformatted && line != "```" {
            write!(preformatted_buffer, "{}\n", line).unwrap();
            continue;
        }

        // Quotes
        if line.starts_with(">") {
            result.push(Node::Quote(line[1..].trim().to_string()));
            continue;
        }

        // List items
        if line.starts_with("*") {
            result.push(Node::ListItem(line[1..].trim().to_string()));
            continue;
        }

        // Headings
        if line.starts_with("###") {
            result.push(Node::Heading {
                level: 3,
                body: line[3..].trim().to_string(),
            });
            continue;
        }
        if line.starts_with("##") {
            result.push(Node::Heading {
                level: 2,
                body: line[2..].trim().to_string(),
            });
            continue;
        }
        if line.starts_with("#") {
            result.push(Node::Heading {
                level: 1,
                body: line[1..].trim().to_string(),
            });
            continue;
        }

        // Links
        if line.starts_with("=>") {
            let sp = line[2..].split_ascii_whitespace().collect::<Vec<&str>>();

            match sp.len() {
                0 => (),
                1 => result.push(Node::Link {
                    to: sp[0].trim().to_string(),
                    name: None,
                }),
                _ => result.push(Node::Link {
                    to: sp[0].trim().to_string(),
                    name: Some(sp[1..].join(" ").trim().to_string()),
                }),
            }

            continue;
        }

        result.push(Node::Text(line.to_string()));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic() {
        let _ = pretty_env_logger::try_init();
        let msg = include_str!("../../majc/src/help.gmi");
        let doc = super::parse(msg);
        assert_ne!(doc.len(), 0);
    }

    #[test]
    fn quote() {
        let _ = pretty_env_logger::try_init();
        let msg = ">hi there";
        let expected: Vec<Node> = vec![Node::Quote("hi there".to_string())];
        assert_eq!(expected, parse(msg));
    }

    #[test]
    fn list() {
        let _ = pretty_env_logger::try_init();
        let msg = "*hi there";
        let expected: Vec<Node> = vec![Node::ListItem("hi there".to_string())];
        assert_eq!(expected, parse(msg));
    }

    #[test]
    fn preformatted() {
        let _ = pretty_env_logger::try_init();
        let msg = "```\n\
                   hi there\n\
                   ```\n\
                   \n\
                   Test\n";
        let expected: Vec<Node> = vec![
            Node::Preformatted("hi there".to_string()),
            Node::Text(String::new()),
            Node::Text("Test".to_string()),
        ];
        assert_eq!(expected, parse(msg));
    }

    #[test]
    fn header() {
        let _ = pretty_env_logger::try_init();
        let msg = "#hi\n##there\n### my friends";
        let expected: Vec<Node> = vec![
            Node::Heading {
                level: 1,
                body: "hi".to_string(),
            },
            Node::Heading {
                level: 2,
                body: "there".to_string(),
            },
            Node::Heading {
                level: 3,
                body: "my friends".to_string(),
            },
        ];
        assert_eq!(expected, parse(msg));
    }

    #[test]
    fn link() {
        let _ = pretty_env_logger::try_init();
        let msg = "=>/\n=> / Go home\n=>";
        let expected: Vec<Node> = vec![
            Node::Link {
                to: "/".to_string(),
                name: None,
            },
            Node::Link {
                to: "/".to_string(),
                name: Some("Go home".to_string()),
            },
        ];
        assert_eq!(expected, parse(msg));
    }

    #[test]
    fn ambiguous_preformatted() {
        let _ = pretty_env_logger::try_init();
        let msg = include_str!("../../testdata/ambig_preformatted.gmi");
        let expected: Vec<Node> = vec![
            Node::Preformatted("FOO".to_string()),
            Node::Text("Foo bar".to_string()),
        ];
        assert_eq!(expected, parse(msg));
    }

    #[test]
    fn ambiguous_text() {
        let _ = pretty_env_logger::try_init();
        let original = Node::Text("#1 World's Best Coder".to_string());
        let expected = " #1 World's Best Coder\n";
        let mut rendered: Vec<u8> = vec![];
        render(vec![original], &mut rendered).unwrap();
        let rendered = String::from_utf8(rendered).unwrap();
        assert_eq!(expected, rendered)
    }
}
