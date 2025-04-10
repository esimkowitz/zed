use gpui::SharedString;
use linkify::LinkFinder;
pub use pulldown_cmark::TagEnd as MarkdownTagEnd;
use pulldown_cmark::{Alignment, HeadingLevel, LinkType, MetadataBlockKind, Options, Parser};
use std::{collections::HashSet, ops::Range};

const PARSE_OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_FOOTNOTES)
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_SMART_PUNCTUATION)
    .union(Options::ENABLE_HEADING_ATTRIBUTES)
    .union(Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS)
    .union(Options::ENABLE_OLD_FOOTNOTES)
    .union(Options::ENABLE_GFM);

pub fn parse_markdown(text: &str) -> (Vec<(Range<usize>, MarkdownEvent)>, HashSet<SharedString>) {
    let mut events = Vec::new();
    let mut languages = HashSet::new();
    let mut within_link = false;
    let mut within_metadata = false;
    for (pulldown_event, mut range) in Parser::new_ext(text, PARSE_OPTIONS).into_offset_iter() {
        if within_metadata {
            if let pulldown_cmark::Event::End(pulldown_cmark::TagEnd::MetadataBlock { .. }) =
                pulldown_event
            {
                within_metadata = false;
            }
            continue;
        }
        match pulldown_event {
            pulldown_cmark::Event::Start(tag) => {
                match tag {
                    pulldown_cmark::Tag::Link { .. } => within_link = true,
                    pulldown_cmark::Tag::MetadataBlock { .. } => within_metadata = true,
                    pulldown_cmark::Tag::CodeBlock(pulldown_cmark::CodeBlockKind::Fenced(
                        ref language,
                    )) => {
                        languages.insert(SharedString::from(language.to_string()));
                    }
                    _ => {}
                }
                events.push((range, MarkdownEvent::Start(tag.into())))
            }
            pulldown_cmark::Event::End(tag) => {
                if let pulldown_cmark::TagEnd::Link = tag {
                    within_link = false;
                }
                events.push((range, MarkdownEvent::End(tag)));
            }
            pulldown_cmark::Event::Text(parsed) => {
                // Automatically detect links in text if we're not already within a markdown
                // link.
                let mut parsed = parsed.as_ref();
                if !within_link {
                    let mut finder = LinkFinder::new();
                    finder.kinds(&[linkify::LinkKind::Url]);
                    let text_range = range.clone();
                    for link in finder.links(&text[text_range.clone()]) {
                        let link_range =
                            text_range.start + link.start()..text_range.start + link.end();

                        if link_range.start > range.start {
                            let (text, tail) = parsed.split_at(link_range.start - range.start);
                            events.push((
                                range.start..link_range.start,
                                MarkdownEvent::Text(SharedString::new(text)),
                            ));
                            parsed = tail;
                        }

                        events.push((
                            link_range.clone(),
                            MarkdownEvent::Start(MarkdownTag::Link {
                                link_type: LinkType::Autolink,
                                dest_url: SharedString::from(link.as_str().to_string()),
                                title: SharedString::default(),
                                id: SharedString::default(),
                            }),
                        ));

                        let (link_text, tail) = parsed.split_at(link_range.end - link_range.start);
                        events.push((
                            link_range.clone(),
                            MarkdownEvent::Text(SharedString::new(link_text)),
                        ));
                        events.push((link_range.clone(), MarkdownEvent::End(MarkdownTagEnd::Link)));

                        range.start = link_range.end;
                        parsed = tail;
                    }
                }
                if range.start < range.end {
                    events.push((range, MarkdownEvent::Text(SharedString::new(parsed))));
                }
            }
            pulldown_cmark::Event::Code(_) => {
                range.start += 1;
                range.end -= 1;
                events.push((range, MarkdownEvent::Code))
            }
            pulldown_cmark::Event::Html(_) => events.push((range, MarkdownEvent::Html)),
            pulldown_cmark::Event::InlineHtml(_) => events.push((range, MarkdownEvent::InlineHtml)),
            pulldown_cmark::Event::FootnoteReference(_) => {
                events.push((range, MarkdownEvent::FootnoteReference))
            }
            pulldown_cmark::Event::SoftBreak => events.push((range, MarkdownEvent::SoftBreak)),
            pulldown_cmark::Event::HardBreak => events.push((range, MarkdownEvent::HardBreak)),
            pulldown_cmark::Event::Rule => events.push((range, MarkdownEvent::Rule)),
            pulldown_cmark::Event::TaskListMarker(checked) => {
                events.push((range, MarkdownEvent::TaskListMarker(checked)))
            }
            pulldown_cmark::Event::InlineMath(_) | pulldown_cmark::Event::DisplayMath(_) => {}
        }
    }
    (events, languages)
}

