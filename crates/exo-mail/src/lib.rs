#![deny(
    clippy::correctness,
    clippy::suspicious,
    clippy::complexity,
    clippy::perf
)]
//! Message bodies, rendered and shown.
//!
//! Two halves, kept apart so the first can run in the background and be cached:
//!
//! - [`render`] runs the render pipeline on a message's HTML (or plain text): sanitize,
//!   linkify, mark detected dates, strip tracking pixels, find links and the unsubscribe link,
//!   split off the quoted part (the `…`), classify marketing mail and tag colors for restyling.
//!   The result is a [`Render`], plain data the store can cache.
//! - [`BodyView`] lays a [`Render`] out with Blitz at the message column's width, paints it into
//!   an RGBA buffer, and answers hit tests (links, the `…`) and text selection.
//!
//! ```no_run
//! # fn demo(fonts: &exo_mail::Fonts) {
//! use exo_mail::{BodyView, Message, Region, ViewOptions, render};
//!
//! let message = Message {
//!     html: "<p>Hello <b>there</b></p>".into(),
//!     from_email: "ann@example.com".into(),
//!     ..Message::default()
//! };
//! let rendered = render(&message, None, &[]);
//! let mut view = BodyView::new(&rendered, ViewOptions::default(), fonts, None);
//! let height = view.layout(640.0, 2.0);
//! let width = view.content_width();
//! let pixels = view.paint(Region { top: 0.0, height, left: 0.0, width, phase: [0.0; 2] }, [255; 4]);
//! # }
//! ```
//!
//! [`printable_body`] is a body as it prints, for a browser engine to paginate.
//!
//! The compose editor shares three of the pipeline's parts: [`sanitize_fragment`] (DOMPurify as the
//! editor configures it), the linkifier ([`links`], [`match_link`]) and [`css_color`]. The
//! calendar's event details sanitize descriptions with [`sanitize_description`]. The editor draws
//! the tables it keeps whole with a [`BodyView`] of [`render_fragment`].

mod actions;
mod animation;
mod classify;
mod colors;
mod content;
mod dates;
mod dom;
mod fonts;
mod hints;
mod linkify;
mod links;
mod net;
mod paste;
mod pipeline;
mod pixels;
mod plain;
mod print;
mod purify;
mod quotes;
mod remote;
mod render;
mod sanitize;
mod senders;
mod view;

pub use actions::{Action, ActionSource, CalendarData, EventStatus, RsvpLinks};
pub use animation::{Player, may_animate};
pub use colors::css_color;
pub use dates::{DateParts, date_parts, find_dates};
pub use dom::html_text;
pub use fonts::{EMAIL_FACES, Fonts};
pub use linkify::{links, match_link};
pub use net::{Done, Fetch, decode_data_url};
pub use paste::prepare_paste;
pub use pipeline::{
    Message, RenderOptions, render, render_fragment, render_with, text_to_html,
};
pub use pixels::is_read_receipt;
pub use print::{BLOCKED_SCHEME, REMOTE_SCHEME_PREFIX, printable_body};
pub use remote::load_read_receipts;
pub use render::{BodyAttributes, Link, Render};
pub use sanitize::{sanitize_description, sanitize_fragment};
pub use view::{
    Animated, BodyView, ContextHit, ContextImage, Granularity, Hit, Image, Region,
    SCROLLBAR_HEIGHT, Theme, ViewOptions,
};
