//! The render pipeline, from a message's HTML to what the thread view shows. Nothing is measured
//! here, because Blitz measures when the body is shown.

use std::rc::Rc;

use crate::{
    BodyAttributes, Render, classify, colors, content, dates,
    dom::{self, NodeRef, Style},
    linkify, links, pixels, plain,
    quotes::{MessageBody, Previous, Quoted, Sender},
    remote, sanitize, senders,
};

/// What the pipeline needs to know about a message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    /// The body as HTML: the `text/html` part, or a plain-text one through [`text_to_html`].
    pub html: String,
    /// The sender's address.
    pub from_email: String,
    /// The sender's display name, as the `From` header has it.
    pub from_name: String,
    /// The `Message-ID` header (help-desk mail gets special quote handling).
    pub rfc822_id: String,
    /// Attachment file names. Exchange replaces quoted attachments with `<name>`.
    pub attachment_names: Vec<String>,
}

/// What a render does besides what every body gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    /// Mark the dates in the text. A message shown as a file in the attachment viewer has none
    /// (it's only sanitized).
    pub detect_dates: bool,
    /// Put the quote and signature behind a `…`. The attachment viewer's message shows whole, for
    /// the same reason.
    pub split_quoted: bool,
    /// Classify the body (marketing or not) and tag its colors for the restyle (`exo-color-*`).
    /// A read-only snippet shows its body only sanitized. It's never marketing, and its text takes
    /// the pane's ink where no color of its own says otherwise.
    pub classify: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            detect_dates: true,
            split_quoted: true,
            classify: true,
        }
    }
}

/// Runs the pipeline on `message`. `replying_to` is the message it answers (the one its
/// `In-Reply-To` names, else the one before it in the thread) and that message's render. The
/// quote of that message is what ends up behind the `…`. `own_domains` are the account's domains,
/// and a `mailto:` to them is never an unsubscribe link.
pub fn render(
    message: &Message,
    replying_to: Option<(&Message, &Render)>,
    own_domains: &[String],
) -> Render {
    render_with(message, replying_to, own_domains, RenderOptions::default())
}

/// [`render`], with `options`.
pub fn render_with(
    message: &Message,
    replying_to: Option<(&Message, &Render)>,
    own_domains: &[String],
    options: RenderOptions,
) -> Render {
    let (document, found) = sanitize::clean(&message.html);
    let root = document_element(&document);
    linkify::linkify(&root);
    linkify::add_word_breaks(&root);
    content::trim(&root);
    if options.detect_dates {
        dates::decorate(&root);
    }
    classify::mark_external_banners(&root);
    if options.classify {
        colors::carbonize(&root, classify::is_marketing(&root, &message.from_email));
    }
    classify::mark_small_pre(&root);
    let html = dom::outer_html(&document);
    let body_attributes = body_attributes(&document);

    // The new part is a copy, so `html` keeps the whole message for the next reply.
    let unquoted = dom::parse(&html);
    let read_status_urls = pixels::remove_read_status_pixels(&unquoted);
    let has_remote_images = remote::has_remote_content(&unquoted);
    let from = sender(message);
    let body = MessageBody::new(&unquoted, &from, &message.rfc822_id);
    let unquoted_body = dom::body(&unquoted).unwrap_or_else(|| unquoted.clone());
    let (links, unsubscribe_link) = links::find_links(&unquoted_body, &body.content, own_domains);
    let previous = replying_to
        .map(|(previous, render)| (previous, dom::parse(&render.html), sender(previous)));
    let (quoted, mut content) = if options.split_quoted {
        body.split(
            previous
                .as_ref()
                .map(|(previous, document, from)| Previous {
                    document,
                    from,
                    attachment_names: &previous.attachment_names,
                }),
        )
    } else {
        (
            Quoted {
                nodes: Vec::new(),
                hidden: None,
            },
            body.content,
        )
    };
    dom::detach_all(&quoted.nodes);
    let quoted_html: String = quoted.nodes.iter().map(dom::outer_html).collect();
    let unquoted_dates = dates::unquoted(&unquoted_body);
    let proposes_time = dates::proposes_time(&unquoted_body);
    classify::mark_flush_top(&unquoted);

    let visible_text = match quoted.hidden {
        Some(hidden) => {
            let before = content.formatted_slice(0, hidden.start);
            let length = content.len();
            let after = content.formatted_slice(hidden.end, length);
            match (before.is_empty(), after.is_empty()) {
                (false, false) => format!("{before}\n{after}"),
                (false, true) => before,
                _ => after,
            }
        }
        None => content.to_formatted_string(),
    }
    .replace('\u{a0}', " ");
    let quoted_document = dom::parse(&quoted_html);
    Render {
        html,
        unquoted_html: fragment(&unquoted),
        has_quoted_content: !quoted.nodes.is_empty(),
        quoted_html,
        body_attributes,
        unquoted_is_marketing: options.classify
            && classify::is_marketing(&unquoted, &message.from_email),
        quoted_is_marketing: options.classify
            && classify::is_marketing(&quoted_document, &message.from_email),
        snippet: visible_text.chars().take(200).collect(),
        visible_text,
        read_status_urls,
        has_remote_images,
        links,
        unsubscribe_link,
        actions: found.actions,
        calendar_data: found.calendar,
        unquoted_dates,
        proposes_time,
        sender_css: senders::css(&message.from_email),
    }
}