pub fn parse_links_only(mut text: &str) -> Vec<(Range<usize>, MarkdownEvent)> {
    let mut events = Vec::new();
    let mut finder = LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);
    let mut text_range = Range {
        start: 0,
        end: text.len(),
    };
    for link in finder.links(text) {
        let link_range = link.start()..link.end();

        if link_range.start > text_range.start {
            let (head, tail) = text.split_at(link_range.start - text_range.start);
            events.push((
                text_range.start..link_range.start,
                MarkdownEvent::Text(SharedString::new(head)),
            ));
            text = tail;
        }

        let (link_text, tail) = text.split_at(link_range.end - link_range.start);
        events.push((
            link_range.clone(),
            MarkdownEvent::Start(MarkdownTag::Link {
                link_type: LinkType::Autolink,
                dest_url: SharedString::from(link.as_str().to_string()),
                title: SharedString::default(),
                id: SharedString::default(),
            }),
        ));
        events.push((
            link_range.clone(),
            MarkdownEvent::Text(SharedString::new(link_text)),
        ));
        events.push((link_range.clone(), MarkdownEvent::End(MarkdownTagEnd::Link)));

        text_range.start = link_range.end;
        text = tail;
    }

    if text_range.end > text_range.start {
        events.push((text_range, MarkdownEvent::Text(SharedString::new(text))));
    }

    events
}

/// A static-lifetime equivalent of pulldown_cmark::Event so we can cache the
/// parse result for rendering without resorting to unsafe lifetime coercion.
#[derive(Clone, Debug, PartialEq)]
pub enum MarkdownEvent {
    /// Start of a tagged element. Events that are yielded after this event
    /// and before its corresponding `End` event are inside this element.
    /// Start and end events are guaranteed to be balanced.
    Start(MarkdownTag),
    /// End of a tagged element.
    End(MarkdownTagEnd),
    /// A text node.
    Text(SharedString),
    /// An inline code node.
    Code,
    /// An HTML node.
    Html,
    /// An inline HTML node.
    InlineHtml,
    /// A reference to a footnote with given label, which may or may not be defined
    /// by an event with a `Tag::FootnoteDefinition` tag. Definitions and references to them may
    /// occur in any order.
    FootnoteReference,
    /// A soft line break.
    SoftBreak,
    /// A hard line break.
    HardBreak,
    /// A horizontal ruler.
    Rule,
    /// A task list marker, rendered as a checkbox in HTML. Contains a true when it is checked.
    TaskListMarker(bool),
}

/// Tags for elements that can contain other elements.
#[derive(Clone, Debug, PartialEq)]
pub enum MarkdownTag {
    /// A paragraph of text and other inline elements.
    Paragraph,

    /// A heading, with optional identifier, classes and custom attributes.
    /// The identifier is prefixed with `#` and the last one in the attributes
    /// list is chosen, classes are prefixed with `.` and custom attributes
    /// have no prefix and can optionally have a value (`myattr` o `myattr=myvalue`).
    Heading {
        level: HeadingLevel,
        id: Option<SharedString>,
        classes: Vec<SharedString>,
        /// The first item of the tuple is the attr and second one the value.
        attrs: Vec<(SharedString, Option<SharedString>)>,
    },

    BlockQuote,

    /// A code block.
    CodeBlock(CodeBlockKind),

    /// A HTML block.
    HtmlBlock,

    /// A list. If the list is ordered the field indicates the number of the first item.
    /// Contains only list items.
    List(Option<u64>), // TODO: add delim and tight for ast (not needed for html)

    /// A list item.
    Item,

    /// A footnote definition. The value contained is the footnote's label by which it can
    /// be referred to.
    FootnoteDefinition(SharedString),

    /// A table. Contains a vector describing the text-alignment for each of its columns.
    Table(Vec<Alignment>),

    /// A table header. Contains only `TableCell`s. Note that the table body starts immediately
    /// after the closure of the `TableHead` tag. There is no `TableBody` tag.
    TableHead,

    /// A table row. Is used both for header rows as body rows. Contains only `TableCell`s.
    TableRow,
    TableCell,

    // span-level tags
    Emphasis,
    Strong,
    Strikethrough,

    /// A link.
    Link {
        link_type: LinkType,
        dest_url: SharedString,
        title: SharedString,
        /// Identifier of reference links, e.g. `world` in the link `[hello][world]`.
        id: SharedString,
    },

    /// An image. The first field is the link type, the second the destination URL and the third is a title,
    /// the fourth is the link identifier.
    Image {
        link_type: LinkType,
        dest_url: SharedString,
        title: SharedString,
        /// Identifier of reference links, e.g. `world` in the link `[hello][world]`.
        id: SharedString,
    },

    /// A metadata block.
    MetadataBlock(MetadataBlockKind),

    DefinitionList,
    DefinitionListTitle,
    DefinitionListDefinition,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodeBlockKind {
    Indented,
    /// The value contained in the tag describes the language of the code, which may be empty.
    Fenced(SharedString),
}

impl From<pulldown_cmark::Tag<'_>> for MarkdownTag {
    fn from(tag: pulldown_cmark::Tag) -> Self {
        match tag {
            pulldown_cmark::Tag::Paragraph => MarkdownTag::Paragraph,
            pulldown_cmark::Tag::Heading {
                level,
                id,
                classes,
                attrs,
            } => {
                let id = id.map(|id| SharedString::from(id.into_string()));
                let classes = classes
                    .into_iter()
                    .map(|c| SharedString::from(c.into_string()))
                    .collect();
                let attrs = attrs
                    .into_iter()
                    .map(|(key, value)| {
                        (
                            SharedString::from(key.into_string()),
                            value.map(|v| SharedString::from(v.into_string())),
                        )
                    })
                    .collect();
                MarkdownTag::Heading {
                    level,
                    id,
                    classes,
                    attrs,
                }
            }
            pulldown_cmark::Tag::BlockQuote(_kind) => MarkdownTag::BlockQuote,
            pulldown_cmark::Tag::CodeBlock(kind) => match kind {
                pulldown_cmark::CodeBlockKind::Indented => {
                    MarkdownTag::CodeBlock(CodeBlockKind::Indented)
                }
                pulldown_cmark::CodeBlockKind::Fenced(info) => MarkdownTag::CodeBlock(
                    CodeBlockKind::Fenced(SharedString::from(info.into_string())),
                ),
            },
            pulldown_cmark::Tag::List(start_number) => MarkdownTag::List(start_number),
            pulldown_cmark::Tag::Item => MarkdownTag::Item,
            pulldown_cmark::Tag::FootnoteDefinition(label) => {
                MarkdownTag::FootnoteDefinition(SharedString::from(label.to_string()))
            }
            pulldown_cmark::Tag::Table(alignments) => MarkdownTag::Table(alignments),
            pulldown_cmark::Tag::TableHead => MarkdownTag::TableHead,
            pulldown_cmark::Tag::TableRow => MarkdownTag::TableRow,
            pulldown_cmark::Tag::TableCell => MarkdownTag::TableCell,
            pulldown_cmark::Tag::Emphasis => MarkdownTag::Emphasis,
            pulldown_cmark::Tag::Strong => MarkdownTag::Strong,
            pulldown_cmark::Tag::Strikethrough => MarkdownTag::Strikethrough,
            pulldown_cmark::Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            } => MarkdownTag::Link {
                link_type,
                dest_url: SharedString::from(dest_url.into_string()),
                title: SharedString::from(title.into_string()),
                id: SharedString::from(id.into_string()),
            },
            pulldown_cmark::Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            } => MarkdownTag::Image {
                link_type,
                dest_url: SharedString::from(dest_url.into_string()),
                title: SharedString::from(title.into_string()),
                id: SharedString::from(id.into_string()),
            },
            pulldown_cmark::Tag::HtmlBlock => MarkdownTag::HtmlBlock,
            pulldown_cmark::Tag::MetadataBlock(kind) => MarkdownTag::MetadataBlock(kind),
            pulldown_cmark::Tag::DefinitionList => MarkdownTag::DefinitionList,
            pulldown_cmark::Tag::DefinitionListTitle => MarkdownTag::DefinitionListTitle,
            pulldown_cmark::Tag::DefinitionListDefinition => MarkdownTag::DefinitionListDefinition,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNWANTED_OPTIONS: Options = Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        .union(Options::ENABLE_MATH)
        .union(Options::ENABLE_DEFINITION_LIST);

    #[test]
    fn all_options_considered() {
        // The purpose of this is to fail when new options are added to pulldown_cmark, so that they
        // can be evaluated for inclusion.
        assert_eq!(PARSE_OPTIONS.union(UNWANTED_OPTIONS), Options::all());
    }

    #[test]
    fn wanted_and_unwanted_options_disjoint() {
        assert_eq!(
            PARSE_OPTIONS.intersection(UNWANTED_OPTIONS),
            Options::empty()
        );
    }
}