/// A fragment of a draft's body (a table the compose editor keeps whole) as the editor shows it,
/// for a [`crate::BodyView`]. Its colours are tagged for the restyle as the editor's content is,
/// as personal mail. Nothing else of the pipeline runs, because the editor sanitized it as it came
/// in and doesn't linkify or date what it shows.
pub fn render_fragment(html: &str) -> Render {
    let document = dom::parse(html);
    colors::carbonize(&document_element(&document), false);
    Render {
        unquoted_html: fragment(&document),
        ..Render::default()
    }
}

/// A plain-text body as HTML: paragraphs, `>` quotes as blockquotes.
pub fn text_to_html(text: &str) -> String {
    plain::to_html(text)
}

fn sender(message: &Message) -> Sender {
    Sender {
        email: message.from_email.clone(),
        name: message.from_name.clone(),
    }
}

fn document_element(document: &NodeRef) -> NodeRef {
    dom::element_children(document)
        .into_iter()
        .find(|n| dom::is(n, "html"))
        .unwrap_or_else(|| Rc::clone(document))
}

/// The new part as the body container's content: the `<head>`'s stylesheets, then the body.
fn fragment(document: &NodeRef) -> String {
    let head = dom::head(document)
        .map(|head| dom::inner_html(&head))
        .unwrap_or_default();
    let body = dom::body(document)
        .map(|body| dom::inner_html(&body))
        .unwrap_or_default();
    head + &body
}

/// The attributes of the message's `<body>` that the shown body keeps.
fn body_attributes(document: &NodeRef) -> BodyAttributes {
    let Some(body) = dom::body(document).filter(|b| dom::is(b, "body")) else {
        return BodyAttributes::default();
    };
    let mut style = Style::of(&body);
    if style.get("background-color").is_empty()
        && let Some(bgcolor) = dom::attr(&body, "bgcolor").filter(|c| !c.is_empty())
    {
        style.set("background-color", &bgcolor, false);
    }
    // Any `background` shorthand sets the background color too.
    let has_background =
        !style.get("background-color").is_empty() || !style.get("background").is_empty();
    let style = (!style.is_empty()).then(|| style.css());
    BodyAttributes {
        class: dom::attr(&body, "class").filter(|c| !c.is_empty()),
        id: dom::attr(&body, "id").filter(|i| !i.is_empty()),
        style,
        has_background,
    }
}

#[cfg(test)]
mod options_tests {
    use super::*;

    #[test]
    fn a_senders_fixes_follow_its_from_address_not_its_markup() {
        let ticket = |from: &str| {
            let message = Message {
                html: "<p>Fixed.</p><div class=\"zd_encoded_id\" style=\"color:#fff\">[7L5P0J-92RQ]</div>".into(),
                from_email: from.into(),
                ..Message::default()
            };
            let rendered = render(&message, None, &[]);
            crate::view::shadow_root(&rendered, false, "ShadowHTML", "", "", 1.0)
        };
        assert!(ticket("support@superhuman.com").contains("display: none"));
        assert!(!ticket("support@acme.zendesk.com").contains(".zd_encoded_id"));
    }

    #[test]
    fn a_render_without_dates_marks_none() {
        let message = Message {
            html: "<p>See you Thursday, Nov 26 at 3pm.</p>".into(),
            ..Message::default()
        };
        assert!(
            render(&message, None, &[]).html.contains("exo-date"),
            "dates are marked by default"
        );
        let plain = render_with(
            &message,
            None,
            &[],
            RenderOptions {
                detect_dates: false,
                ..RenderOptions::default()
            },
        );
        assert!(!plain.html.contains("exo-date"), "{}", plain.html);
    }

    #[test]
    fn a_render_without_quotes_keeps_the_signature_in_view() {
        let html = "<p>Thanks all for making the time on Monday. Below are the notes from the kickoff.</p><p>If you can't make the first review, send me your updates.</p><p>Daniel</p>";
        let message = Message {
            html: html.into(),
            from_email: "daniel@northwind.example.com".into(),
            from_name: "Daniel Ortiz".into(),
            ..Message::default()
        };
        assert!(
            render(&message, None, &[]).has_quoted_content,
            "the signature goes behind the `…` by default"
        );
        let whole = render_with(
            &message,
            None,
            &[],
            RenderOptions {
                split_quoted: false,
                ..RenderOptions::default()
            },
        );
        assert!(
            !whole.has_quoted_content && whole.unquoted_html.contains("Daniel"),
            "{}",
            whole.unquoted_html
        );
    }

    #[test]
    fn a_render_without_classifying_tags_no_colors_and_is_never_marketing() {
        let html = "<table><tr><td><a href=\"https://shop.example.com\">Shop the sale</a></td></tr></table><p style=\"color:#111\">Unsubscribe</p>";
        let message = Message {
            html: html.into(),
            ..Message::default()
        };
        let classified = render(&message, None, &[]);
        assert!(classified.html.contains("exo-color"), "{}", classified.html);
        let plain = render_with(
            &message,
            None,
            &[],
            RenderOptions {
                classify: false,
                ..RenderOptions::default()
            },
        );
        assert!(
            !plain.html.contains("exo-color") && !plain.unquoted_is_marketing,
            "{}",
            plain.html
        );
    }
}
